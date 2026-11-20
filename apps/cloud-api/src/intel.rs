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
}

/// GET /intel/jobs — collection job status (WI §9 scheduler view).
async fn list_jobs(State(state): State<AppState>) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT source_id, schedule, status, last_run_at, last_error FROM intel_collection_jobs ORDER BY source_id, schedule",
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
    let cfg = crate::intel_collector::CollectorConfig::from_env();
    let (inserted, dups, failures) = crate::intel_collector::collect_once(pool, &cfg).await;
    let (opps, upserted) = refresh_opportunities_db(pool).await?;
    Ok(Json(json!({
        "inserted": inserted, "duplicates": dups, "failures": failures,
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
}

async fn list_items(
    State(state): State<AppState>,
    Query(q): Query<ItemsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = match &q.kind {
        Some(k) => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at FROM intel_items WHERE kind = $1 ORDER BY collected_at DESC LIMIT $2",
            )
            .bind(k)
            .bind(q.limit.unwrap_or(100))
            .fetch_all(pool)
            .await
            .map_err(err)?
        }
        None => {
            sqlx::query(
                "SELECT id, source_id, provider_id, kind, dedup_key, payload, collected_at FROM intel_items ORDER BY collected_at DESC LIMIT $1",
            )
            .bind(q.limit.unwrap_or(100))
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
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let (n, up) = refresh_opportunities_db(pool).await?;
    Ok(Json(json!({ "opportunities": n, "upserted": up })))
}

/// Shared opportunity-engine pass (REST route + collector after new items
/// land). Platform-level: pillar match defaults to 0.5 (no employee context);
/// employee-specific re-scoring happens in the DE adapter (WI §20).
/// Returns (opportunities scored, upserted rows).
pub(crate) async fn refresh_opportunities_db(
    pool: &sqlx::PgPool,
) -> Result<(usize, usize), axum::response::Response> {
    let rows = sqlx::query(
        "SELECT dedup_key, payload, collected_at FROM intel_items WHERE kind = 'trend' ORDER BY collected_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let now = chrono::Utc::now();
    let mut opps = Vec::new();
    for r in &rows {
        let dedup_key: String = r.get("dedup_key");
        let collected_at: chrono::DateTime<chrono::Utc> = r.get("collected_at");
        if let Ok(trend) = serde_json::from_value::<Trend>(r.get::<Value, _>("payload")) {
            let freshness_hours = (now - collected_at).num_minutes() as f64 / 60.0;
            opps.push(opportunity_from_trend(
                &trend,
                0.5,
                0.2,
                freshness_hours.max(0.0),
                vec![dedup_key],
            ));
        }
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
