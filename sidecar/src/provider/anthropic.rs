use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

pub struct AnthropicProvider {
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: Vec<ApiContent>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ApiContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<DeltaPayload>,
    #[serde(default)]
    usage: Option<UsagePayload>,
    #[serde(default)]
    message: Option<MessagePayload>,
}

#[derive(Debug, Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct UsagePayload {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct MessagePayload {
    #[serde(default)]
    usage: Option<UsagePayload>,
}

fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiMessage> {
    messages
        .iter()
        .map(|m| ApiMessage {
            role: m.role.clone(),
            content: m
                .content
                .iter()
                .map(|p| match p {
                    ContentPart::Text { text } => ApiContent::Text { text: text.clone() },
                    ContentPart::Image { media_type, data } => ApiContent::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                    },
                })
                .collect(),
        })
        .collect()
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        let body = ApiRequest {
            model: req.model,
            max_tokens: req.max_tokens,
            messages: convert_messages(&req.messages),
            system: req.system,
            stream: true,
            tools: req.tools,
        };

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key).unwrap());
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, text));
        }

        let byte_stream = response.bytes_stream();

        let event_stream = async_stream::stream! {
            use futures::StreamExt;
            use bytes::BytesMut;

            let mut buffer = String::new();
            let mut current_event_type = String::new();
            let mut input_tokens = 0u32;
            let mut output_tokens = 0u32;

            // Tool use accumulation
            let mut current_tool_id = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_input_json = String::new();
            let mut in_tool_block = false;

            let mut byte_stream = std::pin::pin!(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield ChatEvent::Error { message: format!("stream error: {}", e) };
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(double_newline) = buffer.find("\n\n") {
                    let event_block = buffer[..double_newline].to_string();
                    buffer = buffer[double_newline + 2..].to_string();

                    let mut data_lines = Vec::new();
                    for line in event_block.lines() {
                        if let Some(et) = line.strip_prefix("event: ") {
                            current_event_type = et.trim().to_string();
                        } else if let Some(d) = line.strip_prefix("data: ") {
                            data_lines.push(d.to_string());
                        }
                    }

                    if data_lines.is_empty() {
                        continue;
                    }

                    let data = data_lines.join("\n");

                    match current_event_type.as_str() {
                        "content_block_start" => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(block) = parsed.get("content_block") {
                                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                        current_tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        current_tool_name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        current_tool_input_json.clear();
                                        in_tool_block = true;
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(delta) = parsed.get("delta") {
                                    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                    if delta_type == "input_json_delta" {
                                        if let Some(partial) = delta.get("partial_json").and_then(|t| t.as_str()) {
                                            current_tool_input_json.push_str(partial);
                                        }
                                    } else if delta_type == "text_delta" {
                                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                            if !text.is_empty() {
                                                yield ChatEvent::Delta { text: text.to_string() };
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        "content_block_stop" => {
                            if in_tool_block {
                                let args = serde_json::from_str::<serde_json::Value>(&current_tool_input_json)
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                                yield ChatEvent::ToolUse {
                                    id: current_tool_id.clone(),
                                    name: current_tool_name.clone(),
                                    args,
                                };
                                in_tool_block = false;
                            }
                        }
                        "message_start" => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(usage) = parsed.get("message").and_then(|m| m.get("usage")) {
                                    input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                }
                            }
                        }
                        "message_delta" => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(usage) = parsed.get("usage") {
                                    output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                                }
                            }
                        }
                        "message_stop" => {
                            yield ChatEvent::Done { input_tokens, output_tokens };
                        }
                        _ => {}
                    }
                }
            }
        };

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "anthropic",
            models: vec![
                "claude-opus-4-7".to_string(),
                "claude-sonnet-4-6".to_string(),
                "claude-haiku-4-5".to_string(),
            ],
            streaming: true,
            vision: true,
        }
    }
}
