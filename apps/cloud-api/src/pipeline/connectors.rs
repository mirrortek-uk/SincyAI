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
        let items = match self.manifest.descriptor.connector_type.as_str() {
            "rss" => crate::intel_collector::parse_rss(&body)
                .into_iter()
                .map(|(title, url, summary, published)| {
                    (
                        title.clone(),
                        url.clone(),
                        json!({"title": title, "url": url, "summary": summary, "published_at": published, "source": src.label}),
                    )
                })
                .collect::<Vec<_>>(),
            "aggregator" => vec![], // pulse provider 现役采集, 这里不重复抓
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
    let batch = conn
        .fetch(&FetchParams::default(), None)
        .await
        .map_err(|e| e.to_string())?;
    let fetched = batch.records.len();
    let mut processed = 0;
    for rec in &batch.records {
        match super::ingest_record(pool, rec).await {
            Ok(true) => processed += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(dedup = %rec.dedup_key, error = %e, "pipeline ingest failed"),
        }
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

/// 调度器: 周期跑所有连接器 (与 intel_collector 双写, dedup_key 保证幂等)
pub async fn run_scheduler(pool: sqlx::PgPool) {
    let mut tick = tokio::time::interval(Duration::from_secs(1800));
    loop {
        tick.tick().await;
        for conn in connector_registry() {
            let id = conn.descriptor().id.clone();
            match run_connector(&pool, &id).await {
                Ok((f, p)) => {
                    tracing::info!(connector = %id, fetched = f, processed = p, "connector run done")
                }
                Err(e) => tracing::warn!(connector = %id, error = %e, "connector run failed"),
            }
        }
    }
}
