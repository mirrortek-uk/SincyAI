//! 信源接入全景 (A/B/C 分类), 供控制台"世界情报"页展示每个源的工作状态:
//!
//! - A 类: 挂在真跑着的 HttpConnector 上 (非 aggregator 的 manifest 里声明的源)
//! - B 类: aggregator 源, 走 Sincy Pulse provider (不走 HttpConnector)
//! - C 类: 已注册但没接通抓取路径 → not_configured, 列出缺什么配置

use axum::{Json, extract::State};
use serde_json::{Value, json};
use sqlx::Row;

use sincy_intel::pipeline::{ConnectorManifest, default_connector_manifests};

use crate::pipeline::connectors::parse_path_for;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceClass {
    /// 绑在 HttpConnector 上
    A,
    /// aggregator / pulse 聚合源
    B,
    /// 声明了但没接通抓取路径
    C,
}

impl SourceClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceClass::A => "A",
            SourceClass::B => "B",
            SourceClass::C => "C",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClassifiedSource {
    pub source_id: String,
    pub name: String,
    pub category: String,
    pub class: SourceClass,
    pub connector: Option<String>,
    pub connector_type: Option<String>,
    pub parse_path: Option<String>,
    pub missing: Option<String>,
}

/// C 类源缺什么配置 (按 source id 的静态说明; 不认 connector 好坏)
fn missing_config_for(source_id: &str) -> &'static str {
    match source_id {
        "searxng" => "自托管 SearXNG base URL (SINCY_INTEL_SEARXNG_BASE, searxng-ext 连接器等它)",
        "crawl4ai" => "自托管 Crawl4AI 服务地址",
        "firecrawl" => "SINCY_FIRECRAWL_API_KEY (website 连接器用)",
        "tikhub" => "TikHub API key (vault)",
        "socialdatax" => "SocialDataX key (vault://socialdatax-api-key)",
        "reach-mcp" => "reach-mcp 接入配置",
        "trendradar" => "TrendRadar 未接入",
        "google-trends" => "无开放 API: 要 RSSHub 路由或 provider",
        "baidu-hot" | "douyin-hot" | "weibo-hot" | "zhihu-hot" => "需自托管 RSSHub 路由",
        "reddit" => "Reddit API key 或 RSSHub 路由",
        "x-social" => "X 数据 provider key (vault) 或聚合路径未开",
        "gdelt" => "需专用出口 IP (共享 IP 被 429)",
        _ => "未接入任何 connector (加一行 intel_custom_feeds 或一个新连接器即可)",
    }
}

/// 纯函数: 由 connector manifest + 已注册信源列表算出 A/B/C 分类 (可单测)
pub fn classify_sources(
    manifests: &[ConnectorManifest],
    registered: &[(String, String, String)], // (id, name, category)
) -> Vec<ClassifiedSource> {
    use std::collections::HashMap;
    let mut by_source: HashMap<String, (String, String, bool)> = HashMap::new(); // sid -> (conn_id, conn_type, enabled)
    for m in manifests {
        for src in &m.sources {
            by_source.insert(
                src.source_id.clone(),
                (
                    m.descriptor.id.clone(),
                    m.descriptor.connector_type.clone(),
                    src.enabled,
                ),
            );
        }
    }
    let mut out = Vec::new();
    for (id, name, category) in registered {
        if category == "aggregate" {
            out.push(ClassifiedSource {
                source_id: id.clone(),
                name: name.clone(),
                category: category.clone(),
                class: SourceClass::B,
                connector: by_source.get(id).map(|(c, _, _)| c.clone()),
                connector_type: by_source.get(id).map(|(_, t, _)| t.clone()),
                parse_path: by_source
                    .get(id)
                    .map(|(c, t, _)| parse_path_for(c, t).to_string()),
                missing: None,
            });
            continue;
        }
        if let Some((cid, ctype, _enabled)) = by_source.get(id) {
            let class = if ctype == "aggregator" || category == "aggregate" {
                SourceClass::B
            } else {
                SourceClass::A
            };
            out.push(ClassifiedSource {
                source_id: id.clone(),
                name: name.clone(),
                category: category.clone(),
                class,
                connector: Some(cid.clone()),
                connector_type: Some(ctype.clone()),
                parse_path: Some(parse_path_for(cid, ctype).to_string()),
                missing: None,
            });
        } else {
            out.push(ClassifiedSource {
                source_id: id.clone(),
                name: name.clone(),
                category: category.clone(),
                class: SourceClass::C,
                connector: None,
                connector_type: None,
                parse_path: None,
                missing: Some(missing_config_for(id).to_string()),
            });
        }
    }
    out.sort_by(|a, b| (a.class.as_str(), &a.source_id).cmp(&(b.class.as_str(), &b.source_id)));
    out
}

fn err(e: impl ToString) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::retryable("bad", e.to_string()))
}

/// GET /intel/sources/status — A/B/C 分类 + 健康 + 累计入库 + 挂在哪个 connector
pub async fn source_status(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let sources =
        sqlx::query("SELECT id, name, category, enabled FROM intelligence_sources ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(err)?;
    let jobs = sqlx::query(
        "SELECT source_id, status, last_run_at, last_error, last_fetched, last_new FROM intel_collection_jobs",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let item_counts =
        sqlx::query("SELECT source_id, COUNT(*)::int8 AS n FROM intel_items GROUP BY source_id")
            .fetch_all(pool)
            .await
            .map_err(err)?;

    let registered: Vec<(String, String, String)> = sources
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("id"),
                r.get::<String, _>("name"),
                r.get::<String, _>("category"),
            )
        })
        .collect();
    // 自定义 RSS 源 (intel_custom_feeds 表) 由运行时 rss-custom 连接器抓取, 一并纳入 A 类
    let mut manifests = default_connector_manifests();
    if let Some(cm) = crate::pipeline::connectors::custom_rss_manifest(pool).await {
        manifests.push(cm);
    }
    let classified = classify_sources(&manifests, &registered);

    let mut out: Vec<Value> = Vec::new();
    let mut summary = json!({"A": 0, "B": 0, "C": 0});
    for c in &classified {
        let sid = &c.source_id;
        let enabled = sources
            .iter()
            .find(|r| r.get::<String, _>("id") == *sid)
            .map(|r| r.get::<bool, _>("enabled"))
            .unwrap_or(false);
        let sjobs: Vec<&sqlx::postgres::PgRow> = jobs
            .iter()
            .filter(|j| j.get::<String, _>("source_id") == *sid)
            .collect();
        let items_total: i64 = item_counts
            .iter()
            .find(|r| r.get::<String, _>("source_id") == *sid)
            .map(|r| r.get::<i64, _>("n"))
            .unwrap_or(0);
        let health = crate::intel::health_label(enabled, &sjobs);
        summary[c.class.as_str()] = json!(summary[c.class.as_str()].as_i64().unwrap_or(0) + 1);
        out.push(json!({
            "source_id": sid,
            "name": c.name,
            "category": c.category,
            "class": c.class.as_str(),
            "health": health,
            "items_total": items_total,
            "connector": c.connector,
            "connector_type": c.connector_type,
            "parse_path": c.parse_path,
            "missing": c.missing,
        }));
    }
    Ok(Json(json!({ "summary": summary, "sources": out })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sincy_intel::pipeline::{ConnectorManifest, RecordType, SourceRef};

    fn mani(id: &str, ctype: &str, srcs: &[&str]) -> ConnectorManifest {
        ConnectorManifest {
            descriptor: sincy_intel::pipeline::ConnectorDescriptor {
                id: id.into(),
                name: id.into(),
                connector_type: ctype.into(),
                capabilities: vec![],
                datasets: vec![],
                default_interval_secs: 1800,
                rate_limit_per_min: 30,
                auth: Default::default(),
            },
            sources: srcs
                .iter()
                .map(|s| SourceRef {
                    source_id: s.to_string(),
                    label: s.to_string(),
                    url: format!("https://{s}/feed"),
                    kind: "news".into(),
                    record_type: RecordType::Article,
                    enabled: true,
                })
                .collect(),
            fetch_impl: "http:rss".into(),
        }
    }

    #[test]
    fn classifies_abc() {
        let manifests = vec![
            mani("rss-news", "rss", &["juejin", "sspai"]),
            mani("hn-algolia", "api", &["hn-algolia"]),
            mani("pulse-x-ecology", "aggregator", &["xbangdan"]),
        ];
        let reg = vec![
            ("juejin".to_string(), "掘金".to_string(), "rss".to_string()),
            (
                "hn-algolia".to_string(),
                "HN".to_string(),
                "social".to_string(),
            ),
            (
                "xbangdan".to_string(),
                "xbangdan".to_string(),
                "rank".to_string(),
            ),
            (
                "sincy-pulse".to_string(),
                "Sincy Pulse".to_string(),
                "aggregate".to_string(),
            ),
            (
                "searxng".to_string(),
                "SearXNG".to_string(),
                "search".to_string(),
            ),
        ];
        let out = classify_sources(&manifests, &reg);
        let get = |id: &str| out.iter().find(|c| c.source_id == id).unwrap();
        assert_eq!(get("juejin").class, SourceClass::A);
        assert_eq!(get("juejin").connector.as_deref(), Some("rss-news"));
        assert_eq!(get("juejin").parse_path.as_deref(), Some("parse_rss"));
        assert_eq!(get("hn-algolia").parse_path.as_deref(), Some("parse_hn"));
        assert_eq!(get("xbangdan").class, SourceClass::B);
        assert_eq!(get("sincy-pulse").class, SourceClass::B); // category=aggregate 即使不在 manifest
        let c = get("searxng");
        assert_eq!(c.class, SourceClass::C);
        assert!(c.missing.as_ref().unwrap().contains("SearXNG"));
        // 排序: A 在前, 其次 B, 最后 C
        let classes: Vec<&str> = out.iter().map(|c| c.class.as_str()).collect();
        assert_eq!(classes, vec!["A", "A", "B", "B", "C"]);
    }
}
