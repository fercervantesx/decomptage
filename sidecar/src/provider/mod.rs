pub mod anthropic;
pub mod bedrock;
pub mod gemini;
pub mod ollama;
pub mod openai;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { media_type: String, data: String },
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub system: Option<String>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    #[serde(rename = "delta")]
    Delta { text: String },
    #[serde(rename = "suggested_command")]
    SuggestedCommand {
        command: String,
        explanation: String,
    },
    #[serde(rename = "done")]
    Done {
        input_tokens: u32,
        output_tokens: u32,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub id: &'static str,
    pub models: Vec<String>,
    pub streaming: bool,
    pub vision: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String>;

    fn capabilities(&self) -> Capabilities;
}
