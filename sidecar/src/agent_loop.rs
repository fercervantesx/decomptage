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
    // Inject terminal context as the first message if available.
    // Prioritize the BOTTOM of the screen (most recent output) by taking
    // the last N lines if the content is too long.
    if !context_buffer.is_empty() {
        let full_context = context_buffer.join("\n");
        let lines: Vec<&str> = full_context.lines().collect();
        let max_lines = 80;
        let context = if lines.len() > max_lines {
            // Keep the bottom (most recent) lines
            lines[lines.len() - max_lines..].join("\n")
        } else {
            full_context
        };

        let context_msg = ChatMessage {
            role: "user".to_string(),
            content: vec![ContentPart::Text {
                text: format!(
                    "[Terminal context - most recent terminal output (bottom of screen shown first):]\n\n```\n{}\n```",
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
                ToolCall::ReadFile { path, start_line, end_line, .. } => {
                    read_file_tool(path, *start_line, *end_line).await
                }
                ToolCall::WriteFile { path, content, .. } => {
                    // Write requires approval — send request
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.tool_approval_request".to_string(),
                        json!({
                            "tool_call_id": id,
                            "tool_name": "write_file",
                            "command": format!("Write {} bytes to {}", content.len(), path),
                            "explanation": format!("Create/overwrite file: {}", path),
                            "danger_level": "caution",
                        }),
                    )).await;
                    match wait_for_approval(in_rx, id).await {
                        Some(ClientToAgent::ToolApprovalResponse { approved: true, .. }) => {
                            write_file_tool(path, content).await
                        }
                        _ => "User denied file write.".to_string(),
                    }
                }
                ToolCall::EditFile { path, old_string, new_string, .. } => {
                    let _ = out_tx.send(AgentToClient::Notification(
                        "chat.tool_approval_request".to_string(),
                        json!({
                            "tool_call_id": id,
                            "tool_name": "edit_file",
                            "command": format!("Edit {}", path),
                            "explanation": format!("Replace text in {}", path),
                            "danger_level": "caution",
                        }),
                    )).await;
                    match wait_for_approval(in_rx, id).await {
                        Some(ClientToAgent::ToolApprovalResponse { approved: true, .. }) => {
                            edit_file_tool(path, old_string, new_string).await
                        }
                        _ => "User denied file edit.".to_string(),
                    }
                }
                ToolCall::ListDirectory { path, recursive, .. } => {
                    list_directory_tool(path.as_deref(), *recursive).await
                }
                ToolCall::SearchFiles { pattern, path, file_pattern, .. } => {
                    search_files_tool(pattern, path.as_deref(), file_pattern.as_deref()).await
                }
                ToolCall::WebFetch { url, max_length, .. } => {
                    web_fetch_tool(url, *max_length).await
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

async fn read_file_tool(path: &str, start_line: Option<u32>, end_line: Option<u32>) -> String {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let start = start_line.map(|s| (s as usize).saturating_sub(1)).unwrap_or(0);
            let end = end_line.map(|e| e as usize).unwrap_or(lines.len()).min(start + 1000);
            lines[start..end.min(lines.len())]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{:4} | {}", start + i + 1, l))
                .collect::<Vec<_>>()
                .join("\n")
        }
        Err(e) => format!("Error reading file: {}", e),
    }
}

async fn write_file_tool(path: &str, content: &str) -> String {
    // Create parent directories if needed
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    match tokio::fs::write(path, content).await {
        Ok(_) => format!("Successfully wrote {} bytes to {}", content.len(), path),
        Err(e) => format!("Error writing file: {}", e),
    }
}

async fn edit_file_tool(path: &str, old_string: &str, new_string: &str) -> String {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            let count = content.matches(old_string).count();
            if count == 0 {
                return format!("Error: old_string not found in {}", path);
            }
            if count > 1 {
                return format!("Error: old_string found {} times in {} (must be unique)", count, path);
            }
            let new_content = content.replacen(old_string, new_string, 1);
            match tokio::fs::write(path, &new_content).await {
                Ok(_) => format!("Successfully edited {}", path),
                Err(e) => format!("Error writing file: {}", e),
            }
        }
        Err(e) => format!("Error reading file: {}", e),
    }
}

async fn list_directory_tool(path: Option<&str>, recursive: bool) -> String {
    let dir = path.unwrap_or(".");
    if recursive {
        // Use find-like recursive listing (max 3 levels)
        match tokio::process::Command::new("find")
            .args([dir, "-maxdepth", "3", "-type", "f", "-o", "-type", "d"])
            .output()
            .await
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = text.lines().take(200).collect();
                lines.join("\n")
            }
            Err(e) => format!("Error listing directory: {}", e),
        }
    } else {
        match tokio::fs::read_dir(dir).await {
            Ok(mut entries) => {
                let mut items = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                    items.push(if is_dir { format!("{}/", name) } else { name });
                }
                items.sort();
                items.join("\n")
            }
            Err(e) => format!("Error listing directory: {}", e),
        }
    }
}

async fn search_files_tool(pattern: &str, path: Option<&str>, file_pattern: Option<&str>) -> String {
    let dir = path.unwrap_or(".");
    let mut args = vec!["-rn".to_string(), pattern.to_string(), dir.to_string()];
    if let Some(fp) = file_pattern {
        args = vec!["-rn".to_string(), "--include".to_string(), fp.to_string(), pattern.to_string(), dir.to_string()];
    }

    match tokio::process::Command::new("grep")
        .args(&args)
        .output()
        .await
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = text.lines().take(50).collect();
            if lines.is_empty() {
                "No matches found.".to_string()
            } else {
                lines.join("\n")
            }
        }
        Err(e) => format!("Error searching: {}", e),
    }
}

async fn web_fetch_tool(url: &str, max_length: Option<u32>) -> String {
    let max = max_length.unwrap_or(10000) as usize;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    match client.get(url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return format!("HTTP error: {}", resp.status());
            }
            match resp.text().await {
                Ok(body) => {
                    // Strip HTML tags for cleaner output
                    let text = strip_html_tags(&body);
                    if text.len() > max {
                        format!("{}...\n[truncated at {} chars]", &text[..max], max)
                    } else {
                        text
                    }
                }
                Err(e) => format!("Error reading response: {}", e),
            }
        }
        Err(e) => format!("Error fetching URL: {}", e),
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut in_script = false;

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            // Check if entering script/style
            let lower = html[html.find(c).unwrap_or(0)..].to_lowercase();
            if lower.starts_with("<script") || lower.starts_with("<style") {
                in_script = true;
            }
            if lower.starts_with("</script") || lower.starts_with("</style") {
                in_script = false;
            }
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag && !in_script {
            result.push(c);
        }
    }

    // Collapse multiple newlines/spaces
    let mut prev_newline = false;
    let mut cleaned = String::new();
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_newline {
                cleaned.push('\n');
                prev_newline = true;
            }
        } else {
            cleaned.push_str(trimmed);
            cleaned.push('\n');
            prev_newline = false;
        }
    }
    cleaned
}
