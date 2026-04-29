use async_trait::async_trait;
use futures::stream::BoxStream;

use super::{Capabilities, ChatEvent, ChatMessage, ChatRequest, Provider};

/// AWS Bedrock provider using the Converse Stream API.
///
/// This is a stub implementation. Full Bedrock support requires:
/// - aws-sdk-bedrockruntime crate
/// - AWS credential chain (env → file → IMDS → SSO)
/// - SigV4 signing
///
/// For now, returns an error directing the user to configure AWS credentials.
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
        _req: ChatRequest,
    ) -> Result<BoxStream<'static, ChatEvent>, String> {
        // TODO: Implement with aws-sdk-bedrockruntime
        // The converse_stream API provides a unified interface across
        // Bedrock-hosted models (Claude, Llama, Nova, etc.)
        Err(format!(
            "Bedrock provider not yet fully implemented. Region: {}. \
             Will use aws-sdk-bedrockruntime converse_stream API.",
            self.region
        ))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            id: "bedrock",
            models: vec![
                "anthropic.claude-sonnet-4-6-20250514-v1:0".to_string(),
                "anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
                "meta.llama3-3-70b-instruct-v1:0".to_string(),
            ],
            streaming: true,
            vision: true,
        }
    }
}
