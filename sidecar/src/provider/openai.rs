use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

pub struct OpenAIProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: serde_json::Value,
}

fn convert_messages(messages: &[ChatMessage], system: Option<&str>) -> Vec<ApiMessage> {
    let mut result = Vec::new();

    if let Some(sys) = system {
        result.push(ApiMessage {
            role: "system".to_string(),
            content: serde_json::Value::String(sys.to_string()),
        });
    }

    for m in messages {
        let content = if m.content.len() == 1 {
            if let Some(ContentPart::Text { text }) = m.content.first() {
                serde_json::Value::String(text.clone())
            } else {
                serde_json::to_value(&m.content).unwrap_or_default()
            }
        } else {
            let parts: Vec<serde_json::Value> = m.content.iter().map(|p| match p {
                ContentPart::Text { text } => serde_json::json!({"type": "text", "text": text}),
                ContentPart::Image { media_type, data } => serde_json::json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:{};base64,{}", media_type, data)}
                }),
            }).collect();
            serde_json::Value::Array(parts)
        };

        result.push(ApiMessage {
            role: m.role.clone(),
            content,
        });
    }

    result
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        let body = ApiRequest {
            model: req.model,
            messages: convert_messages(&req.messages, req.system.as_deref()),
            max_tokens: req.max_tokens,
            stream: true,
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key)).unwrap(),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
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

            let mut buffer = String::new();
            let mut total_tokens = 0u32;
            let mut prompt_tokens = 0u32;
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

                while let Some(pos) = buffer.find("\n") {
                    let line = buffer[..pos].to_string();
                    buffer = buffer[pos + 1..].to_string();

                    let line = line.trim();
                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];
                    if data == "[DONE]" {
                        yield ChatEvent::Done {
                            input_tokens: prompt_tokens,
                            output_tokens: total_tokens,
                        };
                        return;
                    }

                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                            for choice in choices {
                                if let Some(delta) = choice.get("delta") {
                                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                        if !content.is_empty() {
                                            yield ChatEvent::Delta { text: content.to_string() };
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(usage) = parsed.get("usage") {
                            prompt_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            total_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        }
                    }
                }
            }
        };

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "openai",
            models: vec![
                "gpt-4o".to_string(),
                "gpt-4o-mini".to_string(),
                "o3".to_string(),
            ],
            streaming: true,
            vision: true,
        }
    }
}
