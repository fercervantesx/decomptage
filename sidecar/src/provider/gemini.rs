use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::Serialize;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

pub struct GeminiProvider {
    api_key: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct ApiRequest {
    contents: Vec<ApiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct SystemInstruction {
    parts: Vec<Part>,
}

#[derive(Serialize)]
struct GenerationConfig {
    max_output_tokens: u32,
}

#[derive(Serialize)]
struct ApiContent {
    role: String,
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Text { text: String },
    InlineData { inline_data: InlineData },
}

#[derive(Serialize)]
struct InlineData {
    mime_type: String,
    data: String,
}

fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiContent> {
    messages
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" { "model" } else { "user" };
            let parts = m.content.iter().map(|p| match p {
                ContentPart::Text { text } => Part::Text { text: text.clone() },
                ContentPart::Image { media_type, data } => Part::InlineData {
                    inline_data: InlineData {
                        mime_type: media_type.clone(),
                        data: data.clone(),
                    },
                },
            }).collect();

            ApiContent {
                role: role.to_string(),
                parts,
            }
        })
        .collect()
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        let system_instruction = req.system.map(|s| SystemInstruction {
            parts: vec![Part::Text { text: s }],
        });

        let body = ApiRequest {
            contents: convert_messages(&req.messages),
            system_instruction,
            generation_config: GenerationConfig {
                max_output_tokens: req.max_tokens,
            },
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            req.model, self.api_key
        );

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Gemini error {}: {}", status, text));
        }

        let byte_stream = response.bytes_stream();

        let event_stream = async_stream::stream! {
            use futures::StreamExt;

            let mut buffer = String::new();
            let mut total_input = 0u32;
            let mut total_output = 0u32;
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
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(candidates) = parsed.get("candidates").and_then(|c| c.as_array()) {
                            for candidate in candidates {
                                if let Some(parts) = candidate.get("content")
                                    .and_then(|c| c.get("parts"))
                                    .and_then(|p| p.as_array()) {
                                    for part in parts {
                                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                            if !text.is_empty() {
                                                yield ChatEvent::Delta { text: text.to_string() };
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(usage) = parsed.get("usageMetadata") {
                            total_input = usage.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            total_output = usage.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        }
                    }
                }
            }

            yield ChatEvent::Done { input_tokens: total_input, output_tokens: total_output };
        };

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "gemini",
            models: vec![
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
            ],
            streaming: true,
            vision: true,
        }
    }
}
