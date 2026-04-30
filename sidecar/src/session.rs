use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc;
use tracing::info;

use crate::agent_loop::{self, AgentToClient, ClientToAgent};
use crate::provider::anthropic::AnthropicProvider;
use crate::provider::bedrock::BedrockProvider;
use crate::provider::gemini::GeminiProvider;
use crate::provider::ollama::OllamaProvider;
use crate::provider::openai::OpenAIProvider;
use crate::provider::{ChatMessage, ChatRequest, ContentPart, Provider};
use crate::rpc::{Request, Response};
use crate::tools;

pub async fn handle_connection(stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut context_buffer: Vec<String> = Vec::new();

    // Channel for routing messages to an active agent loop
    let mut agent_tx: Option<mpsc::Sender<ClientToAgent>> = None;

    info!("client connected");

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::error(None, -32700, format!("parse error: {}", e));
                let _ = send_line(&mut writer, &resp).await;
                continue;
            }
        };

        match req.method.as_str() {
            "session.hello" => {
                let providers = get_available_providers().await;
                let resp = Response::success(
                    req.id,
                    json!({
                        "session_id": uuid::Uuid::new_v4().to_string(),
                        "providers": providers,
                        "features": ["attachments", "streaming", "context", "tools"]
                    }),
                );
                let _ = send_line(&mut writer, &resp).await;
            }
            "context.push" => {
                if let Some(payload) = req.params.get("payload") {
                    if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
                        let kind = req.params.get("kind").and_then(|k| k.as_str()).unwrap_or("snapshot");
                        if kind == "full_screen" {
                            // Replace entire buffer with the latest full screen
                            context_buffer.clear();
                            context_buffer.push(text.to_string());
                        } else {
                            // Legacy diff-based: append
                            context_buffer.push(text.to_string());
                            if context_buffer.len() > 10 {
                                context_buffer.remove(0);
                            }
                        }
                    }
                }
                let resp = Response::success(req.id, json!({"status": "ok"}));
                let _ = send_line(&mut writer, &resp).await;
            }
            "settings.update" => {
                if let Some(vars) = req.params.as_object() {
                    for (key, value) in vars {
                        if let Some(v) = value.as_str() {
                            if !v.is_empty() {
                                std::env::set_var(key, v);
                            }
                        }
                    }
                }
                let resp = Response::success(req.id, json!({"status": "ok"}));
                let _ = send_line(&mut writer, &resp).await;
            }
            "chat.send" => {
                let resp = Response::success(
                    req.id.clone(),
                    json!({"stream_id": uuid::Uuid::new_v4().to_string(), "status": "streaming"}),
                );
                let _ = send_line(&mut writer, &resp).await;

                // Build the provider and request
                let provider = build_provider(&req.params).await;
                let Some(provider) = provider else {
                    let notif = json!({"method": "chat.error", "params": {"message": "Failed to build provider"}});
                    let _ = writer.write_all(format!("{}\n", notif).as_bytes()).await;
                    continue;
                };

                let chat_req = build_chat_request(&req.params);

                // Create channels for the agent loop
                let (to_agent_tx, mut to_agent_rx) = mpsc::channel::<ClientToAgent>(32);
                let (from_agent_tx, mut from_agent_rx) = mpsc::channel::<AgentToClient>(64);
                agent_tx = Some(to_agent_tx);

                let context = context_buffer.clone();
                context_buffer.clear();

                // Spawn the agent loop
                tokio::spawn(async move {
                    agent_loop::run_agent_loop(
                        provider,
                        chat_req,
                        &context,
                        &from_agent_tx,
                        &mut to_agent_rx,
                    ).await;
                });

                // Forward agent notifications to the client until done
                while let Some(msg) = from_agent_rx.recv().await {
                    match msg {
                        AgentToClient::Notification(method, params) => {
                            let notif = json!({"method": method, "params": params});
                            let _ = writer.write_all(format!("{}\n", notif).as_bytes()).await;
                            let _ = writer.flush().await;
                        }
                        AgentToClient::Done => break,
                    }
                }

                agent_tx = None;
            }
            // Route tool responses to the active agent loop
            "chat.tool_approval_response" => {
                if let Some(tx) = &agent_tx {
                    let tool_call_id = req.params.get("tool_call_id")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let approved = req.params.get("approved")
                        .and_then(|v| v.as_bool()).unwrap_or(false);
                    let _ = tx.send(ClientToAgent::ToolApprovalResponse { tool_call_id, approved }).await;
                }
            }
            "chat.read_terminal_response" => {
                if let Some(tx) = &agent_tx {
                    let request_id = req.params.get("request_id")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let text = req.params.get("text")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let _ = tx.send(ClientToAgent::ReadTerminalResponse { request_id, text }).await;
                }
            }
            "chat.run_command_result" => {
                if let Some(tx) = &agent_tx {
                    let tool_call_id = req.params.get("tool_call_id")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let output = req.params.get("output")
                        .and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let exit_code = req.params.get("exit_code")
                        .and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
                    let _ = tx.send(ClientToAgent::RunCommandResult { tool_call_id, output, exit_code }).await;
                }
            }
            _ => {
                let resp = Response::error(
                    req.id,
                    -32601,
                    format!("method not found: {}", req.method),
                );
                let _ = send_line(&mut writer, &resp).await;
            }
        }
    }

    info!("client disconnected");
}

async fn build_provider(params: &Value) -> Option<Box<dyn Provider>> {
    let provider_id = params.get("provider").and_then(|v| v.as_str()).unwrap_or("anthropic");

    match provider_id {
        "anthropic" => {
            let key = std::env::var("ANTHROPIC_API_KEY").ok()?;
            Some(Box::new(AnthropicProvider::new(key)))
        }
        "openai" => {
            let key = std::env::var("OPENAI_API_KEY").ok()?;
            Some(Box::new(OpenAIProvider::new(key)))
        }
        "gemini" => {
            let key = std::env::var("GEMINI_API_KEY").ok()?;
            Some(Box::new(GeminiProvider::new(key)))
        }
        "ollama" => {
            let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
            Some(Box::new(OllamaProvider::with_url(url)))
        }
        "bedrock" => {
            let region = std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .unwrap_or_else(|_| "us-east-1".to_string());
            Some(Box::new(BedrockProvider::new(region)))
        }
        _ => None,
    }
}

fn build_chat_request(params: &Value) -> ChatRequest {
    let provider_id = params.get("provider").and_then(|v| v.as_str()).unwrap_or("anthropic");

    let default_model = match provider_id {
        "anthropic" => "claude-sonnet-4-6".to_string(),
        "openai" => "gpt-4o".to_string(),
        "gemini" => "gemini-2.5-flash".to_string(),
        "ollama" => "llama3.3".to_string(),
        "bedrock" => std::env::var("BEDROCK_MODEL_ID")
            .unwrap_or_else(|_| "anthropic.claude-sonnet-4-6-20250514-v1:0".to_string()),
        _ => "unknown".to_string(),
    };

    let model = params.get("model").and_then(|v| v.as_str())
        .map(|s| s.to_string()).unwrap_or(default_model);

    let messages = parse_messages(params);

    let base_system = params.get("system").and_then(|v| v.as_str()).unwrap_or(
        "You are a senior developer assistant embedded in a terminal emulator. You can see the user's terminal output in real-time. You have tools to read files, read the terminal, suggest commands, and run commands (with user approval). Help the user with development tasks. When guiding through multi-step processes, suggest one command at a time and wait for the result before proceeding."
    );

    ChatRequest {
        model,
        messages,
        system: Some(base_system.to_string()),
        max_tokens: params.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096) as u32,
        tools: Some(tools::tool_definitions()),
    }
}

fn parse_messages(params: &Value) -> Vec<ChatMessage> {
    let Some(msgs) = params.get("messages").and_then(|v| v.as_array()) else {
        return vec![];
    };

    msgs.iter()
        .filter_map(|m| {
            let role = m.get("role")?.as_str()?.to_string();
            let content = if let Some(text) = m.get("content").and_then(|c| c.as_str()) {
                vec![ContentPart::Text {
                    text: text.to_string(),
                }]
            } else if let Some(parts) = m.get("content").and_then(|c| c.as_array()) {
                parts
                    .iter()
                    .filter_map(|p| serde_json::from_value(p.clone()).ok())
                    .collect()
            } else {
                return None;
            };
            Some(ChatMessage { role, content })
        })
        .collect()
}

async fn get_available_providers() -> Value {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let ollama_models = OllamaProvider::fetch_models(&ollama_url).await;
    json!([
        {
            "id": "anthropic",
            "models": ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
            "streaming": true,
            "vision": true,
            "configured": std::env::var("ANTHROPIC_API_KEY").is_ok()
        },
        {
            "id": "openai",
            "models": ["gpt-4o", "gpt-4o-mini", "o3"],
            "streaming": true,
            "vision": true,
            "configured": std::env::var("OPENAI_API_KEY").is_ok()
        },
        {
            "id": "gemini",
            "models": ["gemini-2.5-pro", "gemini-2.5-flash"],
            "streaming": true,
            "vision": true,
            "configured": std::env::var("GEMINI_API_KEY").is_ok()
        },
        {
            "id": "ollama",
            "models": ollama_models,
            "streaming": true,
            "vision": false,
            "configured": true
        },
        {
            "id": "bedrock",
            "models": ["anthropic.claude-sonnet-4-6-20250514-v1:0", "anthropic.claude-haiku-4-5-20251001-v1:0", "us.anthropic.claude-opus-4-7-20250506-v1:0", "meta.llama3-3-70b-instruct-v1:0", "amazon.nova-pro-v1:0"],
            "streaming": true,
            "vision": true,
            "configured": std::env::var("AWS_ACCESS_KEY_ID").is_ok() || std::env::var("AWS_PROFILE").is_ok()
        }
    ])
}

async fn send_line(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    msg: &impl serde::Serialize,
) -> Result<(), std::io::Error> {
    let mut data = serde_json::to_string(msg).unwrap();
    data.push('\n');
    writer.write_all(data.as_bytes()).await?;
    writer.flush().await
}
