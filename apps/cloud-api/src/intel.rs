//! World Intelligence REST surface (WI guide §21): source registry, provider
//! routing/health, ingestion, and opportunity discovery. Collection is
//! metadata/HTTP only — the red line (never bypass platform auth, CAPTCHA,
//! or rate limits) is enforced by using only registered read-only sources.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sincy_intel::model::Trend;
use sincy_intel::opportunity::{opportunity_from_trend, rank_opportunities};
use sincy_intel::registry::{Health, Provider, ProviderRegistry, SourceRegistry, seed_sources};
use sincy_intel::router::{Router as ProviderRouter, RouterRequest};
use sqlx::Row;
use uuid::Uuid;

use crate::api::app_error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/intel/sources", get(list_sources))
        .route(
            "/intel/sources/status",
            get(crate::source_status::source_status),
        )
        .route("/intel/feeds", get(crate::feed_import::list_feeds))
        .route(
            "/intel/feeds/import",
            post(crate::feed_import::import_feed_list),
        )
        .route("/intel/providers", get(list_providers))
        .route("/intel/providers/{id}/health", post(provider_health))
        .route("/intel/route", post(route_query))
        .route("/intel/ingest", post(ingest))
        .route("/intel/items", get(list_items))
        .route(
            "/intel/opportunities",
            get(list_opportunities).post(refresh_opportunities),
        )
        .route("/intel/jobs", get(list_jobs))
        .route("/intel/collect", post(collect_now))
        .route("/intel/stats", get(stats))
        .route("/intel/health", get(health_audit))
        .route("/intel/trends", get(trends))
        .route("/intel/sources/{id}/toggle", post(toggle_source))
        .route("/intel/sources/enable-all", post(enable_all_sources))
        .route("/intel/jobs/resume-all", post(resume_all_jobs))
}

/// POST /intel/sources/{id}/toggle {enabled} — 启用/停用单个信源。
async fn toggle_source(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "body.enabled (bool) required",
            ))
        })?;
    let n = sqlx::query("UPDATE intelligence_sources SET enabled = $1 WHERE id = $2")
        .bind(enabled)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    if n == 0 {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "source not found",
        )));
    }
    Ok(Json(json!({"id": id, "enabled": enabled})))
}

/// POST /intel/sources/enable-all — 一键启用全部信源。
/// 需要外部配置(自托管 RSSHub / SearXNG / provider key)的源启用后仍会
/// 带具体原因显示 degraded/not_configured, 不会静默假装工作。
async fn enable_all_sources(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let n = sqlx::query("UPDATE intelligence_sources SET enabled = true")
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    Ok(Json(json!({"enabled": n})))
}

/// POST /intel/jobs/resume-all — 把所有 paused 任务恢复为 idle。
async fn resume_all_jobs(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let n = sqlx::query("UPDATE intel_collection_jobs SET status = 'idle' WHERE status = 'paused'")
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    Ok(Json(json!({"resumed": n})))
}

/// 由启用标志 + 该源的 job 行算健康标签 (health_audit 与 source_status 共用)
pub fn health_label(enabled: bool, sjobs: &[&sqlx::postgres::PgRow]) -> &'static str {
    if !enabled {
        "disabled"
    } else if sjobs.is_empty() {
        "not_configured"
    } else if sjobs
        .iter()
        .all(|j| j.get::<String, _>("status") == "paused")
    {
        // 已注册 + 有 job 但全部暂停 = 等待配置 (自托管实例/密钥/出口 IP)
        "not_configured"
    } else {
        let ran: Vec<&&sqlx::postgres::PgRow> = sjobs
            .iter()
            .filter(|j| {
                j.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_run_at")
                    .is_some()
            })
            .collect();
        if ran.is_empty() {
            "not_configured"
        } else {
            let any_new: bool = ran
                .iter()
                .any(|j| j.get::<Option<i32>, _>("last_new").unwrap_or(0) > 0);
            let any_failed = ran.iter().any(|j| {
                j.get::<String, _>("status") == "failed"
                    || j.get::<Option<String>, _>("last_error").is_some()
            });
            let all_failed = ran.iter().all(|j| j.get::<String, _>("status") == "failed");
            if all_failed {
                "failed"
            } else if any_new {
                if any_failed { "degraded" } else { "healthy" }
            } else {
                // 跑了但 0 新入库 (全去重) — 数据在流但近期无新内容
                "degraded"
            }
        }
    }
}

/// GET /intel/health — Source Health Audit (WI §28): 每个注册信源的健康状态
/// healthy / degraded / failed / not_configured / disabled,
/// 由 job 运行记录 + 入库量计算, 不只看"是否注册"。
async fn health_audit(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let sources =
        sqlx::query("SELECT id, name, category, enabled FROM intelligence_sources ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(err)?;
    let jobs = sqlx::query(
        "SELECT source_id, schedule, status, last_run_at, last_error, last_fetched, last_new, last_inserted
         FROM intel_collection_jobs ORDER BY source_id, schedule",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let item_counts =
        sqlx::query("SELECT source_id, COUNT(*)::int8 AS n FROM intel_items GROUP BY source_id")
            .fetch_all(pool)
            .await
            .map_err(err)?;

    let mut out: Vec<Value> = Vec::new();
    let mut summary = std::collections::BTreeMap::from([
        ("healthy", 0i64),
        ("degraded", 0i64),
        ("failed", 0i64),
        ("not_configured", 0i64),
        ("disabled", 0i64),
    ]);
    for s in &sources {
        let sid: String = s.get("id");
        let enabled: bool = s.get("enabled");
        let sjobs: Vec<&sqlx::postgres::PgRow> = jobs
            .iter()
            .filter(|j| j.get::<String, _>("source_id") == sid)
            .collect();
        let items_n: i64 = item_counts
            .iter()
            .find(|r| r.get::<String, _>("source_id") == sid)
            .map(|r| r.get::<i64, _>("n"))
            .unwrap_or(0);

        let status = health_label(enabled, &sjobs);
        *summary.entry(status).or_insert(0) += 1;
        out.push(json!({
            "source_id": sid,
            "name": s.get::<String, _>("name"),
            "category": s.get::<String, _>("category"),
            "enabled": enabled,
            "health": status,
            "jobs": sjobs.len(),
            "jobs_ran": sjobs.iter().filter(|j| j.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_run_at").is_some()).count(),
            "items_total": items_n,
            "last_new": sjobs.iter().map(|j| j.get::<Option<i32>, _>("last_new").unwrap_or(0) as i64).max(),
            "last_error": sjobs.iter().find_map(|j| j.get::<Option<String>, _>("last_error")),
        }));
    }
    Ok(Json(json!({
        "generated_at": chrono::Utc::now(),
        "summary": summary,
        "sources": out,
    })))
}

/// GET /intel/trends — Trend Intelligence (WI §11 Trend + §19):
/// 按 topic 聚合最近 24h 情报, 计算 6h 速度 / 相对前 18h 的增长率 / 跨平台分布。
async fn trends(State(state): State<AppState>) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        r#"WITH t AS (
             SELECT coalesce(payload->>'topic', payload->>'title') AS topic,
                    source_id, collected_at, payload->>'url' AS url
             FROM intel_items
             WHERE collected_at > now() - interval '24 hours'
               AND coalesce(payload->>'topic', payload->>'title') IS NOT NULL
           )
           SELECT topic,
                  count(*)::int8 AS total,
                  count(*) FILTER (WHERE collected_at > now() - interval '6 hours')::int8 AS h6,
                  count(*) FILTER (WHERE collected_at <= now() - interval '6 hours')::int8 AS h18,
                  count(DISTINCT source_id)::int8 AS platforms,
                  array_agg(DISTINCT source_id) AS sources,
                  (array_agg(topic || '|' || coalesce(url,'') ORDER BY collected_at DESC))[1:3] AS samples
           FROM t
           WHERE length(topic) > 2
           GROUP BY topic
           HAVING count(*) >= 3
           ORDER BY h6 DESC, total DESC
           LIMIT 30"#,
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let h6 = r.get::<i64, _>("h6");
            let h18 = r.get::<i64, _>("h18");
            // growth: 最近6小时速率 vs 前18小时速率
            let rate_recent = h6 as f64 / 6.0;
            let rate_prev = (h18 as f64 / 18.0).max(0.05);
            let growth_pct = ((rate_recent / rate_prev) - 1.0) * 100.0;
            json!({
                "topic": r.get::<String, _>("topic"),
                "total_24h": r.get::<i64, _>("total"),
                "last_6h": h6,
                "velocity_per_h": (h6 as f64 / 6.0 * 10.0).round() / 10.0,
                "growth_pct": (growth_pct * 10.0).round() / 10.0,
                "platforms": r.get::<i64, _>("platforms"),
                "sources": r.get::<Vec<String>, _>("sources"),
                "samples": r.get::<Vec<String>, _>("samples"),
            })
        })
        .collect();
    Ok(Json(
        json!({ "generated_at": chrono::Utc::now(), "window_hours": 24, "trends": items }),
    ))
}

/// GET /intel/stats — 数据池规模与采集概况 (WI dashboard §28).
/// 视角: owner 角色看全平台 (scope=platform); 普通租户看 共享池规模 + 自己租户的认领/工厂数
/// (scope=tenant)——实体/信号/机会池是全局共享资产, 让租户感知后端数据体量,
/// 同时只能看到自己租户的机会认领数与内容工厂数据。
async fn stats(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let is_owner = auth.roles.iter().any(|r| r == "owner" || r == "admin");
    let scope = if is_owner { "platform" } else { "tenant" };
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intel_items")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let by_kind = sqlx::query(
        "SELECT kind, COUNT(*)::int8 AS n FROM intel_items GROUP BY kind ORDER BY n DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let by_source = sqlx::query(
        "SELECT source_id, COUNT(*)::int8 AS n, MAX(collected_at) AS last_at FROM intel_items GROUP BY source_id ORDER BY n DESC",
    )
    .fetch_all(pool).await.map_err(err)?;
    let opps: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intel_opportunities")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let opps_new: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM intel_opportunities WHERE status = 'new'")
            .fetch_one(pool)
            .await
            .map_err(err)?;
    let entities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intelligence_entities")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    // 可行动机会 = status new 且 7 天内 (cycle 打分只看这个范围)
    let opps_actionable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM intel_opportunities WHERE status = 'new' AND created_at > now() - interval '7 days'",
    )
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let signals_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM intelligence_signals")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(last_inserted), 0)::int8 FROM intel_collection_jobs",
    )
    .fetch_one(pool)
    .await
    .map_err(err)?;
    let last_run: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT MAX(last_run_at) FROM intel_collection_jobs")
            .fetch_one(pool)
            .await
            .map_err(err)?;
    let kinds: Vec<Value> = by_kind
        .iter()
        .map(|r| json!({ "kind": r.get::<String, _>("kind"), "n": r.get::<i64, _>("n") }))
        .collect();
    let sources: Vec<Value> = by_source
        .iter()
        .map(|r| {
            json!({
                "source_id": r.get::<String, _>("source_id"),
                "n": r.get::<i64, _>("n"),
                "last_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_at"),
            })
        })
        .collect();
    // 租户私有补充指标 (仅非 owner): 本租户员工的机会认领数 + 内容工厂 brief 数
    let (claimed, factory_briefs): (i64, i64) = if is_owner {
        (0, 0)
    } else {
        let agency = auth.agency_id.as_uuid();
        let c: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM opportunity_claims c JOIN digital_employees e ON e.id = c.employee_id WHERE e.agency_id = $1",
        )
        .bind(agency)
        .fetch_one(pool).await.map_err(err)?;
        let f: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM factory_briefs WHERE agency_id = $1")
            .bind(agency)
            .fetch_one(pool)
            .await
            .map_err(err)?;
        (c, f)
    };
    Ok(Json(json!({
        "scope": scope,
        "total_items": total,
        "by_kind": kinds,
        "by_source": sources,
        "opportunities": opps,
        "opportunities_new": opps_new,
        "opportunities_actionable": opps_actionable,
        "opportunities_claimed": claimed,
        "factory_briefs": factory_briefs,
        "entities_total": entities,
        "signals_total": signals_total,
        "last_pass_inserted_total": jobs,
        "last_run_at": last_run,
    })))
}

/// GET /intel/jobs — collection job status (WI §9 scheduler view).
async fn list_jobs(State(state): State<AppState>) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT source_id, schedule, status, last_run_at, last_error, last_inserted, last_fetched, last_new FROM intel_collection_jobs ORDER BY source_id, schedule",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "source_id": r.get::<String, _>("source_id"),
                "schedule": r.get::<String, _>("schedule"),
                "status": r.get::<String, _>("status"),
                "last_run_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_run_at"),
                "last_error": r.get::<Option<String>, _>("last_error"),
                "last_inserted": r.get::<Option<i32>, _>("last_inserted").map(|v| v as i64),
                "last_fetched": r.get::<Option<i32>, _>("last_fetched").map(|v| v as i64),
                "last_new": r.get::<Option<i32>, _>("last_new").map(|v| v as i64),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// POST /intel/collect — run one collection pass immediately (WI §9 manual
/// trigger) and refresh the opportunity engine afterwards.
async fn collect_now(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    // 老 intel_collector 定时循环已退役: 手动触发走连接器框架 (唯一抓取方)
    let (fetched, inserted, failures) = crate::pipeline::connectors::run_all_connectors(pool).await;
    let (opps, upserted) = refresh_opportunities_db(pool).await?;
    Ok(Json(json!({
        "fetched": fetched, "inserted": inserted,
        "duplicates": fetched.saturating_sub(inserted), "failures": failures,
        "opportunities": opps, "opportunities_upserted": upserted,
    })))
}

fn err(e: sqlx::Error) -> axum::response::Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

fn health_from(s: &str) -> Health {
    match s {
        "healthy" => Health::Healthy,
        "degraded" => Health::Degraded,
        "down" => Health::Down,
        _ => Health::Unknown,
    }
}

/// GET /intel/sources — platform-level registry (seeded at migration time,
/// plus anything registered at runtime).
async fn list_sources(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT id, name, category, access_method, capabilities, platforms, auth_ref, terms, enabled FROM intelligence_sources ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("name"),
                "category": r.get::<String, _>("category"),
                "access_method": r.get::<String, _>("access_method"),
                "capabilities": r.get::<Value, _>("capabilities"),
                "platforms": r.get::<Value, _>("platforms"),
                "auth_ref": r.get::<Option<String>, _>("auth_ref"),
                "terms": r.get::<Option<String>, _>("terms"),
                "enabled": r.get::<bool, _>("enabled"),
            })
        })
        .collect();
    // fresh env without seed: surface the canonical tool list from the crate
    if items.is_empty() {
        let mut reg = SourceRegistry::new();
        seed_sources(&mut reg);
        items = reg
            .all()
            .iter()
            .map(|s| {
                json!({
                    "id": s.id, "name": s.name, "category": s.category,
                    "access_method": s.access_method, "capabilities": s.capabilities,
                    "platforms": s.platforms, "auth_ref": null, "terms": null, "enabled": s.enabled,
                })
            })
            .collect();
    }
    Ok(Json(json!({ "items": items })))
}

async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT p.id, p.source_id, p.name, p.endpoint, p.cost_model, p.rate_limit, p.health,
                p.reliability, p.enabled, p.last_health_at, s.capabilities, s.platforms
         FROM intel_providers p JOIN intelligence_sources s ON s.id = p.source_id
         ORDER BY p.name",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "source_id": r.get::<String, _>("source_id"),
                "name": r.get::<String, _>("name"),
                "endpoint": r.get::<Option<String>, _>("endpoint"),
                "capabilities": r.get::<Value, _>("capabilities"),
                "platforms": r.get::<Value, _>("platforms"),
                "cost_model": r.get::<Value, _>("cost_model"),
                "rate_limit": r.get::<Value, _>("rate_limit"),
                "health": r.get::<String, _>("health"),
                "reliability": r.get::<rust_decimal::Decimal, _>("reliability"),
                "enabled": r.get::<bool, _>("enabled"),
                "last_health_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_health_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// POST /intel/providers/{id}/health — recompute health with a metadata probe
/// (plain GET against the provider endpoint; never touches platform APIs).
async fn provider_health(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let r = sqlx::query("SELECT name, endpoint FROM intel_providers WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "provider not found",
            ))
        })?;
    let endpoint: Option<String> = r.get("endpoint");
    let (healthy, detail, latency_ms) = match endpoint {
        Some(url) => {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .map_err(|e| {
                    app_error_response(sincy_common::AppError::retryable("http", e.to_string()))
                })?;
            let t0 = std::time::Instant::now();
            match client.get(&url).send().await {
                Ok(resp) => {
                    let ok = resp.status().is_success();
                    (
                        ok,
                        format!("HTTP {}", resp.status().as_u16()),
                        t0.elapsed().as_millis() as i64,
                    )
                }
                Err(e) => (false, e.to_string(), t0.elapsed().as_millis() as i64),
            }
        }
        None => (true, "no endpoint (metadata-only provider)".to_string(), 0),
    };
    let health = if healthy { "healthy" } else { "degraded" };
    sqlx::query("UPDATE intel_providers SET health = $1, last_health_at = now() WHERE id = $2")
        .bind(health)
        .bind(id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(Json(json!({
        "id": id,
        "name": r.get::<String, _>("name"),
        "health": health,
        "latency_ms": latency_ms,
        "detail": detail,
        "checked_at": chrono::Utc::now(),
    })))
}

#[derive(Debug, Deserialize)]
struct RouteBody {
    capability: String,
    platform: Option<String>,
    query_type: Option<String>,
    max_cost_usd: Option<f64>,
}

/// POST /intel/route — score providers and return the ordered fallback chain
/// of provider ids (WI §8).
async fn route_query(
    State(state): State<AppState>,
    Json(body): Json<RouteBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let registry = load_provider_registry(pool).await?;
    let decision = ProviderRouter::new(&registry).route(&RouterRequest {
        capability: body.capability.clone(),
        platform: body.platform.clone(),
        query_type: body.query_type.clone(),
        max_cost_usd: body.max_cost_usd,
    });
    if decision.chain.is_empty() {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "no_provider",
            decision.reason.clone(),
        )));
    }
    // resolve chain ids to names for the console
    let names: Vec<Value> = decision
        .chain
        .iter()
        .map(|id| {
            registry
                .all()
                .iter()
                .find(|p| &p.id == id)
                .map(|p| json!({ "id": p.id, "name": p.name }))
                .unwrap_or_else(|| json!({ "id": id }))
        })
        .collect();
    Ok(Json(
        json!({ "chain": decision.chain, "providers": names, "reason": decision.reason }),
    ))
}

/// Load intel_providers (+ source capabilities/platforms) into the in-memory
/// ProviderRegistry used by the router core.
async fn load_provider_registry(
    pool: &sqlx::PgPool,
) -> Result<ProviderRegistry, axum::response::Response> {
    let rows = sqlx::query(
        "SELECT p.id, p.source_id, p.name, p.cost_model, p.rate_limit, p.health, p.reliability, p.enabled,
                s.capabilities, s.platforms
         FROM intel_providers p JOIN intelligence_sources s ON s.id = p.source_id",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut reg = ProviderRegistry::new();
    for r in &rows {
        let cost_model = r.get::<Value, _>("cost_model");
        let rate_limit = r.get::<Value, _>("rate_limit");
        let caps: Vec<String> = r
            .get::<Value, _>("capabilities")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let platforms: Vec<String> = r
            .get::<Value, _>("platforms")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        reg.register(Provider {
            id: r.get::<Uuid, _>("id").to_string(),
            source_id: r.get("source_id"),
            name: r.get("name"),
            capabilities: caps,
            platforms,
            cost_per_1k: cost_model
                .get("per_1k_usd")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
                * 1000.0,
            rate_budget_left: rate_limit
                .get("window_left")
                .and_then(|v| v.as_i64())
                .unwrap_or(1000),
            rate_budget_max: rate_limit
                .get("window_max")
                .and_then(|v| v.as_i64())
                .unwrap_or(1000),
            health: health_from(&r.get::<String, _>("health")),
            reliability: r
                .get::<rust_decimal::Decimal, _>("reliability")
                .try_into()
                .unwrap_or(0.9),
            enabled: r.get("enabled"),
        });
    }
    Ok(reg)
}

#[derive(Debug, Deserialize)]
struct IngestBody {
    source_id: String,
    provider_id: Option<Uuid>,
    kind: String, // social_content | trend | event | topic | creator | news | knowledge
    dedup_key: String,
    payload: Value,
}

/// POST /intel/ingest — store one normalized intel item (dedup on dedup_key,
/// WI §12). Called by the collection scheduler; also usable manually.
async fn ingest(
    State(state): State<AppState>,
    Json(body): Json<IngestBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM intelligence_sources WHERE id = $1)",
    )
    .bind(&body.source_id)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if !exists {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "unknown_source",
            format!("source {} is not registered", body.source_id),
        )));
    }
    let id = Uuid::new_v4();
    let dup = sqlx::query(
        "INSERT INTO intel_items (id, source_id, provider_id, kind, dedup_key, payload)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (dedup_key) DO NOTHING RETURNING id",
    )
    .bind(id)
    .bind(&body.source_id)
    .bind(body.provider_id)
    .bind(&body.kind)
    .bind(&body.dedup_key)
    .bind(&body.payload)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    match dup {
        Some(r) => Ok(Json(
            json!({ "id": r.get::<Uuid, _>(0), "duplicate": false }),
        )),
        None => Ok(Json(
            json!({ "id": null, "duplicate": true, "dedup_key": body.dedup_key }),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct ItemsQuery {
    kind: Option<String>,
    limit: Option<i64>,
    /// Sincy-04: 按行业过滤 (命中 industries 数组)
    industry: Option<String>,
}

async fn list_items(
    State(state): State<AppState>,
    Query(q): Query<ItemsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = q.limit.unwrap_or(100);
    let rows = match (&q.kind, &q.industry) {
        (Some(k), Some(ind)) => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at, industries, tags FROM intel_items WHERE kind = $1 AND industries @> $2::jsonb ORDER BY collected_at DESC LIMIT $3",
            )
            .bind(k)
            .bind(json!([ind]))
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(err)?
        }
        (Some(k), None) => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at, industries, tags FROM intel_items WHERE kind = $1 ORDER BY collected_at DESC LIMIT $2",
            )
            .bind(k)
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(err)?
        }
        (None, Some(ind)) => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at, industries, tags FROM intel_items WHERE industries @> $1::jsonb ORDER BY collected_at DESC LIMIT $2",
            )
            .bind(json!([ind]))
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(err)?
        }
        (None, None) => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at, industries, tags FROM intel_items ORDER BY collected_at DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(pool)
            .await
            .map_err(err)?
        }
    };
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "source_id": r.get::<String, _>("source_id"),
                "provider_id": r.get::<Option<Uuid>, _>("provider_id"),
                "kind": r.get::<String, _>("kind"),
                "dedup_key": r.get::<String, _>("dedup_key"),
                "payload": r.get::<Value, _>("payload"),
                "collected_at": r.get::<chrono::DateTime<chrono::Utc>, _>("collected_at"),
                "industries": r.get::<Value, _>("industries"),
                "tags": r.get::<Value, _>("tags"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_opportunities(
    State(state): State<AppState>,
    Query(q): Query<ItemsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT id, topic, score, rationale, evidence, status, created_at FROM intel_opportunities ORDER BY score DESC LIMIT $1",
    )
    .bind(q.limit.unwrap_or(50))
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "topic": r.get::<String, _>("topic"),
                "score": r.get::<rust_decimal::Decimal, _>("score"),
                "rationale": r.get::<Value, _>("rationale"),
                "evidence": r.get::<Value, _>("evidence"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// POST /intel/opportunities — run the opportunity engine (WI §19).
async fn refresh_opportunities(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let (n, up) = refresh_opportunities_db(pool).await?;
    // 断点①: 机会命中阈值(0.45)自动建工序0 Brief, 绑定调用方 agency + 其品牌
    let auto =
        crate::factory::auto_briefs_from_opportunities(pool, *auth.agency_id.as_uuid(), 0.45).await;
    tracing::info!(auto_briefs = auto, "opportunity-driven briefs auto-created");
    Ok(Json(
        json!({ "opportunities": n, "upserted": up, "auto_briefs": auto }),
    ))
}

/// Shared opportunity-engine pass (REST route + collector after new items
/// land). Platform-level: pillar match defaults to 0.5 (no employee context);
/// employee-specific re-scoring happens in the DE adapter (WI §20).
/// Returns (opportunities scored, upserted rows).
/// 过期清理: 超 7 天仍为 new 的机会标 dismissed (intel_opportunities 按 topic UNIQUE 无清理的已知问题)
pub async fn expire_stale_opportunities(
    pool: &sqlx::PgPool,
) -> Result<u64, axum::response::Response> {
    let mut n = sqlx::query("UPDATE intel_opportunities SET status = 'dismissed' WHERE status = 'new' AND created_at < now() - interval '7 days'")
        .execute(pool)
        .await
        .map_err(|e| crate::api::app_error_response(sincy_common::AppError::retryable("db", e.to_string())))?
        .rows_affected();
    // 机会池上限: 动态化 (0048)—— cap = GREATEST(500, 50 * 全平台活跃员工数),
    // 活跃员工越多池子越大, 避免多员工把共享 new 池抢光; platform_active_employee_count()
    // 是 SECURITY DEFINER, 后台无租户上下文时也能统计 (digital_employees 有 RLS)。
    n += sqlx::query(
        "UPDATE intel_opportunities SET status = 'dismissed' WHERE status = 'new' AND topic NOT IN (SELECT topic FROM intel_opportunities WHERE status = 'new' ORDER BY score DESC, created_at DESC LIMIT (SELECT GREATEST(500, 50 * platform_active_employee_count())))",
    )
        .execute(pool)
        .await
        .map_err(|e| crate::api::app_error_response(sincy_common::AppError::retryable("db", e.to_string())))?
        .rows_affected();
    Ok(n)
}
pub(crate) async fn refresh_opportunities_db(
    pool: &sqlx::PgPool,
) -> Result<(usize, usize), axum::response::Response> {
    // 机会输入: trend + news + knowledge + social_content (72h 内), 显式排除
    // event 类 (如 Wikipedia 最近修改) —— 高频低价值噪声不参与选题。
    let rows = sqlx::query(
        "SELECT dedup_key, source_id, kind, payload, collected_at FROM intel_items \
         WHERE kind IN ('trend','news','knowledge','social_content') \
           AND collected_at > now() - interval '72 hours' \
         ORDER BY collected_at DESC LIMIT 400",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let now = chrono::Utc::now();
    let mut opps = Vec::new();
    for r in &rows {
        let dedup_key: String = r.get("dedup_key");
        let source_id: String = r.get("source_id");
        let kind: String = r.get("kind");
        let payload: Value = r.get("payload");
        let collected_at: chrono::DateTime<chrono::Utc> = r.get("collected_at");
        let freshness_hours = (now - collected_at).num_minutes() as f64 / 60.0;
        let title = payload
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let url = payload
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let trend = if kind == "trend" {
            match serde_json::from_value::<Trend>(payload.clone()) {
                Ok(t) => t,
                Err(_) => continue,
            }
        } else {
            // 编辑类内容 (news/knowledge/social_content): 用标题作 topic,
            // 给一个中性的基线量/速度, 由 freshness 和品牌相关性主导排序
            if title.is_empty() {
                continue;
            }
            Trend {
                topic: title.clone(),
                platforms: vec![source_id.clone()],
                volume: 600,
                velocity: 0.8,
                sentiment: 0.5,
                window_hours: 24,
            }
        };
        let mut opp = opportunity_from_trend(
            &trend,
            0.5,
            0.2,
            freshness_hours.max(0.0),
            vec![dedup_key.clone()],
        );
        opp.rationale["title"] = json!(title);
        opp.rationale["url"] = json!(url);
        opp.rationale["source"] = json!(source_id);
        opp.rationale["kind"] = json!(kind);
        opps.push(opp);
    }
    let ranked = rank_opportunities(opps);
    let mut upserted = 0usize;
    for o in &ranked {
        let res = sqlx::query(
            "INSERT INTO intel_opportunities (topic, score, rationale, evidence, status)
             VALUES ($1, $2, $3, $4, 'new')
             ON CONFLICT (topic) DO UPDATE SET
               score = EXCLUDED.score, rationale = EXCLUDED.rationale, evidence = EXCLUDED.evidence",
        )
        .bind(&o.topic)
        .bind(rust_decimal::Decimal::try_from(o.score).unwrap_or(rust_decimal::Decimal::ZERO))
        .bind(&o.rationale)
        .bind(json!(o.evidence))
        .execute(pool)
        .await
        .map_err(err)?;
        if res.rows_affected() > 0 {
            upserted += 1;
        }
    }
    Ok((ranked.len(), upserted))
}

#[doc(hidden)]
pub async fn refresh_opportunities_db_for_test(pool: &sqlx::PgPool) -> (usize, usize) {
    refresh_opportunities_db(pool).await.unwrap()
}
