//! 通用 Provider 接口 (Sincy Pulse §3) + 内置 feed provider 实现。
//!
//! 引擎只认识 IntelligenceEntity / Observation, 不认识任何具体站点。

use serde_json::Value;

use super::entity::IntelligenceEntity;
use super::manifest::{AcquisitionType, ProviderManifest};

/// Provider 健康状态
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderHealth {
    pub provider_id: String,
    pub status: String, // healthy / degraded / failed / never
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    pub runs_24h: i64,
    pub ok_rate: f64,
}

/// fetcher 签名: (source_id, feed_label) → [(dedup_key, title, payload)]
pub type FeedFetcher = std::sync::Arc<
    dyn Fn(
            &str,
            &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<Vec<(String, String, Value)>, String>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// 通用 Provider 接口 (设计 §3)。
/// 复杂 provider 写自己的 adapter 实现这个 trait;
/// 简单 feed 类 provider 用 BuiltinFeedProvider 包一层即可。
#[async_trait::async_trait]
pub trait IntelligenceProvider: Send + Sync {
    fn manifest(&self) -> &ProviderManifest;
    fn provider_id(&self) -> &str {
        &self.manifest().id
    }
    fn capabilities(&self) -> Vec<String> {
        self.manifest().capabilities.clone()
    }
    /// 采集一个 dataset, 归一化成统一实体 (每条实体自带 observations)
    async fn fetch_dataset(&self, dataset_id: &str) -> Result<Vec<IntelligenceEntity>, String>;
}

/// 内置 feed provider: 复用 intel_collector 的 FeedConfig 拉取 + 解析,
/// 把条目包装成 IntelligenceEntity (canonical_key 与现有 dedup 口径一致)。
pub struct BuiltinFeedProvider {
    manifest: ProviderManifest,
    /// (dataset_id, source_id, feed_label)
    feeds: Vec<(String, String, String)>,
    fetcher: FeedFetcher,
}

impl BuiltinFeedProvider {
    pub fn new(
        manifest: ProviderManifest,
        feeds: Vec<(String, String, String)>,
        fetcher: FeedFetcher,
    ) -> Self {
        Self {
            manifest,
            feeds,
            fetcher,
        }
    }
}

#[async_trait::async_trait]
impl IntelligenceProvider for BuiltinFeedProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn fetch_dataset(&self, dataset_id: &str) -> Result<Vec<IntelligenceEntity>, String> {
        let ds = self
            .manifest
            .dataset(dataset_id)
            .ok_or_else(|| format!("unknown dataset {dataset_id}"))?;
        let (_, source_id, label) = self
            .feeds
            .iter()
            .find(|(d, _, _)| d == dataset_id)
            .ok_or_else(|| format!("dataset {dataset_id} not mapped to a feed"))?;
        if self.manifest.acquisition.acq_type != AcquisitionType::BuiltinFeed {
            return Err("not a builtin_feed provider".into());
        }
        let items = (self.fetcher)(source_id, label).await?;
        let entity_type = if ds.entity_type.is_empty() {
            "trend".to_string()
        } else {
            ds.entity_type.clone()
        };
        let mut out = Vec::new();
        for (dk, title, payload) in items {
            // 与 intel_collector 口径一致: 第一元是 dedup_key, 第二元才是标题
            let key = format!("{source_id}:{entity_type}:{dk}");
            let mut e =
                IntelligenceEntity::new(&entity_type, &self.manifest.platform, &key, &title);
            e.payload = payload.clone();
            // 数值指标进 observation, 供聚合引擎跨源合并
            for (k, v) in payload.as_object().into_iter().flatten() {
                if v.is_number() {
                    e.observe(&self.manifest.id, dataset_id, k, v.clone());
                }
            }
            if e.observations.is_empty() {
                e.observe(&self.manifest.id, dataset_id, "seen", Value::from(1));
            }
            out.push(e);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn builtin_feed_provider_wraps_items_as_entities() {
        let manifest = ProviderManifest::parse(&json!({
            "id": "hotrank", "name": "HotRank", "platform": "github",
            "acquisition": {"type": "builtin_feed"},
            "datasets": [{"id": "gh_trending", "update_frequency_secs": 3600, "entity_type": "trend", "feed_label": "github trending (llm)"}],
            "legal_status": "public", "reliability": 0.8
        }))
        .unwrap();
        let fetcher: FeedFetcher = std::sync::Arc::new(|_s: &str, _l: &str| {
            Box::pin(async move {
                Ok(vec![(
                    "dk1".into(),
                    "llm repo".into(),
                    json!({"url": "https://github.com/x/y", "stars": 42000}),
                )])
            })
        });
        let p = BuiltinFeedProvider::new(
            manifest,
            vec![(
                "gh_trending".into(),
                "hotrank-hub".into(),
                "github trending (llm)".into(),
            )],
            fetcher,
        );
        let entities = p.fetch_dataset("gh_trending").await.unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "trend");
        assert!(entities[0].canonical_key.starts_with("hotrank-hub:trend:"));
        assert_eq!(entities[0].observations[0].metric, "stars");
        assert_eq!(entities[0].observations[0].value, json!(42000));
    }
}
