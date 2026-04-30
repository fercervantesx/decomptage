use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

use crate::codeblock::CodeblockDetector;
use crate::provider::{ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};
use crate::tools::ToolCall;

/// Messages the agent loop sends to the connection handler (for the client)
#[derive(Debug)]
pub enum AgentToClient {
    Notification(String, Value), // method, params
    Done,
}

/// Messages the connection handler sends to the agent loop (from the client)
#[derive(Debug)]
pub enum ClientToAgent {
    ToolApprovalResponse { tool_call_id: String, approved: bool },
    ReadTerminalResponse { request_id: String, text: String },
    RunCommandResult { tool_call_id: String, output: String, exit_code: i32 },
}

/// Runs the full agentic loop: call LLM → handle tool use → re-call → repeat
pub async fn run_agent_loop(
    provider: Box<dyn Provider>,
    mut request: ChatRequest,
    context_buffer: &[String],
    out_tx: &mpsc::Sender<AgentToClient>,
    in_rx: &mut mpsc::Receiver<ClientToAgent>,
) {
    // Inject terminal context as the first message if available
    if !context_buffer.is_empty() {
        let context = context_buffer.join("\n---\n");
        let context_msg = ChatMessage {
            role: "user".to_string(),
            content: vec![ContentPart::Text {
                text: format!(
                    "[Terminal context - this is what's currently visible in my terminal.]\n\n```\n{}\n```",
                    context
                ),
            }],
        };
        request.messages.insert(0, context_msg);
    }

    // Debug logging
    if std::env::var("DECOMPTAGE_DEBUG").unwrap_or_default() == "1" {
        info!("--- agent_loop debug ---");
        info!("model: {}", request.model);
        info!("system: {}", request.system.as_deref().unwrap_or("(none)"));
        info!("tools: {}", request.tools.as_ref().map(|t| t.len()).unwrap_or(0));
        info!("context entries: {}", context_buffer.len());
        for (i, msg) in request.messages.iter().enumerate() {
            let preview: String = msg.content.iter().map(|p| match p {
                ContentPart::Text { text } => {
                    if text.len() > 300 { format!("{}...[truncated]", &text[..300]) } else { text.clone() }
                }
                ContentPart::Image { .. } => "[image]".to_string(),
            }).collect::<Vec<_>>().join(" ");
            info!("  msg[{}] role={} | {}", i, msg.role, preview);
        }
        info!("--- end debug ---");
    }

    let max_tool_iterations = 10;
    let mut iteration = 0;

    loop {
        iteration += 1;
        if iteration > max_tool_iterations {
            let _ = out_tx.send(AgentToClient::Notification(
                "chat.error".to_string(),
                json!({"message": "Too many tool iterations (max 10). Stopping."}),
            )).await;
            break;
        }

        // Call the LLM
        let stream_result = provider.chat_stream(request.clone()).await;
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                let _ = out_tx.send(AgentToClient::Notification(
                    "chat.error".to_string(),
                    json!({"message": e}),
                )).await;
                break;
            }
        };

        let mut codeblock = CodeblockDetector::new();
        let mut pending_tool_calls: Vec<(String, String, Value)> = vec![]; // (id, name, args)
        let mut assistant_text = String::new();

        // Stream the response
        while let Some(event) = stream.next().await {
            match event {
                ChatEvent::Delta { text } => {
                    assistant_text.push_str(&text);

                    // Codeblock detection for suggest_command fallback
                    if let Some(command) = codeblock.feed(&text) {
                        let _ = out_tx.send(AgentToClient::Notification(
                            "chat.suggested_command".to_string(),
                            json!({
                                "command": command,
                                "explanation": "Detected from code block",
                                "source": "codeblock_detected"
                            }),
                        )).await;
                    }

                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.delta".to_string(),
                        json!({"text": text}),
                    )).await;
                }
                ChatEvent::ToolUse { id, name, args } => {
                    pending_tool_calls.push((id, name, args));
                }
                ChatEvent::Done { input_tokens, output_tokens } => {
                    if pending_tool_calls.is_empty() {
                        // No tool calls — we're done
                        let _ = out_tx.send(AgentToClient::Notification(
                            "chat.done".to_string(),
                            json!({"usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}}),
                        )).await;
                    }
                }
                ChatEvent::Error { message } => {
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.error".to_string(),
                        json!({"message": message}),
                    )).await;
                    let _ = out_tx.send(AgentToClient::Done).await;
                    return;
                }
                ChatEvent::SuggestedCommand { command, explanation } => {
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.suggested_command".to_string(),
                        json!({"command": command, "explanation": explanation, "source": "tool_call"}),
                    )).await;
                }
            }
        }

        // If no tool calls, the conversation turn is complete
        if pending_tool_calls.is_empty() {
            break;
        }

        // Process tool calls — execute each and collect results
        // First, add the assistant message (with tool use) to history
        request.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: vec![ContentPart::Text { text: assistant_text.clone() }],
        });

        let mut tool_results: Vec<(String, String)> = vec![]; // (tool_use_id, result_text)

        for (id, name, args) in &pending_tool_calls {
            let tool_call = match ToolCall::from_raw(id, &name, args) {
                Some(tc) => tc,
                None => {
                    tool_results.push((id.clone(), format!("Unknown tool: {}", name)));
                    continue;
                }
            };

            let result = match &tool_call {
                ToolCall::SuggestCommand { command, explanation, danger_level, .. } => {
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.suggested_command".to_string(),
                        json!({
                            "command": command,
                            "explanation": explanation,
                            "danger_level": danger_level,
                            "source": "tool_call"
                        }),
                    )).await;
                    format!("Command suggested to user: {}", command)
                }
                ToolCall::RunCommand { command, explanation, .. } => {
                    let danger = tool_call.danger_level();
                    // Request approval from the user
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.tool_approval_request".to_string(),
                        json!({
                            "tool_call_id": id,
                            "tool_name": "run_command",
                            "command": command,
                            "explanation": explanation,
                            "danger_level": danger,
                        }),
                    )).await;

                    // Wait for approval response
                    match wait_for_approval(in_rx, id).await {
                        Some(ClientToAgent::ToolApprovalResponse { approved: true, .. }) => {
                            // Wait for command execution result
                            match wait_for_command_result(in_rx, id).await {
                                Some(ClientToAgent::RunCommandResult { output, exit_code, .. }) => {
                                    format!("Command executed. Exit code: {}. Output:\n{}", exit_code, output)
                                }
                                _ => "Command execution timed out or failed.".to_string(),
                            }
                        }
                        Some(ClientToAgent::ToolApprovalResponse { approved: false, .. }) => {
                            "User denied this command.".to_string()
                        }
                        _ => "Approval timed out.".to_string(),
                    }
                }
                ToolCall::ReadTerminal { .. } => {
                    // Request terminal content from client
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.read_terminal_request".to_string(),
                        json!({"request_id": id}),
                    )).await;

                    // Wait for response
                    match wait_for_terminal_read(in_rx, id).await {
                        Some(ClientToAgent::ReadTerminalResponse { text, .. }) => text,
                        _ => "Failed to read terminal.".to_string(),
                    }
                }
                ToolCall::ReadFile { path, lines, .. } => {
                    read_file_tool(path, *lines).await
                }
            };

            tool_results.push((id.clone(), result));
        }

        // Add tool results as a user message and loop back to call the LLM again
        let results_text = tool_results
            .iter()
            .map(|(id, result)| format!("[Tool result for {}]:\n{}", id, result))
            .collect::<Vec<_>>()
            .join("\n\n");

        request.messages.push(ChatMessage {
            role: "user".to_string(),
            content: vec![ContentPart::Text { text: results_text }],
        });

        info!("agent loop iteration {} — {} tool calls processed, re-calling LLM", iteration, tool_results.len());
    }

    let _ = out_tx.send(AgentToClient::Done).await;
}

async fn wait_for_approval(
    rx: &mut mpsc::Receiver<ClientToAgent>,
    tool_call_id: &str,
) -> Option<ClientToAgent> {
    // Wait up to 60 seconds for approval
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while let Some(msg) = rx.recv().await {
            if let ClientToAgent::ToolApprovalResponse { tool_call_id: id, .. } = &msg {
                if id == tool_call_id {
                    return Some(msg);
                }
            }
        }
        None
    })
    .await
    .unwrap_or(None)
}

async fn wait_for_command_result(
    rx: &mut mpsc::Receiver<ClientToAgent>,
    tool_call_id: &str,
) -> Option<ClientToAgent> {
    // Wait up to 30 seconds for command output
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(msg) = rx.recv().await {
            if let ClientToAgent::RunCommandResult { tool_call_id: id, .. } = &msg {
                if id == tool_call_id {
                    return Some(msg);
                }
            }
        }
        None
    })
    .await
    .unwrap_or(None)
}

async fn wait_for_terminal_read(
    rx: &mut mpsc::Receiver<ClientToAgent>,
    request_id: &str,
) -> Option<ClientToAgent> {
    // Wait up to 5 seconds for terminal read
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(msg) = rx.recv().await {
            if let ClientToAgent::ReadTerminalResponse { request_id: id, .. } = &msg {
                if id == request_id {
                    return Some(msg);
                }
            }
        }
        None
    })
    .await
    .unwrap_or(None)
}

async fn read_file_tool(path: &str, max_lines: Option<u32>) -> String {
    let max = max_lines.unwrap_or(500) as usize;
    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().take(max).collect();
            lines.join("\n")
        }
        Err(e) => format!("Error reading file: {}", e),
    }
}
