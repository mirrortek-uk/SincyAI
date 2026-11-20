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
        let text = mock_draft(&req.prompt, req.max_tokens);
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

/// Deterministic mock *draft*: instead of echoing the prompt, produce a short
/// post-shaped text derived from the prompt's "about: <topic>" line, so local
/// review flows (package -> approve -> publish) see realistic content.
/// Always prefixed with [mock:...] so it can never be mistaken for a real LLM.
fn mock_draft(prompt: &str, max_tokens: u32) -> String {
    let topic = prompt
        .lines()
        .find_map(|l| {
            l.split("about:")
                .nth(1)
                .and_then(|t| t.split_whitespace().next())
                .map(|t| {
                    t.trim_matches(|c| c == '.' || c == ',' || c == ';')
                        .to_string()
                })
        })
        .unwrap_or_else(|| "the topic at hand".to_string());
    // output language from the prompt's "Language: <bcp47>" line (default zh-CN)
    let language = prompt
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .split("language:")
                .nth(1)
                .and_then(|t| t.split_whitespace().next())
                .map(|t| {
                    t.trim_matches(|c| c == '.' || c == ',' || c == ';')
                        .to_string()
                })
        })
        .unwrap_or_else(|| "zh-CN".to_string());
    let is_zh = language.starts_with("zh");
    // platform limit for X posts is 280 WEIGHTED chars INCLUDING the [mock:...] label
    // (CJK counts double, same as the critic in sincy-employee)
    let prefix = "[mock:mock-1] ";
    let limit = if max_tokens > 0 {
        (max_tokens as usize * 3).min(280)
    } else {
        280
    };
    let budget = limit.saturating_sub(weighted_len(prefix));
    let body = if is_zh {
        format!(
            "本周聊聊 {topic} 的三个收获：\n1) 它带来的复利比想象中快；\n2) 最不起眼的部分（评测、护栏）反而最关键；\n3) 先从一个窄场景做起，别一上来就想做大而全的 agent。你会补充哪一条？"
        )
    } else {
        format!(
            "3 things I learned about {topic} this week:\n1) it compounds faster than expected;\n2) the boring parts (evals, guardrails) matter most;\n3) start with one narrow use case, not a grand agent. What would you add?"
        )
    };
    let mut w = 0usize;
    let body: String = body
        .chars()
        .take_while(|c| {
            let cw = char_weight(*c);
            if w + cw > budget {
                false
            } else {
                w += cw;
                true
            }
        })
        .collect();
    format!("{prefix}{body}")
}

/// X-style weighted length: CJK ideographs/kana/hangul count 2, the rest 1.
fn char_weight(c: char) -> usize {
    let v = c as u32;
    if (0x1100..=0x115F).contains(&v)
        || (0x2E80..=0xA4CF).contains(&v)
        || (0xAC00..=0xD7A3).contains(&v)
        || (0xF900..=0xFAFF).contains(&v)
        || (0xFE30..=0xFE4F).contains(&v)
        || (0xFF00..=0xFF60).contains(&v)
        || (0xFFE0..=0xFFE6).contains(&v)
        || (0x20000..=0x3FFFD).contains(&v)
    {
        2
    } else {
        1
    }
}

fn weighted_len(s: &str) -> usize {
    s.chars().map(char_weight).sum()
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
        // no Language line in prompt -> default zh-CN -> Chinese draft
        assert!(r.text.contains("本周聊聊")); // post-shaped draft, not a prompt echo
        assert!(weighted_len(&r.text) <= 280); // X weighted limit includes the mock label
        // topic-aware draft, explicit en-US
        let r2 = gw
            .generate(&GenerationRequest {
                prompt: "You are Aria. Language: en-US.\nWrite ONE x post about: obra/superpowers. Evidence: []".into(),
                model: "mock-1".into(),
                max_tokens: 512,
                content_item_id: None,
            })
            .await
            .unwrap();
        assert!(r2.text.contains("obra/superpowers"));
        assert!(r2.text.contains("3 things I learned"));
        assert!(weighted_len(&r2.text) <= 280);
        // explicit zh-CN -> Chinese, topic-aware
        let r3 = gw
            .generate(&GenerationRequest {
                prompt: "You are Aria. Language: zh-CN.\nWrite ONE x post about: addyosmani/agent-skills. Evidence: []".into(),
                model: "mock-1".into(),
                max_tokens: 512,
                content_item_id: None,
            })
            .await
            .unwrap();
        assert!(r3.text.contains("addyosmani/agent-skills"));
        assert!(r3.text.contains("本周聊聊"));
        assert!(weighted_len(&r3.text) <= 280);
    }
}
