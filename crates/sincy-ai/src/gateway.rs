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

/// Real provider: any OpenAI-compatible /chat/completions endpoint
/// (OpenAI, Azure OpenAI, vLLM, LiteLLM...). API key never logged.
pub struct OpenAiCompatibleGateway {
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
    http: reqwest::Client,
}

impl OpenAiCompatibleGateway {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmGateway for OpenAiCompatibleGateway {
    fn provider(&self) -> &str {
        "openai-compatible"
    }

    async fn generate(&self, req: &GenerationRequest) -> Result<GenerationResult, GatewayError> {
        if self.api_key.is_empty() {
            return Err(GatewayError::Provider(
                "SINCY_LLM_API_KEY not set for openai-compatible provider".into(),
            ));
        }
        let model = if req.model.is_empty() {
            self.default_model.clone()
        } else {
            req.model.clone()
        };
        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": req.prompt}],
            "max_tokens": req.max_tokens,
        });
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::Provider(format!("network: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(GatewayError::Provider(format!(
                "provider returned {status}: {}",
                &text[..text.len().min(300)]
            )));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::Provider(format!("bad json: {e}")))?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let usage = &v["usage"];
        Ok(GenerationResult {
            text,
            model: v["model"].as_str().unwrap_or(&model).to_string(),
            tokens_in: usage["prompt_tokens"].as_u64().unwrap_or(0),
            tokens_out: usage["completion_tokens"].as_u64().unwrap_or(0),
            mock: false,
            raw: v,
        })
    }
}

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
