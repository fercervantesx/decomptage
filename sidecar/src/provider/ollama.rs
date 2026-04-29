use async_trait::async_trait;
use futures::stream::BoxStream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Serialize;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

pub struct OllamaProvider {
    base_url: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_url(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
}

fn convert_messages(messages: &[ChatMessage], system: Option<&str>) -> Vec<ApiMessage> {
    let mut result = Vec::new();

    if let Some(sys) = system {
        result.push(ApiMessage {
            role: "system".to_string(),
            content: sys.to_string(),
            images: None,
        });
    }

    for m in messages {
        let mut text_parts = Vec::new();
        let mut images = Vec::new();

        for part in &m.content {
            match part {
                ContentPart::Text { text } => text_parts.push(text.clone()),
                ContentPart::Image { data, .. } => images.push(data.clone()),
            }
        }

        result.push(ApiMessage {
            role: m.role.clone(),
            content: text_parts.join("\n"),
            images: if images.is_empty() { None } else { Some(images) },
        });
    }

    result
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        let body = ApiRequest {
            model: req.model,
            messages: convert_messages(&req.messages, req.system.as_deref()),
            stream: true,
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Ollama error {}: {}", status, text));
        }

        let byte_stream = response.bytes_stream();

        // Ollama streams line-delimited JSON (not SSE)
        let event_stream = async_stream::stream! {
            use futures::StreamExt;

            let mut buffer = String::new();
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

                    if line.trim().is_empty() {
                        continue;
                    }

                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
                        let done = parsed.get("done").and_then(|d| d.as_bool()).unwrap_or(false);

                        if let Some(msg) = parsed.get("message") {
                            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                                if !content.is_empty() {
                                    yield ChatEvent::Delta { text: content.to_string() };
                                }
                            }
                        }

                        if done {
                            let prompt_tokens = parsed.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let output_tokens = parsed.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            yield ChatEvent::Done { input_tokens: prompt_tokens, output_tokens };
                            return;
                        }
                    }
                }
            }
        };

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "ollama",
            models: vec![
                "llama3.3".to_string(),
                "qwen2.5-coder".to_string(),
                "mistral".to_string(),
            ],
            streaming: true,
            vision: false,
        }
    }
}
