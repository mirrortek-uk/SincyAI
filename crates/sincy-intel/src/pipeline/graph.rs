//! Pipeline 注册表 (设计 §10/§11): record_type → 有序 processor 链。
//! 不同形状走不同 pipeline, 最后汇入 Universal Intelligence Layer。

use super::processor::ProcessorRegistry;
use super::record::RecordType;

/// 一条 pipeline: 输入 record_type + 有序 processor id 列表 + 输出 entity types + 产出的 signal types
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub id: String,
    pub version: u32,
    pub input_record_type: RecordType,
    pub processor_ids: Vec<String>,
    pub output_entity_types: Vec<String>,
    pub enabled: bool,
}

/// Pipeline 注册表: 按 record_type 路由
#[derive(Default)]
pub struct PipelineRegistry {
    pipelines: Vec<Pipeline>,
}

impl PipelineRegistry {
    pub fn new() -> Self {
        Self {
            pipelines: Vec::new(),
        }
    }
    pub fn register(&mut self, p: Pipeline) {
        self.pipelines.push(p);
    }
    pub fn route(&self, rt: RecordType) -> Option<&Pipeline> {
        self.pipelines
            .iter()
            .find(|p| p.enabled && p.input_record_type == rt)
    }
    pub fn list(&self) -> &[Pipeline] {
        &self.pipelines
    }
    /// 默认 pipeline 配置 (v1): 每种 record_type 一条链
    pub fn with_defaults(processors: &ProcessorRegistry) -> Self {
        let mut reg = Self::new();
        // 通用富化 processor id (universal_* 在所有 pipeline 末尾跑)
        let universal: Vec<String> = processors
            .list()
            .into_iter()
            .filter(|(id, _, _)| id.starts_with("universal_"))
            .map(|(id, _, _)| id)
            .collect();
        let mut mk = |id: &str, rt: RecordType, type_proc: &str, out: Vec<&str>| {
            let mut ids = vec![type_proc.to_string()];
            ids.extend(universal.iter().cloned());
            reg.register(Pipeline {
                id: id.to_string(),
                version: 1,
                input_record_type: rt,
                processor_ids: ids,
                output_entity_types: out.iter().map(|s| s.to_string()).collect(),
                enabled: true,
            });
        };
        mk(
            "article_pipeline",
            RecordType::Article,
            "article",
            vec!["article"],
        );
        mk("trend_pipeline", RecordType::Trend, "trend", vec!["trend"]);
        mk(
            "ranking_pipeline",
            RecordType::Ranking,
            "ranking",
            vec!["ranking", "social_account", "social_post"],
        );
        mk(
            "social_post_pipeline",
            RecordType::SocialPost,
            "social_post",
            vec!["social_post"],
        );
        mk(
            "social_account_pipeline",
            RecordType::SocialAccount,
            "social_account",
            vec!["social_account"],
        );
        mk(
            "research_pipeline",
            RecordType::ResearchPaper,
            "research",
            vec!["research_paper"],
        );
        mk("event_pipeline", RecordType::Event, "event", vec!["event"]);
        mk(
            "metric_pipeline",
            RecordType::Metric,
            "metric",
            vec!["metric"],
        );
        mk("video_pipeline", RecordType::Video, "video", vec!["video"]);
        mk(
            "product_pipeline",
            RecordType::Product,
            "product",
            vec!["product"],
        );
        mk(
            "search_pipeline",
            RecordType::SearchResult,
            "search",
            vec!["search_result"],
        );
        mk(
            "fallback_pipeline",
            RecordType::Unknown,
            "universal_only",
            vec!["unknown"],
        );
        reg
    }
}
