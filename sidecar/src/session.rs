use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tracing::{info, warn};

use crate::codeblock::CodeblockDetector;
use crate::provider::anthropic::AnthropicProvider;
use crate::provider::gemini::GeminiProvider;
use crate::provider::ollama::OllamaProvider;
use crate::provider::openai::OpenAIProvider;
use crate::provider::{ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};
use crate::rpc::{Notification, Request, Response};

pub async fn handle_connection(stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut context_buffer: Vec<String> = Vec::new();

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
                let resp = Response::success(
                    req.id,
                    json!({
                        "session_id": uuid::Uuid::new_v4().to_string(),
                        "providers": get_available_providers(),
                        "features": ["attachments", "streaming", "context"]
                    }),
                );
                let _ = send_line(&mut writer, &resp).await;
            }
            "context.push" => {
                if let Some(payload) = req.params.get("payload") {
                    if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
                        // Keep a rolling buffer of recent context (max 10 entries, ~256KB)
                        context_buffer.push(text.to_string());
                        if context_buffer.len() > 10 {
                            context_buffer.remove(0);
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

                handle_chat_send(&req.params, &context_buffer, &mut writer).await;
                // Clear context after use so it doesn't repeat
                context_buffer.clear();
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

async fn handle_chat_send(
    params: &Value,
    context_buffer: &[String],
    writer: &mut tokio::net::unix::OwnedWriteHalf,
) {
    let provider_id = params
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("anthropic");

    let provider: Box<dyn Provider> = match provider_id {
        "anthropic" => {
            let key = match std::env::var("ANTHROPIC_API_KEY") {
                Ok(k) => k,
                Err(_) => {
                    let notif = Notification {
                        method: "chat.error",
                        params: json!({"message": "ANTHROPIC_API_KEY not set"}),
                    };
                    let _ = send_line(writer, &notif).await;
                    return;
                }
            };
            Box::new(AnthropicProvider::new(key))
        }
        "openai" => {
            let key = match std::env::var("OPENAI_API_KEY") {
                Ok(k) => k,
                Err(_) => {
                    let notif = Notification {
                        method: "chat.error",
                        params: json!({"message": "OPENAI_API_KEY not set"}),
                    };
                    let _ = send_line(writer, &notif).await;
                    return;
                }
            };
            Box::new(OpenAIProvider::new(key))
        }
        "gemini" => {
            let key = match std::env::var("GEMINI_API_KEY") {
                Ok(k) => k,
                Err(_) => {
                    let notif = Notification {
                        method: "chat.error",
                        params: json!({"message": "GEMINI_API_KEY not set"}),
                    };
                    let _ = send_line(writer, &notif).await;
                    return;
                }
            };
            Box::new(GeminiProvider::new(key))
        }
        "ollama" => {
            let url = std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string());
            Box::new(OllamaProvider::with_url(url))
        }
        _ => {
            let notif = Notification {
                method: "chat.error",
                params: json!({"message": format!("unknown provider: {}", provider_id)}),
            };
            let _ = send_line(writer, &notif).await;
            return;
        }
    };

    let model = params
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(match provider_id {
            "anthropic" => "claude-sonnet-4-6",
            "openai" => "gpt-4o",
            "gemini" => "gemini-2.5-flash",
            "ollama" => "llama3.3",
            _ => "unknown",
        })
        .to_string();

    let messages = parse_messages(params);
    let base_system = params.get("system").and_then(|v| v.as_str()).unwrap_or(
        "You are a terminal copilot. You see what the user is doing in their terminal and help them. Be concise and practical. When suggesting commands, use the suggest_command tool or put them in ```sh code blocks."
    );

    let system = if context_buffer.is_empty() {
        Some(base_system.to_string())
    } else {
        let context = context_buffer.join("\n---\n");
        Some(format!(
            "{}\n\n<terminal_context>\nRecent terminal output:\n{}\n</terminal_context>",
            base_system, context
        ))
    };

    let req = ChatRequest {
        model,
        messages,
        system,
        max_tokens: params
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(4096) as u32,
    };

    match provider.chat_stream(req).await {
        Ok(mut stream) => {
            let mut codeblock = CodeblockDetector::new();

            while let Some(event) = stream.next().await {
                let notif = match &event {
                    ChatEvent::Delta { text } => {
                        // Feed into codeblock detector
                        if let Some(command) = codeblock.feed(text) {
                            let cmd_notif = Notification {
                                method: "chat.suggested_command",
                                params: json!({
                                    "command": command,
                                    "explanation": "Detected from code block",
                                    "source": "codeblock_detected"
                                }),
                            };
                            let _ = send_line(writer, &cmd_notif).await;
                        }

                        Notification {
                            method: "chat.delta",
                            params: json!({"text": text}),
                        }
                    }
                    ChatEvent::Done {
                        input_tokens,
                        output_tokens,
                    } => Notification {
                        method: "chat.done",
                        params: json!({"usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}}),
                    },
                    ChatEvent::Error { message } => Notification {
                        method: "chat.error",
                        params: json!({"message": message}),
                    },
                    ChatEvent::SuggestedCommand {
                        command,
                        explanation,
                    } => Notification {
                        method: "chat.suggested_command",
                        params: json!({"command": command, "explanation": explanation, "source": "tool_call"}),
                    },
                };
                if send_line(writer, &notif).await.is_err() {
                    break;
                }
            }
        }
        Err(e) => {
            let notif = Notification {
                method: "chat.error",
                params: json!({"message": e}),
            };
            let _ = send_line(writer, &notif).await;
        }
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

fn get_available_providers() -> Value {
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
            "models": ["llama3.3", "qwen2.5-coder", "mistral"],
            "streaming": true,
            "vision": false,
            "configured": true
        },
        {
            "id": "bedrock",
            "models": ["anthropic.claude-sonnet-4-6-20250514-v1:0", "meta.llama3-3-70b-instruct-v1:0"],
            "streaming": true,
            "vision": true,
            "configured": false,
            "note": "stub — requires aws-sdk-bedrockruntime"
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
