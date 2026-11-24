//! 现役可抓取 Connector: HttpConnector 实现 sincy-intel 的 Connector SPI。
//!
//! 传输工具 (rss / http_json / html) 是共享零件——解析复用 intel_collector 的
//! parse_rss / JSON 启发式抽取, 不再维护封闭 Parser enum。
//! 每个 manifest 一个 HttpConnector; fetch 产出 RawRecord 批次 + cursor (幂等)。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};
use sincy_intel::pipeline::{
    Batch, Connector, ConnectorDescriptor, ConnectorError, ConnectorManifest, Cursor, FetchParams,
    RawRecord, RecordMetadata, default_connector_manifests,
};
use sqlx::Row;

/// 基于 HTTP 的通用连接器: 按 manifest 里的 source 逐个抓取并归一化成 RawRecord
pub struct HttpConnector {
    manifest: ConnectorManifest,
    client: reqwest::Client,
}

impl HttpConnector {
    pub fn new(manifest: ConnectorManifest) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("SincyIntel/0.1 (+https://sincy.dev; read-only public feeds)")
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { manifest, client }
    }

    async fn fetch_source(
        &self,
        src: &sincy_intel::pipeline::SourceRef,
    ) -> Result<Vec<RawRecord>, ConnectorError> {
        if !src.enabled {
            return Ok(vec![]);
        }
        // 聚合站由 Sincy Pulse provider 独占采集: 按 connector_type 在发 GET 之前
        // 直接跳过, 避免和 pulse 抓同一批站点 (礼貌/限流红线)
        if self.manifest.descriptor.connector_type == "aggregator" {
            return Ok(vec![]);
        }
        let resp = self
            .client
            .get(&src.url)
            .send()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ConnectorError::RateLimited(src.url.clone()));
        }
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ConnectorError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(ConnectorError::Http(format!("{} {}", status, src.url)));
        }
        let cid = self.manifest.descriptor.id.as_str();
        let items = match (cid, self.manifest.descriptor.connector_type.as_str()) {
            (_, "rss") => crate::intel_collector::parse_rss(&body)
                .into_iter()
                .map(|(title, url, summary, published)| {
                    (
                        title.clone(),
                        url.clone(),
                        json!({"title": title, "url": url, "summary": summary, "published_at": published, "source": src.label}),
                    )
                })
                .collect::<Vec<_>>(),
            (_, "website") => vec![website_extract(&self.client, src, &body).await],
            ("hn-algolia", _) => crate::intel_collector::parse_hn(&body, &src.kind)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("devto-api", _) => crate::intel_collector::parse_devto(&body)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("github-trending", _) => crate::intel_collector::parse_github_trending(&body)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("bilibili-rank", _) => crate::intel_collector::parse_bili_rank(&body)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("wikipedia-rc", _) => crate::intel_collector::parse_wiki_rc(&body)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("gdelt-api", _) => crate::intel_collector::parse_gdelt(&body)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            ("searxng-ext", _) => crate::intel_collector::parse_searxng(&body, &src.label)
                .into_iter().map(|(t, p)| (t, p["url"].as_str().unwrap_or("").to_string(), p)).collect(),
            _ => parse_json_items(&body, &src.label),
        };
        let mut out = Vec::new();
        for (title, url, payload) in items.into_iter().take(100) {
            let dk = crate::intel_collector::dedup_key(&src.source_id, &src.kind, &url, &title);
            let mut rec = RawRecord::new(
                &self.manifest.descriptor.id,
                &src.source_id,
                &self.manifest.descriptor.id,
                src.record_type,
                &dk,
                payload,
            );
            rec.url = Some(url);
            rec.metadata = RecordMetadata {
                connector: self.manifest.descriptor.id.clone(),
                fetch_seq: 0,
                content_type: self.manifest.descriptor.connector_type.clone(),
                extra: json!({"source_label": src.label}),
            };
            out.push(rec);
        }
        Ok(out)
    }
}

/// WebsiteConnector 抓取: 配了 SINCY_FIRECRAWL_API_KEY 走 Firecrawl scrape API,
/// 否则本地去 script/style 后剥标签取正文 (纯 JS 渲染站需 Firecrawl/Crawl4AI 类服务)。
async fn website_extract(
    client: &reqwest::Client,
    src: &sincy_intel::pipeline::SourceRef,
    body: &str,
) -> (String, String, Value) {
    if let Ok(key) = std::env::var("SINCY_FIRECRAWL_API_KEY")
        && !key.is_empty()
        && let Ok(r) = client
            .post("https://api.firecrawl.dev/v1/scrape")
            .bearer_auth(key)
            .json(&json!({"url": src.url, "formats": ["markdown"]}))
            .send()
            .await
        && let Ok(v) = r.json::<Value>().await
    {
        let md = v["data"]["markdown"].as_str().unwrap_or("");
        let title = md
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches('#')
            .trim()
            .to_string();
        let title = if title.is_empty() {
            src.label.clone()
        } else {
            title
        };
        return (
            title.clone(),
            src.url.clone(),
            json!({"title": title, "url": src.url, "content": md.chars().take(8000).collect::<String>(), "source": src.label}),
        );
    }
    let no_script = regex::Regex::new(r"(?s)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>")
        .unwrap()
        .replace_all(body, " ");
    let text = regex::Regex::new(r"<[^>]+>")
        .unwrap()
        .replace_all(&no_script, " ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (
        src.label.clone(),
        src.url.clone(),
        json!({"title": src.label, "url": src.url, "content": text.chars().take(8000).collect::<String>(), "source": src.label}),
    )
}

/// JSON 启发式抽取: 找第一个对象数组 (hits/items/data.list/results/...), 每个元素成一条记录
fn parse_json_items(body: &str, label: &str) -> Vec<(String, String, Value)> {
    let v: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let arr = find_first_array(&v);
    let mut out = Vec::new();
    for item in arr.unwrap_or(&Vec::new()).iter() {
        let title = [
            "title",
            "name",
            "full_name",
            "keyword",
            "topic",
            "headline",
            "query",
        ]
        .iter()
        .find_map(|k| item.get(k).and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
        let url = ["url", "html_url", "link", "permalink", "webUrl"]
            .iter()
            .find_map(|k| item.get(k).and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        if title.is_empty() && url.is_empty() {
            continue;
        }
        let mut payload = item.clone();
        payload["source"] = json!(label);
        out.push((title, url, payload));
    }
    out
}

fn find_first_array(v: &Value) -> Option<&Vec<Value>> {
    if let Some(a) = v.as_array() {
        return Some(a);
    }
    if let Some(o) = v.as_object() {
        for key in [
            "hits", "items", "results", "list", "data", "articles", "entries",
        ] {
            if let Some(a) = o.get(key).and_then(|x| x.as_array()) {
                return Some(a);
            }
            if let Some(inner) = o
                .get(key)
                .and_then(|x| x.get("list"))
                .and_then(|x| x.as_array())
            {
                return Some(inner);
            }
        }
        // 递归一层
        for (_, x) in o.iter() {
            if let Some(a) = find_first_array(x) {
                return Some(a);
            }
        }
    }
    None
}

#[async_trait::async_trait]
impl Connector for HttpConnector {
    fn descriptor(&self) -> &ConnectorDescriptor {
        &self.manifest.descriptor
    }

    async fn fetch(
        &self,
        params: &FetchParams,
        _cursor: Option<Cursor>,
    ) -> Result<Batch, ConnectorError> {
        let mut records = Vec::new();
        let mut last: Option<String> = None;
        for src in &self.manifest.sources {
            if !params.dataset_id.is_empty() && params.dataset_id != self.manifest.descriptor.id {
                continue;
            }
            match self.fetch_source(src).await {
                Ok(mut rs) => {
                    if let Some(r) = rs.last() {
                        last = Some(r.dedup_key.clone());
                    }
                    records.append(&mut rs);
                }
                Err(e) => {
                    tracing::warn!(connector = %self.manifest.descriptor.id, source = %src.source_id, error = %e, "connector fetch source failed");
                }
            }
        }
        Ok(Batch {
            next_cursor: last.map(|v| Cursor { value: v }),
            records,
        })
    }
}

/// 全部内置连接器注册表 (manifest 驱动, 新增源只改 manifest.rs)
pub fn connector_registry() -> &'static Vec<Box<dyn Connector>> {
    static REG: OnceLock<Vec<Box<dyn Connector>>> = OnceLock::new();
    REG.get_or_init(|| {
        default_connector_manifests()
            .into_iter()
            .map(|m| Box::new(HttpConnector::new(m)) as Box<dyn Connector>)
            .collect()
    })
}

/// 手动/调度触发一个连接器: 抓取全部启用源 → 入管线。返回 (抓取数, 新处理数)
pub async fn run_connector(pool: &sqlx::PgPool, id: &str) -> Result<(usize, usize), String> {
    let reg = connector_registry();
    let conn = reg
        .iter()
        .find(|c| c.descriptor().id == id)
        .ok_or_else(|| format!("connector {id} not found"))?;
    run_connector_instance(pool, conn.as_ref()).await
}

/// 跑一个连接器实例 (注册表之外的自定义 manifest / 测试可直接调用)
pub async fn run_connector_instance(
    pool: &sqlx::PgPool,
    conn: &dyn Connector,
) -> Result<(usize, usize), String> {
    let id = conn.descriptor().id.clone();
    let batch = conn
        .fetch(&FetchParams::default(), None)
        .await
        .map_err(|e| e.to_string())?;
    let fetched = batch.records.len();
    let mut processed = 0;
    for rec in &batch.records {
        match super::ingest_record(pool, rec).await {
            Ok(true) => {
                processed += 1;
                mirror_legacy_item(pool, rec).await;
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(dedup = %rec.dedup_key, error = %e, "pipeline ingest failed"),
        }
    }
    // 健康审计: 给每个覆盖到的 source 续 intel_collection_jobs
    for src in batch_record_sources(&batch) {
        upsert_job(
            pool,
            &src,
            "idle",
            Some(fetched as i32),
            Some(processed as i32),
        )
        .await;
    }
    // 记录游标 (幂等轮询)
    if let Some(c) = &batch.next_cursor {
        let _ = sqlx::query(
            "INSERT INTO connector_cursors (connector_id, dataset_id, cursor_value, updated_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (connector_id, dataset_id) DO UPDATE SET cursor_value = EXCLUDED.cursor_value, updated_at = now()",
        )
        .bind(id)
        .bind("")
        .bind(&c.value)
        .execute(pool)
        .await;
    }
    Ok((fetched, processed))
}

/// DB 驱动的自定义 RSS 连接器: intel_custom_feeds 表里启用的每一行就是一个源,
/// 零代码接入 (新增源 = 加一行配置, 或 POST /intel/feeds/import 批量导入)。
pub async fn custom_rss_manifest(pool: &sqlx::PgPool) -> Option<ConnectorManifest> {
    let rows = sqlx::query(
        "SELECT f.id, f.label, f.url FROM intel_custom_feeds f
         JOIN intelligence_sources s ON s.id = f.id AND s.enabled
         WHERE f.enabled ORDER BY f.id",
    )
    .fetch_all(pool)
    .await
    .ok()?;
    if rows.is_empty() {
        return None;
    }
    let sources = rows
        .iter()
        .map(|r| sincy_intel::pipeline::SourceRef {
            source_id: r.get::<String, _>("id"),
            label: r.get::<String, _>("label"),
            url: r.get::<String, _>("url"),
            kind: "news".into(),
            record_type: sincy_intel::pipeline::RecordType::Article,
            enabled: true,
        })
        .collect::<Vec<_>>();
    Some(ConnectorManifest {
        descriptor: ConnectorDescriptor {
            id: "rss-custom".into(),
            name: "自定义 RSS 源 (intel_custom_feeds)".into(),
            connector_type: "rss".into(),
            capabilities: vec!["article_discovery".into()],
            datasets: vec![sincy_intel::pipeline::DatasetDecl {
                id: "rss-custom".into(),
                record_type: sincy_intel::pipeline::RecordType::Article,
                url: None,
            }],
            default_interval_secs: 1800,
            rate_limit_per_min: 30,
            auth: Default::default(),
        },
        sources,
        fetch_impl: "http:rss".into(),
    })
}

/// 跑一轮 DB 自定义 RSS 源。返回 (抓取数, 新处理数, 错误)
pub async fn run_custom_feeds(pool: &sqlx::PgPool) -> (usize, usize, Option<String>) {
    match custom_rss_manifest(pool).await {
        Some(m) => {
            let conn = HttpConnector::new(m);
            match run_connector_instance(pool, &conn).await {
                Ok((f, p)) => (f, p, None),
                Err(e) => (0, 0, Some(e)),
            }
        }
        None => (0, 0, None),
    }
}

/// 调度器: 周期跑所有连接器 — 现役唯一抓取方 (老 intel_collector 定时循环已退役, dedup_key 保证幂等)
pub async fn run_scheduler(pool: sqlx::PgPool) {
    let mut tick = tokio::time::interval(Duration::from_secs(1800));
    loop {
        tick.tick().await;
        let mut total_new = 0usize;
        for conn in connector_registry() {
            let id = conn.descriptor().id.clone();
            match run_connector(&pool, &id).await {
                Ok((f, p)) => {
                    total_new += p;
                    tracing::info!(connector = %id, fetched = f, processed = p, "connector run done")
                }
                Err(e) => tracing::warn!(connector = %id, error = %e, "connector run failed"),
            }
        }
        let (cf, cp, cerr) = run_custom_feeds(&pool).await;
        if cf > 0 || cp > 0 {
            total_new += cp;
            tracing::info!(
                connector = "rss-custom",
                fetched = cf,
                processed = cp,
                "connector run done"
            );
        }
        if let Some(e) = cerr {
            tracing::warn!(connector = "rss-custom", error = %e, "connector run failed");
        }
        let _ = crate::intel::expire_stale_opportunities(&pool).await;
        if total_new > 0
            && let Ok((n, up)) = crate::intel::refresh_opportunities_db(&pool).await
        {
            tracing::info!(
                new_items = total_new,
                opportunities = n,
                upserted = up,
                "opportunity engine refreshed"
            );
        }
    }
}

/// 状态页展示用: 连接器的解析路径 (对应 parse_* / 抓取实现)
pub fn parse_path_for(connector_id: &str, connector_type: &str) -> &'static str {
    match connector_id {
        "hn-algolia" => "parse_hn",
        "devto-api" => "parse_devto",
        "github-trending" | "hotrank-hub" => "parse_github_trending",
        "bilibili-rank" => "parse_bili_rank",
        "wikipedia-rc" => "parse_wiki_rc",
        "gdelt-api" => "parse_gdelt",
        "searxng-ext" => "parse_searxng",
        "website" => "website_extract (Firecrawl/本地剥标签)",
        _ => match connector_type {
            "rss" => "parse_rss",
            "aggregator" => "pulse WebScrapeProvider",
            _ => "parse_json_items (JSON 启发式)",
        },
    }
}

fn batch_record_sources(batch: &Batch) -> Vec<String> {
    let mut v: Vec<String> = batch.records.iter().map(|r| r.source_id.clone()).collect();
    v.sort();
    v.dedup();
    v
}

/// legacy 镜像: intel_items 保留给机会引擎/趋势接口, 与 RawRecord 管线双写
/// RecordType → legacy intel_items.kind (表上 CHECK 只认老 kind 枚举)
fn legacy_kind(rt: sincy_intel::pipeline::RecordType) -> &'static str {
    use sincy_intel::pipeline::RecordType as R;
    match rt {
        R::Article | R::SearchResult => "news",
        R::Trend | R::Ranking | R::Metric => "trend",
        R::SocialPost | R::SocialAccount | R::Video | R::Comment => "social_content",
        R::Event => "event",
        R::ResearchPaper => "knowledge",
        R::Product | R::Dataset | R::Unknown => "topic",
    }
}

async fn mirror_legacy_item(pool: &sqlx::PgPool, rec: &RawRecord) {
    let title = rec
        .payload
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let summary = rec
        .payload
        .get("summary")
        .or_else(|| rec.payload.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = format!("{title} {summary}");
    let inds = sincy_intel::industry::classify_industries(&text);
    let industries: Vec<String> = inds.iter().map(|(id, _)| id.clone()).collect();
    let tags = json!({"source_kind": rec.record_type.as_str(), "via": "connector"});
    let mut payload = rec.payload.clone();
    payload["title"] = json!(title);
    let _ = sqlx::query(
        "INSERT INTO intel_items (id, source_id, kind, dedup_key, payload, industries, tags)
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (dedup_key) DO NOTHING",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(&rec.source_id)
    .bind(legacy_kind(rec.record_type))
    .bind(&rec.dedup_key)
    .bind(&payload)
    .bind(serde_json::to_value(&industries).unwrap_or(json!([])))
    .bind(&tags)
    .execute(pool)
    .await;
}

async fn upsert_job(
    pool: &sqlx::PgPool,
    source_id: &str,
    status: &str,
    fetched: Option<i32>,
    new: Option<i32>,
) {
    let _ = sqlx::query(
        "INSERT INTO intel_collection_jobs (source_id, schedule, status, last_run_at, last_error, last_inserted, last_fetched, last_new)
         VALUES ($1, $2, $3, now(), NULL, $4, $5, $6)
         ON CONFLICT (source_id, schedule) DO UPDATE SET
           status = EXCLUDED.status, last_run_at = now(),
           last_inserted = COALESCE(EXCLUDED.last_inserted, intel_collection_jobs.last_inserted),
           last_fetched = COALESCE(EXCLUDED.last_fetched, intel_collection_jobs.last_fetched),
           last_new = COALESCE(EXCLUDED.last_new, intel_collection_jobs.last_new)",
    )
    .bind(source_id)
    .bind("connector")
    .bind(status)
    .bind(new)
    .bind(fetched)
    .bind(new)
    .execute(pool)
    .await;
}

/// 手动触发一轮全部连接器 (替代老的 POST /intel/collect), 完成后刷新机会引擎
pub async fn run_all_connectors(pool: &sqlx::PgPool) -> (usize, usize, Vec<String>) {
    let mut fetched = 0;
    let mut processed = 0;
    let mut failures = Vec::new();
    for conn in connector_registry() {
        let id = conn.descriptor().id.clone();
        match run_connector(pool, &id).await {
            Ok((f, p)) => {
                fetched += f;
                processed += p;
            }
            Err(e) => failures.push(format!("{id}: {e}")),
        }
    }
    let (cf, cp, cerr) = run_custom_feeds(pool).await;
    fetched += cf;
    processed += cp;
    if let Some(e) = cerr {
        failures.push(format!("rss-custom: {e}"));
    }
    let _ = crate::intel::expire_stale_opportunities(pool).await;
    if processed > 0
        && let Ok((n, up)) = crate::intel::refresh_opportunities_db(pool).await
    {
        tracing::info!(
            new_items = processed,
            opportunities = n,
            upserted = up,
            "opportunity engine refreshed"
        );
    }
    (fetched, processed, failures)
}
