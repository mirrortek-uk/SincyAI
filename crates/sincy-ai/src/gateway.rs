//! LLM gateway abstraction (TDD ai/gateway): providers are pluggable; costs are
//! always recorded via BudgetChecker. MockLlmGateway is for tests/dev only and
//! is labeled as mock in every result.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationRequest {
    pub prompt: String,
    pub model: String,
    pub max_tokens: u32,
    /// request id for cost attribution
    pub content_item_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationResult {
    pub text: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// true when produced by MockLlmGateway
    pub mock: bool,
    pub raw: Value,
}

#[async_trait]
pub trait LlmGateway: Send + Sync {
    fn provider(&self) -> &str;
    async fn generate(&self, req: &GenerationRequest) -> Result<GenerationResult, GatewayError>;
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("budget circuit open: {0}")]
    BudgetOpen(String),
}

/// Deterministic mock gateway. NEVER used in production paths (see DoD).
pub struct MockLlmGateway;

#[async_trait]
impl LlmGateway for MockLlmGateway {
    fn provider(&self) -> &str {
        "mock"
    }

    async fn generate(&self, req: &GenerationRequest) -> Result<GenerationResult, GatewayError> {
        let text = format!(
            "[mock:{}] {}",
            req.model,
            req.prompt.chars().take(200).collect::<String>()
        );
        Ok(GenerationResult {
            tokens_in: req.prompt.len() as u64 / 4,
            tokens_out: text.len() as u64 / 4,
            text,
            model: req.model.clone(),
            mock: true,
            raw: json!({"mock": true}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_gateway_is_labeled() {
        let gw = MockLlmGateway;
        let r = gw
            .generate(&GenerationRequest {
                prompt: "hi".into(),
                model: "mock-1".into(),
                max_tokens: 100,
                content_item_id: None,
            })
            .await
            .expect("gen");
        assert!(r.mock);
        assert!(r.text.starts_with("[mock:mock-1]"));
    }
}
