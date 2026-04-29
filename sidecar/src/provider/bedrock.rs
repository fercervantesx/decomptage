use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseStreamOutput, Message, SystemContentBlock,
};
use futures::stream::BoxStream;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Serialize;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

/// Bedrock supports two auth modes:
/// 1. IAM credentials (AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY) → uses AWS SDK + SigV4
/// 2. Bedrock API Key (BEDROCK_API_KEY) → uses bearer token on the REST API directly
pub struct BedrockProvider {
    region: String,
    api_key: Option<String>,
}

impl BedrockProvider {
    pub fn new(region: String) -> Self {
        let api_key = std::env::var("BEDROCK_API_KEY").ok();
        Self { region, api_key }
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        // If a Bedrock API Key is set, use the bearer token REST path
        if let Some(api_key) = &self.api_key {
            return self.chat_stream_with_api_key(api_key, req).await;
        }

        // Otherwise, use IAM credentials via the AWS SDK
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(aws_config::Region::new(self.region.clone()))
            .load()
            .await;

        let client = aws_sdk_bedrockruntime::Client::new(&config);

        let messages: Vec<Message> = req
            .messages
            .iter()
            .map(|m| {
                let role = if m.role == "assistant" {
                    ConversationRole::Assistant
                } else {
                    ConversationRole::User
                };

                let content: Vec<ContentBlock> = m
                    .content
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text { text } => ContentBlock::Text(text.clone()),
                        ContentPart::Image { media_type, data } => {
                            let format = match media_type.as_str() {
                                "image/png" => aws_sdk_bedrockruntime::types::ImageFormat::Png,
                                "image/jpeg" => aws_sdk_bedrockruntime::types::ImageFormat::Jpeg,
                                "image/gif" => aws_sdk_bedrockruntime::types::ImageFormat::Gif,
                                "image/webp" => aws_sdk_bedrockruntime::types::ImageFormat::Webp,
                                _ => aws_sdk_bedrockruntime::types::ImageFormat::Png,
                            };
                            let bytes = base64::Engine::decode(
                                &base64::engine::general_purpose::STANDARD,
                                data,
                            )
                            .unwrap_or_default();
                            let blob = aws_smithy_types::Blob::new(bytes);
                            let source =
                                aws_sdk_bedrockruntime::types::ImageSource::Bytes(blob);
                            let image_block = aws_sdk_bedrockruntime::types::ImageBlock::builder()
                                .format(format)
                                .source(source)
                                .build()
                                .unwrap();
                            ContentBlock::Image(image_block)
                        }
                    })
                    .collect();

                Message::builder()
                    .role(role)
                    .set_content(Some(content))
                    .build()
                    .unwrap()
            })
            .collect();

        let mut request = client
            .converse_stream()
            .model_id(&req.model)
            .set_messages(Some(messages));

        if let Some(sys) = &req.system {
            request = request.system(SystemContentBlock::Text(sys.clone()));
        }

        let output = request
            .send()
            .await
            .map_err(|e| format!("Bedrock error: {}", e))?;

        let mut stream = output.stream;

        let event_stream = async_stream::stream! {
            let mut input_tokens = 0u32;
            let mut output_tokens = 0u32;

            loop {
                match stream.recv().await {
                    Ok(Some(event)) => match event {
                        ConverseStreamOutput::ContentBlockDelta(delta) => {
                            if let Some(d) = delta.delta() {
                                if let aws_sdk_bedrockruntime::types::ContentBlockDelta::Text(text) = d {
                                    if !text.is_empty() {
                                        yield ChatEvent::Delta { text: text.to_string() };
                                    }
                                }
                            }
                        }
                        ConverseStreamOutput::Metadata(meta) => {
                            if let Some(usage) = meta.usage() {
                                input_tokens = usage.input_tokens() as u32;
                                output_tokens = usage.output_tokens() as u32;
                            }
                        }
                        ConverseStreamOutput::MessageStop(_) => {
                            yield ChatEvent::Done { input_tokens, output_tokens };
                            return;
                        }
                        _ => {}
                    },
                    Ok(None) => {
                        yield ChatEvent::Done { input_tokens, output_tokens };
                        return;
                    }
                    Err(e) => {
                        yield ChatEvent::Error { message: format!("stream error: {}", e) };
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(event_stream))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "bedrock",
            models: vec![
                "anthropic.claude-sonnet-4-6-20250514-v1:0".to_string(),
                "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
                "us.anthropic.claude-opus-4-7-20250506-v1:0".to_string(),
                "meta.llama3-3-70b-instruct-v1:0".to_string(),
                "amazon.nova-pro-v1:0".to_string(),
            ],
            streaming: true,
            vision: true,
        }
    }
}

// MARK: - Bedrock API Key path (bearer token, Anthropic Messages format)

#[derive(Serialize)]
struct ApiKeyRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ApiKeyMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
}

#[derive(Serialize)]
struct ApiKeyMessage {
    role: String,
    content: Vec<ApiKeyContent>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ApiKeyContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ApiKeyImageSource },
}

#[derive(Serialize)]
struct ApiKeyImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

impl BedrockProvider {
    async fn chat_stream_with_api_key(
        &self,
        api_key: &str,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        let messages: Vec<ApiKeyMessage> = req
            .messages
            .iter()
            .map(|m| ApiKeyMessage {
                role: m.role.clone(),
                content: m
                    .content
                    .iter()
                    .map(|p| match p {
                        ContentPart::Text { text } => ApiKeyContent::Text { text: text.clone() },
                        ContentPart::Image { media_type, data } => ApiKeyContent::Image {
                            source: ApiKeyImageSource {
                                source_type: "base64".to_string(),
                                media_type: media_type.clone(),
                                data: data.clone(),
                            },
                        },
                    })
                    .collect(),
            })
            .collect();

        let body = ApiKeyRequest {
            model: req.model.clone(),
            max_tokens: req.max_tokens,
            messages,
            system: req.system,
            stream: true,
        };

        // Bedrock API Key endpoint uses the model ID in the URL
        let url = format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke-with-response-stream",
            self.region, req.model
        );

        let client = reqwest::Client::new();
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", api_key)).unwrap(),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = client
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Bedrock API key request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("Bedrock API error {}: {}", status, text));
        }

        let byte_stream = response.bytes_stream();

        // Bedrock with API key uses SSE format similar to Anthropic
        let event_stream = async_stream::stream! {
            use futures::StreamExt;

            let mut buffer = String::new();
            let mut current_event_type = String::new();
            let mut input_tokens = 0u32;
            let mut output_tokens = 0u32;
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

                    if data_lines.is_empty() { continue; }
                    let data = data_lines.join("\n");

                    match current_event_type.as_str() {
                        "content_block_delta" => {
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(delta) = parsed.get("delta") {
                                    if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                        if !text.is_empty() {
                                            yield ChatEvent::Delta { text: text.to_string() };
                                        }
                                    }
                                }
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
}
