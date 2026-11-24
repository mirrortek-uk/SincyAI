//! Pipeline Engine (设计 §5/§18): ingest → 按 record_type 路由 → 顺序执行 pipeline →
//! 合并实体 + 富化 (universal) → 输出 entities + signals。
//! 传输层抽象: 每个 record_type 一个 subject (`intel.raw.{type}`), 当前为进程内实现,
//! 接 NATS JetStream 时只需把 publish/subscribe 换成对应 subject 的实现。

use serde_json::Value;

use super::graph::{Pipeline, PipelineRegistry};
use super::processor::{ProcessingContext, ProcessingResult, ProcessorRegistry};
use super::record::{RawRecord, RecordType};
use super::signals::IntelEntity;

pub struct PipelineEngine {
    pub pipelines: PipelineRegistry,
    pub processors: ProcessorRegistry,
}

impl PipelineEngine {
    pub fn new(processors: ProcessorRegistry) -> Self {
        let pipelines = PipelineRegistry::with_defaults(&processors);
        Self {
            pipelines,
            processors,
        }
    }

    /// 每个 record_type 一个 bus subject (设计 §5: NATS 每个 kind 一个主题)
    pub fn subject_for(rt: RecordType) -> String {
        rt.subject()
    }

    /// 处理一条 raw record: 路由 → 执行 pipeline → 合并 + universal 富化
    pub async fn ingest(
        &self,
        record: &RawRecord,
        ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let pipeline = match self.pipelines.route(record.record_type) {
            Some(p) => p.clone(),
            None => {
                // 没有 pipeline 的类型走 fallback
                self.pipelines
                    .route(RecordType::Unknown)
                    .cloned()
                    .ok_or_else(|| format!("no pipeline for {:?}", record.record_type))?
            }
        };
        self.run_pipeline(&pipeline, record, ctx).await
    }

    async fn run_pipeline(
        &self,
        pipeline: &Pipeline,
        record: &RawRecord,
        ctx: &ProcessingContext,
    ) -> Result<ProcessingResult, String> {
        let mut merged = ProcessingResult::default();
        let mut enrich: Option<Value> = None;
        for pid in &pipeline.processor_ids {
            let procs = self.processors.for_record_type(record.record_type);
            let p = procs.iter().find(|p| p.processor_id() == pid);
            let p = match p {
                Some(p) => *p,
                None => continue,
            };
            let out = p.process(record, ctx).await?;
            for e in out.entities {
                merged.entities.push(e);
            }
            for s in out.signals {
                merged.signals.push(s);
            }
            for se in out.side_effects {
                if se.get("kind").and_then(|v| v.as_str()) == Some("enrich") {
                    enrich = Some(se);
                } else {
                    merged.side_effects.push(se);
                }
            }
        }
        // universal 富化合并到所有实体
        if let Some(en) = enrich {
            apply_enrich(&mut merged.entities, &en);
        }
        Ok(merged)
    }
}

fn apply_enrich(entities: &mut [IntelEntity], en: &Value) {
    let inds: Vec<String> = serde_json::from_value(en["industry_tags"].clone()).unwrap_or_default();
    let topics: Vec<String> = serde_json::from_value(en["topic_tags"].clone()).unwrap_or_default();
    let lang = en["language"].as_str().unwrap_or("").to_string();
    let q = en["quality_score"].as_f64().unwrap_or(0.5);
    // relevance_score 不再由入库层提供 (per-brand 相关性在下游回填表), 这里不再覆盖
    let rel = en["relevance_score"].as_f64();
    for e in entities.iter_mut() {
        e.industry_tags = inds.clone();
        if e.topic_tags.is_empty() {
            e.topic_tags = topics.clone();
        }
        e.language = lang.clone();
        e.quality_score = q;
        if let Some(rel) = rel {
            e.relevance_score = rel;
        }
        e.confidence = (q * 0.6 + 0.4).min(1.0);
    }
}

/// 构建默认 engine
pub fn default_engine() -> PipelineEngine {
    PipelineEngine::new(super::processors::default_processors())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::record::RawRecord;
    use crate::pipeline::signals::SignalType;
    use serde_json::json;

    fn rec(rt: RecordType, payload: Value) -> RawRecord {
        RawRecord::new("test", "src", "ds", rt, "dk1", payload)
    }

    #[tokio::test]
    async fn article_record_flows_through_article_pipeline() {
        let engine = default_engine();
        let r = rec(
            RecordType::Article,
            json!({"title": "OpenAI 发布新的 LLM agent", "url": "https://x.com/a/1", "summary": "AI 大模型 agent 发布"}),
        );
        let out = engine
            .ingest(&r, &ProcessingContext::default())
            .await
            .unwrap();
        assert!(out.entities.iter().any(|e| e.entity_type == "article"));
        assert!(
            out.signals
                .iter()
                .any(|s| s.signal_type == SignalType::IndustryEvent)
        );
        // universal 富化: 行业 tag 应包含 technology/ai
        let e = &out.entities[0];
        assert!(
            e.industry_tags
                .iter()
                .any(|t| t == "ai" || t == "technology")
        );
        assert_eq!(e.language, "zh");
    }

    #[tokio::test]
    async fn trend_record_with_growth_emits_trend_rising() {
        let engine = default_engine();
        let mut ctx = ProcessingContext::default();
        ctx.history.insert("ds:ai agent".into(), json!(50.0));
        let r = rec(
            RecordType::Trend,
            json!({"keyword": "AI Agent", "interest": 90, "region": "UK"}),
        );
        let out = engine.ingest(&r, &ctx).await.unwrap();
        assert!(
            out.signals
                .iter()
                .any(|s| s.signal_type == SignalType::TrendRising)
        );
        assert!(
            out.side_effects
                .iter()
                .any(|s| s["kind"] == "trend_series_upsert")
        );
    }

    #[tokio::test]
    async fn ranking_record_emits_viral_signal_and_snapshot() {
        let engine = default_engine();
        let r = rec(
            RecordType::Ranking,
            json!({
                "text": "Elon Musk", "url": "https://x.com/elonmusk",
                "rank": 2, "followers": 200000000, "views_30d": 5000000
            }),
        );
        let out = engine
            .ingest(&r, &ProcessingContext::default())
            .await
            .unwrap();
        assert!(
            out.entities
                .iter()
                .any(|e| e.entity_type == "social_account")
        );
        assert!(
            out.signals
                .iter()
                .any(|s| s.signal_type == SignalType::ContentGoingViral)
        );
        assert!(
            out.side_effects
                .iter()
                .any(|s| s["kind"] == "ranking_snapshot")
        );
    }

    #[test]
    fn subjects_are_per_record_type() {
        assert_eq!(
            PipelineEngine::subject_for(RecordType::Trend),
            "intel.raw.trend"
        );
        assert_eq!(
            PipelineEngine::subject_for(RecordType::Ranking),
            "intel.raw.ranking"
        );
    }
}
