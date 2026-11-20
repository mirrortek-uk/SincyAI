//! Structured, evidence-linked memory (DE §13). Not raw conversation history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Episodic,
    Semantic,
    Strategic,
    Audience,
    Creative,
    Performance,
    Experiment,
    Operational,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::Episodic => "episodic",
            MemoryType::Semantic => "semantic",
            MemoryType::Strategic => "strategic",
            MemoryType::Audience => "audience",
            MemoryType::Creative => "creative",
            MemoryType::Performance => "performance",
            MemoryType::Experiment => "experiment",
            MemoryType::Operational => "operational",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub r#type: MemoryType,
    pub content: String,
    pub evidence: Vec<Value>,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
    pub last_validated: Option<DateTime<Utc>>,
}
