//! 内置 Processor (v1 规则版, 设计 §7 各 pipeline): Article / Trend / Ranking / SocialPost 为主,
//! 其余形状走通用类型处理器; universal_* 处理器挂在每条 pipeline 末尾 (Universal Intelligence Layer)。

use chrono::Utc;
use serde_json::{Value, json};

use super::processor::{ProcessingContext, ProcessingResult, Processor};
use super::record::{RawRecord, RecordType};
use super::signals::{IntelEntity, Signal, SignalType};
use crate::industry;

fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}

fn lang_of(text: &str) -> String {
    let cjk = text
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .count();
    if cjk >= 3 { "zh".into() } else { "en".into() }
}

fn base_entity(record: &RawRecord, entity_type: &str, title: &str) -> IntelEntity {
    IntelEntity {
        entity_id: format!("ent_{}", record.dedup_key),
        entity_type: entity_type.to_string(),
        source_id: record.source_id.clone(),
        dataset_id: record.dataset_id.clone(),
        external_id: record.external_id.clone(),
        url: record.url.clone(),
        title: title.to_string(),
        payload: record.payload.clone(),
        industry_tags: Vec::new(),
        topic_tags: Vec::new(),
        entities: Vec::new(),
        geography: Vec::new(),
        language: String::new(),
        provenance: json!({
            "provider_id": record.provider_id,
            "source_id": record.source_id,
            "dataset_id": record.dataset_id,
            "connector": record.metadata.connector,
            "record_id": record.id,
            "fetched_at": record.fetched_at,
        }),
        confidence: 0.6,
        quality_score: 0.5,
        relevance_score: 0.0,
        observed_at: record.observed_at.unwrap_or(record.fetched_at),
        created_at: now(),
        updated_at: now(),
    }
}

fn mk_signal(
    rt: SignalType,
    record: &RawRecord,
    entity_id: Option<&str>,
    strength: f64,
    payload: Value,
) -> Signal {
    Signal {
        signal_id: format!("sig_{}_{}", record.dedup_key, rt.as_str()),
        signal_type: rt,
        entity_id: entity_id.map(|s| s.to_string()),
        source_id: record.source_id.clone(),
        industry_id: None,
        topic_id: None,
        strength,
        confidence: 0.6,
        importance: strength,
        observed_at: now(),
        payload,
    }
}

fn text_of(record: &RawRecord) -> String {
    let p = &record.payload;
    format!(
        "{} {}",
        p.get("title")
            .or_else(|| p.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        p.get("summary")
            .or_else(|| p.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    )
}

// ---------- Type processors ----------

/// Article Pipeline: RSS/News/Blog → article entity + industry_event / topic_signal / opportunity_candidate
pub struct ArticleProcessor;
#[async_trait::async_trait]
impl Processor for ArticleProcessor {
    fn processor_id(&self) -> &str {
        "article"
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![RecordType::Article]
    }
    async fn process(
        &self,
        record: &RawRecord,
        _ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let title = record
            .payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() {
            return Ok(ProcessingResult::default());
        }
        let e = base_entity(record, "article", &title);
        let mut r = ProcessingResult::default();
        let sig_payload = json!({"title": title, "url": record.url});
        r.signals.push(mk_signal(
            SignalType::IndustryEvent,
            record,
            Some(&e.entity_id),
            0.5,
            sig_payload.clone(),
        ));
        r.signals.push(mk_signal(
            SignalType::TopicSignal,
            record,
            Some(&e.entity_id),
            0.4,
            sig_payload.clone(),
        ));
        r.signals.push(mk_signal(
            SignalType::OpportunityCandidate,
            record,
            Some(&e.entity_id),
            0.3,
            sig_payload,
        ));
        r.entities.push(e);
        Ok(r)
    }
}

/// Trend Pipeline: keyword/interest/time-series → trend entity + trend_series upsert + trend_rising/topic_emerging
pub struct TrendProcessor;
#[async_trait::async_trait]
impl Processor for TrendProcessor {
    fn processor_id(&self) -> &str {
        "trend"
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![RecordType::Trend]
    }
    async fn process(
        &self,
        record: &RawRecord,
        ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let p = &record.payload;
        let topic = p
            .get("keyword")
            .or_else(|| p.get("topic"))
            .or_else(|| p.get("text"))
            .or_else(|| p.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if topic.is_empty() {
            return Ok(ProcessingResult::default());
        }
        let interest = p
            .get("interest")
            .or_else(|| p.get("tweet_count"))
            .or_else(|| p.get("count"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let geo = p
            .get("region")
            .or_else(|| p.get("geography"))
            .and_then(|v| v.as_str())
            .unwrap_or("global")
            .to_string();
        let hist_key = format!("{}:{}", record.dataset_id, topic.to_lowercase());
        let prev = ctx.history.get(&hist_key).and_then(|v| v.as_f64());
        let growth = match prev {
            Some(pr) if pr > 0.0 => (interest - pr) / pr * 100.0,
            _ => 0.0,
        };
        let mut e = base_entity(record, "trend", &topic);
        e.payload["interest"] = json!(interest);
        e.payload["geography"] = json!(geo);
        e.payload["growth_rate"] = json!(growth);
        let (st, rt) = if growth > 20.0 {
            (0.9, SignalType::TrendRising)
        } else if growth > 0.0 {
            (0.6, SignalType::TopicEmerging)
        } else {
            (0.3, SignalType::TopicSignal)
        };
        let mut r = ProcessingResult::default();
        r.signals.push(mk_signal(
            rt,
            record,
            Some(&e.entity_id),
            st,
            json!({"topic": topic, "interest": interest, "growth_rate": growth, "geo": geo}),
        ));
        r.side_effects.push(json!({
            "kind": "trend_series_upsert",
            "key": hist_key,
            "value": interest,
            "observed_at": now(),
            "source_id": record.source_id,
        }));
        r.entities.push(e);
        Ok(r)
    }
}

/// Ranking Pipeline: 榜单行 → ranking 快照 + social_account/social_post 实体 + growth/viral 信号
pub struct RankingProcessor;
#[async_trait::async_trait]
impl Processor for RankingProcessor {
    fn processor_id(&self) -> &str {
        "ranking"
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![RecordType::Ranking]
    }
    async fn process(
        &self,
        record: &RawRecord,
        ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let p = &record.payload;
        let title = p
            .get("text")
            .or_else(|| p.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if title.is_empty() {
            return Ok(ProcessingResult::default());
        }
        let rank = p.get("rank").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let url = record.url.clone().unwrap_or_default();
        let is_account = url.contains("x.com/") && !url.contains("/status/");
        let entity_type = if is_account {
            "social_account"
        } else {
            "social_post"
        };
        let views = [
            "views_30d",
            "views_t",
            "tweet_count",
            "views",
            "followers",
            "fans",
        ]
        .iter()
        .find_map(|k| p.get(*k).and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
        let mut e = base_entity(record, entity_type, &title);
        e.payload["rank"] = json!(rank);
        e.payload["views"] = json!(views);
        let mut r = ProcessingResult::default();
        // 增速: 与历史 rank 比较
        let hist_key = format!(
            "{}:{}:rank",
            record.dataset_id,
            record.external_id.clone().unwrap_or_else(|| title.clone())
        );
        if let Some(prev_rank) = ctx.history.get(&hist_key).and_then(|v| v.as_f64())
            && prev_rank > rank
            && rank > 0.0
        {
            r.signals.push(mk_signal(
                SignalType::CreatorGrowing,
                record,
                Some(&e.entity_id),
                0.7,
                json!({"prev_rank": prev_rank, "rank": rank}),
            ));
        }
        if views >= 1_000_000.0 || (rank > 0.0 && rank <= 10.0 && views >= 100_000.0) {
            r.signals.push(mk_signal(
                SignalType::ContentGoingViral,
                record,
                Some(&e.entity_id),
                (views / 1e7).min(1.0),
                json!({"views": views, "rank": rank}),
            ));
        }
        r.side_effects.push(json!({
            "kind": "ranking_snapshot",
            "dataset_id": record.dataset_id,
            "external_id": record.external_id,
            "title": title,
            "rank": rank,
            "metrics": p,
            "observed_at": now(),
        }));
        r.side_effects.push(json!({
            "kind": "history_upsert",
            "key": hist_key,
            "value": rank,
        }));
        r.entities.push(e);
        Ok(r)
    }
}

/// SocialPost Pipeline: 帖子 → social_post entity + viral 信号
pub struct SocialPostProcessor;
#[async_trait::async_trait]
impl Processor for SocialPostProcessor {
    fn processor_id(&self) -> &str {
        "social_post"
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![RecordType::SocialPost]
    }
    async fn process(
        &self,
        record: &RawRecord,
        _ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let title = text_of(record);
        if title.is_empty() {
            return Ok(ProcessingResult::default());
        }
        let p = &record.payload;
        let engagement = [
            "views",
            "likes",
            "retweets",
            "replies",
            "tweet_count",
            "count",
        ]
        .iter()
        .find_map(|k| p.get(*k).and_then(|v| v.as_f64()))
        .unwrap_or(0.0);
        let e = base_entity(record, "social_post", &title);
        let mut r = ProcessingResult::default();
        if engagement >= 100_000.0 {
            r.signals.push(mk_signal(
                SignalType::ContentGoingViral,
                record,
                Some(&e.entity_id),
                (engagement / 1e6).min(1.0),
                json!({"engagement": engagement}),
            ));
        }
        r.entities.push(e);
        Ok(r)
    }
}

/// 通用类型处理器: 其余形状 (event/metric/video/product/research/search/account) → 对应 entity
pub struct GenericTypeProcessor {
    pub id: String,
    pub rt: RecordType,
    pub entity_type: String,
}
#[async_trait::async_trait]
impl Processor for GenericTypeProcessor {
    fn processor_id(&self) -> &str {
        &self.id
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![self.rt]
    }
    async fn process(
        &self,
        record: &RawRecord,
        _ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let title = text_of(record);
        if title.is_empty() {
            return Ok(ProcessingResult::default());
        }
        let e = base_entity(record, &self.entity_type, &title);
        Ok(ProcessingResult {
            entities: vec![e],
            ..Default::default()
        })
    }
}

// ---------- Universal processors (挂在每条 pipeline 末尾) ----------

/// Universal: 行业/主题分类 + 语言检测 + 质量分 (设计 §8 Universal Metadata)
pub struct UniversalClassifyProcessor;
#[async_trait::async_trait]
impl Processor for UniversalClassifyProcessor {
    fn processor_id(&self) -> &str {
        "universal_classify"
    }
    fn supported_record_types(&self) -> Vec<RecordType> {
        vec![
            RecordType::Article,
            RecordType::Trend,
            RecordType::Ranking,
            RecordType::SocialPost,
            RecordType::SocialAccount,
            RecordType::Event,
            RecordType::Metric,
            RecordType::Video,
            RecordType::Product,
            RecordType::ResearchPaper,
            RecordType::SearchResult,
            RecordType::Unknown,
        ]
    }
    async fn process(
        &self,
        record: &RawRecord,
        _ctx: &ProcessingContext, // brand_profile 不再于入库层使用 (per-brand 相关性下游回填)
    ) -> Result<ProcessingResult, String> {
        // 对已有实体做富化: 由 engine 在 pipeline 末尾对累积 entities 调用本 processor 的 enrich
        let text = text_of(record);
        let inds = industry::classify_industries(&text);
        let mut r = ProcessingResult::default();
        let mut e = base_entity(record, "classified", &text);
        e.industry_tags = inds.iter().map(|(i, _)| i.clone()).collect();
        e.topic_tags = crate::topic::extract_topics(&text, 5);
        e.language = lang_of(&text);
        e.quality_score = ((text.len() as f64 / 200.0).min(1.0) * 0.6 + 0.4).min(1.0);
        // relevance 不在入库层算: 情报全局共享, per-brand 相关性由下游
        // /brands/{id}/intel-feed 回填 intel_entity_brand_relevance (见 migration 0041)
        // 不产出新实体, 只产出富化结果供 engine 合并
        r.side_effects.push(json!({"kind": "enrich", "record_id": record.id, "industry_tags": e.industry_tags, "topic_tags": e.topic_tags, "language": e.language, "quality_score": e.quality_score}));
        Ok(r)
    }
}

/// 默认 processor 集合 (云端 engine 初始化用)
pub fn default_processors() -> super::processor::ProcessorRegistry {
    use super::processor::ProcessorRegistry;
    let mut reg = ProcessorRegistry::new();
    reg.register(Box::new(ArticleProcessor));
    reg.register(Box::new(TrendProcessor));
    reg.register(Box::new(RankingProcessor));
    reg.register(Box::new(SocialPostProcessor));
    reg.register(Box::new(GenericTypeProcessor {
        id: "social_account".into(),
        rt: RecordType::SocialAccount,
        entity_type: "social_account".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "research".into(),
        rt: RecordType::ResearchPaper,
        entity_type: "research_paper".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "event".into(),
        rt: RecordType::Event,
        entity_type: "event".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "metric".into(),
        rt: RecordType::Metric,
        entity_type: "metric".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "video".into(),
        rt: RecordType::Video,
        entity_type: "video".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "product".into(),
        rt: RecordType::Product,
        entity_type: "product".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "search".into(),
        rt: RecordType::SearchResult,
        entity_type: "search_result".into(),
    }));
    reg.register(Box::new(GenericTypeProcessor {
        id: "universal_only".into(),
        rt: RecordType::Unknown,
        entity_type: "unknown".into(),
    }));
    reg.register(Box::new(UniversalClassifyProcessor));
    reg
}
