use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, ConverseStreamOutput, Message, SystemContentBlock,
};
use futures::stream::BoxStream;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, ContentPart, Provider};

pub struct BedrockProvider {
    region: String,
}

impl BedrockProvider {
    pub fn new(region: String) -> Self {
        Self { region }
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
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
