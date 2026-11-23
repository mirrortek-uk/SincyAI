//! Intelligence Signal (设计 §9 Layer 4): Pipeline 的最终统一输出。
//! Opportunity Engine 只消费 Signal, 不关心数据来自 RSS 还是 xbangdan。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    TrendRising,
    TopicEmerging,
    CreatorGrowing,
    ContentGoingViral,
    ProductLaunched,
    CompanyAnnouncement,
    CompetitorActivity,
    ResearchBreakthrough,
    MarketShift,
    ConsumerSentimentShift,
    IndustryEvent,
    TopicSignal,
    OpportunityCandidate,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalType::TrendRising => "trend_rising",
            SignalType::TopicEmerging => "topic_emerging",
            SignalType::CreatorGrowing => "creator_growing",
            SignalType::ContentGoingViral => "content_going_viral",
            SignalType::ProductLaunched => "product_launched",
            SignalType::CompanyAnnouncement => "company_announcement",
            SignalType::CompetitorActivity => "competitor_activity",
            SignalType::ResearchBreakthrough => "research_breakthrough",
            SignalType::MarketShift => "market_shift",
            SignalType::ConsumerSentimentShift => "consumer_sentiment_shift",
            SignalType::IndustryEvent => "industry_event",
            SignalType::TopicSignal => "topic_signal",
            SignalType::OpportunityCandidate => "opportunity_candidate",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "trend_rising" => SignalType::TrendRising,
            "topic_emerging" => SignalType::TopicEmerging,
            "creator_growing" => SignalType::CreatorGrowing,
            "content_going_viral" => SignalType::ContentGoingViral,
            "product_launched" => SignalType::ProductLaunched,
            "company_announcement" => SignalType::CompanyAnnouncement,
            "competitor_activity" => SignalType::CompetitorActivity,
            "research_breakthrough" => SignalType::ResearchBreakthrough,
            "market_shift" => SignalType::MarketShift,
            "consumer_sentiment_shift" => SignalType::ConsumerSentimentShift,
            "industry_event" => SignalType::IndustryEvent,
            "topic_signal" => SignalType::TopicSignal,
            "opportunity_candidate" => SignalType::OpportunityCandidate,
            _ => SignalType::TopicSignal,
        }
    }
}

/// 统一 Intelligence Entity (设计 §8 Layer 2/3): 类型化数据 + 通用元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelEntity {
    pub entity_id: String,
    pub entity_type: String, // article / trend / ranking / social_post / social_account / ...
    pub source_id: String,
    pub dataset_id: String,
    pub external_id: Option<String>,
    pub url: Option<String>,
    pub title: String,
    pub payload: Value,
    // Universal Metadata
    #[serde(default)]
    pub industry_tags: Vec<String>,
    #[serde(default)]
    pub topic_tags: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub geography: Vec<String>,
    #[serde(default)]
    pub language: String,
    pub provenance: Value,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub quality_score: f64,
    #[serde(default)]
    pub relevance_score: f64,
    pub observed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 统一 Signal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub signal_id: String,
    pub signal_type: SignalType,
    pub entity_id: Option<String>,
    pub source_id: String,
    pub industry_id: Option<String>,
    pub topic_id: Option<String>,
    /// 0..1
    pub strength: f64,
    pub confidence: f64,
    pub importance: f64,
    pub observed_at: DateTime<Utc>,
    pub payload: Value,
}
