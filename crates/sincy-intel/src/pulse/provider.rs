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

/// web 抓取 fetcher 签名: url → (body, content_type)。HTTP 客户端由调用方注入。
pub type WebFetcher = std::sync::Arc<
    dyn Fn(
            &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(String, String), String>> + Send>,
        > + Send
        + Sync,
>;

/// 通用 web provider: 按 manifest 里每个 dataset 的 url 抓取页面/JSON,
/// 归一化成统一实体。引擎不认识具体站点, 解析只依赖通用 HTML/JSON 结构。
pub struct WebScrapeProvider {
    manifest: ProviderManifest,
    fetcher: WebFetcher,
}

impl WebScrapeProvider {
    pub fn new(manifest: ProviderManifest, fetcher: WebFetcher) -> Self {
        Self { manifest, fetcher }
    }
}

/// 从 HTML 里提取候选条目: 锚点 (href + 文本) 与话题标签, 附近数字作为 count 观测。
pub fn extract_html_items(html: &str, base_url: &str) -> Vec<(String, String, Value)> {
    use regex::Regex;
    let mut out: Vec<(String, String, Value)> = Vec::new();
    let anchor = Regex::new(r#"<a\s[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let tag = Regex::new(r"<[^>]+>").unwrap();
    let num = Regex::new(r"[\d][\d,.]*\s*[KkMm]?").unwrap();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cap in anchor.captures_iter(html) {
        let href = cap[1].trim();
        let text = tag.replace_all(&cap[2], "").trim().to_string();
        if text.len() < 3 || text.len() > 120 || seen.contains(&text) {
            continue;
        }
        // 跳过纯导航
        if matches!(
            text.as_str(),
            "首页" | "Home" | "登录" | "Login" | "注册" | "Sign up" | "更多" | "More"
        ) {
            continue;
        }
        seen.insert(text.clone());
        let url = if href.starts_with("http") {
            href.to_string()
        } else if let Some(stripped) = href.strip_prefix('/') {
            format!("{}/{}", base_url.trim_end_matches('/'), stripped)
        } else {
            continue;
        };
        let mut payload = serde_json::json!({"url": url, "text": text});
        if let Some(m) = num.find(&cap[2]) {
            payload["count"] = Value::from(m.as_str().to_string());
        }
        let key = format!("html:{}", url);
        out.push((key, text, payload));
        if out.len() >= 200 {
            break;
        }
    }
    // 话题标签 (#xxx) 单独成 trend 实体
    let hashtag = Regex::new(r"#([\p{L}\p{N}_]{2,60})").unwrap();
    let mut hseen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for cap in hashtag.captures_iter(html) {
        let tag_text = format!("#{}", &cap[1]);
        if hseen.contains(&tag_text) {
            continue;
        }
        hseen.insert(tag_text.clone());
        out.push((
            format!("tag:{}", &cap[1]),
            tag_text.clone(),
            serde_json::json!({"hashtag": tag_text}),
        ));
        if out.len() >= 300 {
            break;
        }
    }
    out
}

/// 从 JSON body 里提取条目: 找到第一个对象数组, 取 title/text/name/keyword 字段 + 数值字段。
pub fn extract_json_items(body: &str) -> Vec<(String, String, Value)> {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    fn walk(v: &Value, out: &mut Vec<(String, String, Value)>) {
        match v {
            Value::Array(arr) => {
                for item in arr {
                    if let Value::Object(obj) = item {
                        let title = ["title", "text", "name", "keyword", "topic", "hashtag"]
                            .iter()
                            .find_map(|k| obj.get(*k).and_then(|x| x.as_str()))
                            .unwrap_or("")
                            .to_string();
                        if title.len() >= 2 {
                            let key = obj
                                .get("id")
                                .or_else(|| obj.get("url"))
                                .map(|x| x.to_string())
                                .unwrap_or_else(|| title.clone());
                            out.push((format!("json:{key}"), title, item.clone()));
                        }
                    } else {
                        walk(item, out);
                    }
                }
            }
            Value::Object(obj) => {
                for (_, val) in obj {
                    walk(val, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&v, &mut out);
    out.truncate(200);
    out
}

#[async_trait::async_trait]
impl IntelligenceProvider for WebScrapeProvider {
    fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    async fn fetch_dataset(&self, dataset_id: &str) -> Result<Vec<IntelligenceEntity>, String> {
        let ds = self
            .manifest
            .dataset(dataset_id)
            .ok_or_else(|| format!("unknown dataset {dataset_id}"))?;
        let url = ds
            .url
            .clone()
            .ok_or_else(|| format!("dataset {dataset_id} 缺 url"))?;
        let (body, content_type) = (self.fetcher)(&url).await?;
        let items = if content_type.contains("json")
            || body.trim_start().starts_with('{')
            || body.trim_start().starts_with('[')
        {
            extract_json_items(&body)
        } else {
            extract_html_items(&body, &url)
        };
        let entity_type = if ds.entity_type.is_empty() {
            "trend".to_string()
        } else {
            ds.entity_type.clone()
        };
        let base = url.trim_end_matches('/').to_string();
        let mut out = Vec::new();
        for (dk, title, payload) in items {
            let key = format!("{}:{}:{}", self.manifest.id, entity_type, dk);
            let mut e =
                IntelligenceEntity::new(&entity_type, &self.manifest.platform, &key, &title);
            e.payload = payload.clone();
            // 数值字段进 observation 供跨源聚合; url 收进 payload
            if let Some(obj) = payload.as_object() {
                for (k, v) in obj {
                    if v.is_number() {
                        e.observe(&self.manifest.id, dataset_id, k, v.clone());
                    }
                }
            }
            if let Some(u) = payload.get("url").and_then(|x| x.as_str()) {
                e.payload["canonical_url"] = Value::from(u);
            }
            let _ = base;
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

    #[test]
    fn html_extractor_pulls_anchors_and_hashtags() {
        let html = r#"<html><body>
            <a href="https://x.com/alice/status/1">Alice post</a>
            <a href="/accounts/">账号榜</a>
            <span>#AI热门 219K</span>
            <a href="https://x.com/bob/status/2">Bob post</a>
        </body></html>"#;
        let items = extract_html_items(html, "https://example.com");
        assert!(items.iter().any(|(_, t, _)| t == "Alice post"));
        assert!(items.iter().any(|(_, t, _)| t == "账号榜"));
        assert!(items.iter().any(|(_, t, _)| t.starts_with('#')));
    }

    #[test]
    fn json_extractor_pulls_titled_objects() {
        let body = r#"{"data":[{"id":"1","title":"Rust 2.0","views":12000},{"id":"2","name":"LLM agents","views":3400}]}"#;
        let items = extract_json_items(body);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(_, t, _)| t == "Rust 2.0"));
    }
}
