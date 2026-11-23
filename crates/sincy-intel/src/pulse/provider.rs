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

/// 榜单/工具类站点的结构化提取策略 (按页面特征自动选择):
/// 1) trend-list: <a class=trend-link>NAME</a> + <span data-count=N> (trends24 等)
/// 2) table-rows: <tr> 行, data-* 属性与数字单元格作为指标 (xbangdan / xpaiming 等榜单表)
/// 3) main-anchors: 仅 <main>/<article> 内的锚点 (兜底), 带导航黑名单。
///
/// 页面 chrome (nav/header/footer/aside/script/style) 先剥离。
pub fn extract_html_items(html: &str, base_url: &str) -> Vec<(String, String, Value)> {
    use regex::Regex;
    let mut stripped = html.to_string();
    for t in [
        "nav", "header", "footer", "aside", "script", "style", "noscript",
    ] {
        let re = Regex::new(&format!("(?is)<{t}[^>]*>.*?</{t}>")).unwrap();
        stripped = re.replace_all(&stripped, " ").into_owned();
    }
    let stripped = Regex::new(r"(?is)<!--.*?-->")
        .unwrap()
        .replace_all(&stripped, " ");

    let trends = extract_trend_list(&stripped, base_url);
    if !trends.is_empty() {
        return trends;
    }
    let rows = extract_table_rows(&stripped, base_url);
    if !rows.is_empty() {
        return rows;
    }
    extract_main_anchors(&stripped, base_url)
}

/// 策略1: 趋势列表 (trend-link + data-count)
fn extract_trend_list(html: &str, base_url: &str) -> Vec<(String, String, Value)> {
    use regex::Regex;
    let re = Regex::new(r#"<a[^>]*class="[^"]*trend-link[^"]*"[^>]*>(.*?)</a>[^<]*(?:<span[^>]*data-count="(\d+)"[^>]*>[^<]*</span>)?"#).unwrap();
    let tag = Regex::new(r"<[^>]+>").unwrap();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in re.captures_iter(html) {
        let name = tag.replace_all(&cap[1], "").trim().to_string();
        if name.len() < 2 || name.len() > 80 || !seen.insert(name.clone()) {
            continue;
        }
        let mut payload = serde_json::json!({"text": name, "url": base_url});
        if let Some(c) = cap.get(2) {
            payload["tweet_count"] = Value::from(c.as_str().parse::<u64>().unwrap_or(0));
        }
        out.push((format!("trend:{}", name.to_lowercase()), name, payload));
        if out.len() >= 100 {
            break;
        }
    }
    out
}

/// 策略2: 榜单表格行 — <tr> 的 data-* 属性 + 数字单元格作为指标, 行内首个链接文本为标题
fn extract_table_rows(html: &str, base_url: &str) -> Vec<(String, String, Value)> {
    use regex::Regex;
    let row_re = Regex::new(r"(?is)<tr[^>]*>(.*?)</tr>").unwrap();
    let attr_re = Regex::new(r#"data-([\w-]+)="([^"]*)""#).unwrap();
    let anchor_re = Regex::new(r#"(?is)<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let tag = Regex::new(r"<[^>]+>").unwrap();
    let numcell = Regex::new(r"(?is)<td[^>]*>(.*?)</td>").unwrap();
    let num = Regex::new(r"^[\d][\d,.]*[KkMm\u{4e07}\u{4ebf}]?$").unwrap();
    let mut out = Vec::new();
    for (ri, row) in row_re.captures_iter(html).enumerate() {
        let attrs: Vec<(String, String)> = attr_re
            .captures_iter(&row[0])
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect();
        let anchor = anchor_re.captures(&row[1]);
        let (href, title) = match anchor {
            Some(c) => (
                c[1].to_string(),
                tag.replace_all(&c[2], "").trim().to_string(),
            ),
            None => continue,
        };
        if title.len() < 2 || title.len() > 80 {
            continue;
        }
        let url = if href.starts_with("http") {
            href
        } else if let Some(st) = href.strip_prefix('/') {
            format!("{}/{}", base_url.trim_end_matches('/'), st)
        } else {
            continue;
        };
        let mut payload = serde_json::json!({"url": url, "text": title, "rank": ri + 1});
        for (k, v) in &attrs {
            if let Ok(n) = v.parse::<u64>() {
                payload[k.replace('-', "_")] = Value::from(n);
            }
        }
        for (ci, c) in numcell.captures_iter(&row[1]).enumerate() {
            let t = tag.replace_all(&c[1], "").trim().to_string();
            if num.is_match(&t) {
                payload[format!("cell{ci}")] = Value::from(t);
            }
        }
        let key = attrs
            .iter()
            .find(|(k, _)| k == "h" || k == "id")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| url.clone());
        out.push((format!("row:{key}"), title, payload));
        if out.len() >= 100 {
            break;
        }
    }
    out
}

/// 策略3: <main>/<article> 内锚点兜底 + 导航/语言黑名单
fn extract_main_anchors(html: &str, base_url: &str) -> Vec<(String, String, Value)> {
    use regex::Regex;
    const STOP: &[&str] = &[
        "首页",
        "home",
        "login",
        "登录",
        "sign up",
        "注册",
        "more",
        "更多",
        "blog",
        "faq",
        "statistics",
        "methodology",
        "monitoring",
        "follow us",
        "see the full",
        "leaderboard",
        "privacy",
        "terms",
        "contact",
        "about",
        "help",
        "support",
        "careers",
        "press",
        "developers",
        "status",
        "language",
        "中文",
        "日本語",
        "한국어",
        "english",
        "español",
        "français",
        "português",
        "portuguese",
        "spanish",
        "french",
        "german",
        "deutsch",
        "italiano",
        "japanese",
        "korean",
        "chinese",
        "thai",
        "türkçe",
        "turkish",
        "bahasa",
        "arabic",
        "russian",
        "ukrainian",
        "vietnamese",
        "hindi",
        "indonesian",
    ];
    let main = Regex::new(r"(?is)<(?:main|article)[^>]*>(.*?)</(?:main|article)>").unwrap();
    let scope = main
        .captures(html)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| html.to_string());
    let anchor = Regex::new(r#"<a\s[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let tag = Regex::new(r"<[^>]+>").unwrap();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in anchor.captures_iter(&scope) {
        let href = cap[1].trim();
        if href.starts_with('#') || href.starts_with("javascript:") || href.starts_with("mailto:") {
            continue;
        }
        let raw = cap[2].to_string();
        let text = tag.replace_all(&raw, "").trim().to_string();
        let tl = text.to_lowercase();
        if text.len() < 3 || text.len() > 120 || !seen.insert(text.clone()) {
            continue;
        }
        if STOP
            .iter()
            .any(|w| tl == *w || (tl.len() < 24 && tl.contains(w)))
        {
            continue;
        }
        let url = if href.starts_with("http") {
            href.to_string()
        } else if let Some(st) = href.strip_prefix('/') {
            format!("{}/{}", base_url.trim_end_matches('/'), st)
        } else {
            continue;
        };
        out.push((
            format!("html:{}", url),
            text.clone(),
            serde_json::json!({"url": url, "text": text}),
        ));
        if out.len() >= 100 {
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
    fn html_extractor_prefers_trend_list() {
        let html = r#"<html><body><ol class="trend-card__list">
            <li><span class="trend-name"><a href="https://twitter.com/search?q=X" class="trend-link">Jensen Huang</a><span class="tweet-count" data-count="219093">219K</span></span></li>
            <li><span class="trend-name"><a href="https://twitter.com/search?q=Y" class="trend-link">#AI</a><span class="tweet-count" data-count="88000">88K</span></span></li>
        </ol></body></html>"#;
        let items = extract_html_items(html, "https://trends24.in/");
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .any(|(_, t, p)| t == "Jensen Huang" && p["tweet_count"] == 219093)
        );
    }

    #[test]
    fn html_extractor_reads_ranking_table_rows() {
        let html = r#"<html><body><table class="tb"><tbody>
            <tr data-h="mi88860" data-followers="11543" data-views-30="7331993"><td class="tc-r"></td><td class="tc-w"><a href="https://x.com/mi88860"><span>大鱼🐟</span></a></td><td>7,331,993</td></tr>
            <tr data-h="elonmusk" data-followers="200000000"><td class="tc-r"></td><td class="tc-w"><a href="https://x.com/elonmusk"><span>Elon Musk</span></a></td><td>1亿</td></tr>
        </tbody></table></body></html>"#;
        let items = extract_html_items(html, "https://xbangdan.com/accounts/");
        assert_eq!(items.len(), 2);
        let e = items.iter().find(|(_, t, _)| t == "Elon Musk").unwrap();
        assert_eq!(e.2["followers"], 200000000u64);
        assert_eq!(e.2["rank"], 2);
    }

    #[test]
    fn html_extractor_falls_back_to_main_anchors_without_nav_junk() {
        let html = r#"<html><body>
            <nav><a href="/blog">Blog</a><a href="/lang">Portuguese</a></nav>
            <main>
              <a href="https://x.com/alice/status/1">Alice post about LLM agents</a>
              <a href="/accounts/">账号榜</a>
            </main>
            <footer><a href="/privacy">Privacy</a></footer>
        </body></html>"#;
        let items = extract_html_items(html, "https://example.com");
        assert!(
            items
                .iter()
                .any(|(_, t, _)| t == "Alice post about LLM agents")
        );
        assert!(items.iter().any(|(_, t, _)| t == "账号榜"));
        assert!(
            !items
                .iter()
                .any(|(_, t, _)| t == "Blog" || t == "Portuguese" || t == "Privacy")
        );
    }

    #[test]
    fn json_extractor_pulls_titled_objects() {
        let body = r#"{"data":[{"id":"1","title":"Rust 2.0","views":12000},{"id":"2","name":"LLM agents","views":3400}]}"#;
        let items = extract_json_items(body);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(_, t, _)| t == "Rust 2.0"));
    }
}
