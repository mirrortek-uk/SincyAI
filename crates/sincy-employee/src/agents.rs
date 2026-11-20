//! Logical agents (DE §8, §21: six MVP agents). Agents are capabilities invoked
//! by the orchestrator with a fixed contract — not long-running chatbots.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Executive,
    ResearchStrategy,
    Creative,
    Content,
    CriticCompliance,
    AnalyticsLearning,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Executive => "executive",
            AgentKind::ResearchStrategy => "research_strategy",
            AgentKind::Creative => "creative",
            AgentKind::Content => "content",
            AgentKind::CriticCompliance => "critic_compliance",
            AgentKind::AnalyticsLearning => "analytics_learning",
        }
    }
}

/// Invocation contract (DE §8): everything an agent needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    pub task_id: String,
    pub employee_id: String,
    pub config_version: i32,
    pub goal_context: Value,
    pub strategy_context: Value,
    pub memory: Vec<Value>,
    pub evidence_refs: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub permission_scope: String,
    pub output_schema: String,
    pub budget_usd: f64,
}

/// What an agent returns (DE §8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub structured: Value,
    pub evidence_refs: Vec<String>,
    pub confidence: f64,
    pub assumptions: Vec<String>,
    pub cost_usd: f64,
    pub recommended_next: Option<String>,
    pub error: Option<String>,
}
