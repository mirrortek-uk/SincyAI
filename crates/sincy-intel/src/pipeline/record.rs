//! RawRecord 统一信封 (Sincy Pipeline 设计 §4): 所有 Connector 的输出都是它。
//! 信封统一、载荷保持原始; 入库 JSON 列, 但入口处强类型校验 record_type。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 原始记录类型 —— Connector 产出的数据形状 (不等于最终 Entity Type)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordType {
    Article,
    SocialPost,
    SocialAccount,
    Ranking,
    Trend,
    Metric,
    SearchResult,
    ResearchPaper,
    Video,
    Product,
    Event,
    Comment,
    Dataset,
    Unknown,
}

impl RecordType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecordType::Article => "article",
            RecordType::SocialPost => "social_post",
            RecordType::SocialAccount => "social_account",
            RecordType::Ranking => "ranking",
            RecordType::Trend => "trend",
            RecordType::Metric => "metric",
            RecordType::SearchResult => "search_result",
            RecordType::ResearchPaper => "research_paper",
            RecordType::Video => "video",
            RecordType::Product => "product",
            RecordType::Event => "event",
            RecordType::Comment => "comment",
            RecordType::Dataset => "dataset",
            RecordType::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "article" => RecordType::Article,
            "social_post" => RecordType::SocialPost,
            "social_account" => RecordType::SocialAccount,
            "ranking" => RecordType::Ranking,
            "trend" => RecordType::Trend,
            "metric" => RecordType::Metric,
            "search_result" => RecordType::SearchResult,
            "research_paper" => RecordType::ResearchPaper,
            "video" => RecordType::Video,
            "product" => RecordType::Product,
            "event" => RecordType::Event,
            "comment" => RecordType::Comment,
            "dataset" => RecordType::Dataset,
            _ => RecordType::Unknown,
        }
    }
    /// NATS subject: 每个 record_type 一个主题 (设计 §5)
    pub fn subject(&self) -> String {
        format!("intel.raw.{}", self.as_str())
    }
}

/// 采集游标: 连接器返回"下一次从哪开始", 重复轮询幂等
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub value: String,
}

/// 一批原始记录 + 下一个游标
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Batch {
    pub records: Vec<RawRecord>,
    pub next_cursor: Option<Cursor>,
}

/// 连接器错误
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectorError {
    #[error("http: {0}")]
    Http(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("rate_limited: {0}")]
    RateLimited(String),
    #[error("other: {0}")]
    Other(String),
}

/// 记录元数据 / provenance (设计 §Raw Data Ingestion)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordMetadata {
    pub connector: String,
    pub fetch_seq: u64,
    pub content_type: String,
    #[serde(default)]
    pub extra: Value,
}

/// 统一 RawRecord 信封
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRecord {
    pub id: String,
    pub provider_id: String,
    pub source_id: String,
    pub dataset_id: String,
    pub record_type: RecordType,
    pub external_id: Option<String>,
    pub url: Option<String>,
    pub fetched_at: DateTime<Utc>,
    pub observed_at: Option<DateTime<Utc>>,
    /// 跨源去重键 (url/标题/实体 hash)
    pub dedup_key: String,
    pub payload: Value,
    pub metadata: RecordMetadata,
}

impl RawRecord {
    pub fn new(
        provider_id: &str,
        source_id: &str,
        dataset_id: &str,
        record_type: RecordType,
        dedup_key: &str,
        payload: Value,
    ) -> Self {
        Self {
            id: uuid_v4_string(),
            provider_id: provider_id.to_string(),
            source_id: source_id.to_string(),
            dataset_id: dataset_id.to_string(),
            record_type,
            external_id: payload
                .get("id")
                .or_else(|| payload.get("external_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            url: payload
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            fetched_at: Utc::now(),
            observed_at: None,
            dedup_key: dedup_key.to_string(),
            payload,
            metadata: RecordMetadata::default(),
        }
    }
}

fn uuid_v4_string() -> String {
    // 不引入 uuid crate: 用时间+计数器生成足够唯一的 id (入库时云端会换成真 uuid)
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    format!(
        "rr_{}_{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0),
        C.fetch_add(1, Ordering::Relaxed)
    )
}
