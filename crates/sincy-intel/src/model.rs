//! Unified intelligence data model (WI guide §11).
//! Tool-specific response formats never leak past the connector boundary —
//! everything is normalized into these schemas.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMethod {
    NativeConnector,
    RestProvider,
    McpServer,
    CliWorker,
    ExternalService,
    RssSource,
    Aggregator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelKind {
    SocialContent,
    Trend,
    Event,
    Topic,
    Creator,
    News,
    Knowledge,
}

impl IntelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntelKind::SocialContent => "social_content",
            IntelKind::Trend => "trend",
            IntelKind::Event => "event",
            IntelKind::Topic => "topic",
            IntelKind::Creator => "creator",
            IntelKind::News => "news",
            IntelKind::Knowledge => "knowledge",
        }
    }
}

/// A registered intelligence source (WI §6 source registry).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub category: String,
    pub access_method: AccessMethod,
    /// capability tags, e.g. "social_search", "trend_search", "news_research"
    pub capabilities: Vec<String>,
    pub platforms: Vec<String>,
    pub enabled: bool,
}

/// Normalized social content (WI §11 SocialContent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialContent {
    pub platform: String,
    pub external_id: String,
    pub author: String,
    pub text: String,
    pub url: Option<String>,
    pub metrics: serde_json::Value,
    pub published_at: Option<DateTime<Utc>>,
}

/// Normalized trend (WI §11 Trend + §16 trend metrics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trend {
    pub topic: String,
    pub platforms: Vec<String>,
    pub volume: i64,
    pub velocity: f64, // growth rate per hour
    pub sentiment: f64,
    pub window_hours: i64,
}

/// A normalized intelligence item as stored in intel_items (dedup by dedup_key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelItem {
    pub source_id: String,
    pub provider_id: Option<String>,
    pub kind: IntelKind,
    pub dedup_key: String,
    pub payload: serde_json::Value,
    pub collected_at: DateTime<Utc>,
}

/// Opportunity engine output (WI §19).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub topic: String,
    pub score: f64,
    pub rationale: serde_json::Value,
    pub evidence: Vec<String>, // dedup keys / source urls
    pub status: String,
}
