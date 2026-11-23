//! Sincy Pulse — Extensible Intelligence Provider Plugin (Phase 1+2 落地)。
//!
//! Provider manifest 注册 → 调度器按 dataset.update_frequency 触发 →
//! adapter 归一化成 IntelligenceEntity(带 observations) → 引擎跨源聚合
//! 写 pulse_entities → 控制台 /pulse 展示产品层 (用户只看到 Sincy Pulse)。
//!
//! 红线: legal_status 非 public/commercial_ok 的 provider 不调度;
//! 采集只走公开只读接口 (builtin feed 复用 intel_collector 的只读抓取)。

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use sincy_intel::pulse::{
    BuiltinFeedProvider, IntelligenceProvider, ProviderManifest, aggregate, classify_trend,
};

use crate::api::app_error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/pulse/providers",
            get(list_providers).post(register_provider),
        )
        .route(
            "/pulse/providers/{id}",
            get(get_provider).put(update_provider),
        )
        .route("/pulse/providers/{id}/toggle", post(toggle_provider))
        .route("/pulse/providers/{id}/run", post(run_provider))
        .route("/pulse/entities", get(list_entities))
        .route("/pulse/entities/{id}/observations", get(list_observations))
        .route("/pulse/health", get(health))
}

fn err(e: sqlx::Error) -> axum::response::Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

// ---------- provider 注册 / 管理 ----------

fn provider_json(r: &sqlx::postgres::PgRow) -> Value {
    let manifest: Value = r.get("manifest");
    json!({
        "id": r.get::<String, _>("id"),
        "name": r.get::<String, _>("name"),
        "platform": r.get::<String, _>("platform"),
        "category": r.get::<String, _>("category"),
        "legal_status": r.get::<String, _>("legal_status"),
        "reliability": r.get::<rust_decimal::Decimal, _>("reliability"),
        "enabled": r.get::<bool, _>("enabled"),
        "schedulable": matches!(r.get::<String, _>("legal_status").as_str(), "public" | "commercial_ok"),
        "manifest": manifest,
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    })
}

async fn list_providers(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query("SELECT * FROM pulse_providers ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(err)?;
    Ok(Json(
        json!({ "items": rows.iter().map(provider_json).collect::<Vec<_>>() }),
    ))
}

async fn get_provider(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<String>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let r = sqlx::query("SELECT * FROM pulse_providers WHERE id = $1")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "provider not found",
            ))
        })?;
    let mut v = provider_json(&r);
    let runs = sqlx::query(
        "SELECT dataset_id, status, items_fetched, items_new, error, started_at, finished_at \
         FROM pulse_provider_runs WHERE provider_id = $1 ORDER BY started_at DESC LIMIT 20",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    v["recent_runs"] = json!(
        runs.iter()
            .map(|r| json!({
                "dataset_id": r.get::<String, _>("dataset_id"),
                "status": r.get::<String, _>("status"),
                "items_fetched": r.get::<i32, _>("items_fetched"),
                "items_new": r.get::<i32, _>("items_new"),
                "error": r.get::<Option<String>, _>("error"),
                "started_at": r.get::<chrono::DateTime<chrono::Utc>, _>("started_at"),
                "finished_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finished_at"),
            }))
            .collect::<Vec<_>>()
    );
    Ok(Json(v))
}

/// POST /pulse/providers — 注册 manifest (Admin Console "Add Intelligence Provider" 的后端)。
async fn register_provider(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let manifest = ProviderManifest::parse(&body).map_err(|e| {
        app_error_response(sincy_common::AppError::non_retryable("bad_manifest", &e))
    })?;
    let legal = manifest.legal_status.as_str();
    // 红线: unknown/blocked 默认不启用
    let enabled = legal == "public" || legal == "commercial_ok";
    sqlx::query(
        "INSERT INTO pulse_providers (id, name, platform, category, manifest, legal_status, reliability, enabled)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, platform = EXCLUDED.platform,
           category = EXCLUDED.category, manifest = EXCLUDED.manifest, legal_status = EXCLUDED.legal_status,
           reliability = EXCLUDED.reliability, updated_at = now()",
    )
    .bind(&manifest.id)
    .bind(&manifest.name)
    .bind(&manifest.platform)
    .bind(&manifest.category)
    .bind(serde_json::to_value(&manifest).map_err(|e| app_error_response(sincy_common::AppError::non_retryable("serde", e.to_string())))?)
    .bind(legal)
    .bind(rust_decimal::Decimal::try_from(manifest.reliability).unwrap_or(rust_decimal::Decimal::new(5, 1)))
    .bind(enabled)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(Json(
        json!({ "id": manifest.id, "enabled": enabled, "legal_status": legal }),
    ))
}

/// PUT /pulse/providers/{id} — 修改 (启用/法务状态/可靠性/manifest)。
async fn update_provider(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let cur = sqlx::query("SELECT * FROM pulse_providers WHERE id = $1")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "provider not found",
            ))
        })?;
    let mut manifest: Value = cur.get("manifest");
    if let Some(m) = body.get("manifest") {
        manifest = m.clone();
    }
    let manifest = ProviderManifest::parse(&manifest).map_err(|e| {
        app_error_response(sincy_common::AppError::non_retryable("bad_manifest", &e))
    })?;
    let legal = body
        .get("legal_status")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| manifest.legal_status.as_str());
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reliability = body
        .get("reliability")
        .and_then(|v| v.as_f64())
        .unwrap_or(manifest.reliability);
    sqlx::query(
        "UPDATE pulse_providers SET manifest = $1, legal_status = $2, enabled = $3, reliability = $4, updated_at = now() WHERE id = $5",
    )
    .bind(serde_json::to_value(&manifest).map_err(|e| app_error_response(sincy_common::AppError::non_retryable("serde", e.to_string())))?)
    .bind(legal)
    .bind(enabled)
    .bind(rust_decimal::Decimal::try_from(reliability).unwrap_or(rust_decimal::Decimal::new(5, 1)))
    .bind(&id)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(Json(
        json!({ "id": id, "enabled": enabled, "legal_status": legal }),
    ))
}

async fn toggle_provider(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "enabled required",
            ))
        })?;
    // 红线: 启用前必须是合规状态
    let legal: String =
        sqlx::query_scalar("SELECT legal_status FROM pulse_providers WHERE id = $1")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .map_err(err)?
            .ok_or_else(|| {
                app_error_response(sincy_common::AppError::non_retryable(
                    "not_found",
                    "provider not found",
                ))
            })?;
    if enabled && !matches!(legal.as_str(), "public" | "commercial_ok") {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "legal_blocked",
            "provider legal_status 不是 public/commercial_ok, 不允许启用 (红线: 不默认获得第三方数据再分发权)",
        )));
    }
    sqlx::query("UPDATE pulse_providers SET enabled = $1, updated_at = now() WHERE id = $2")
        .bind(enabled)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(Json(json!({ "id": id, "enabled": enabled })))
}

// ---------- 采集执行 ----------

/// 构造一个 manifest 驱动的 provider 实例 (builtin_feed 复用 collector 的 feed)。
async fn build_provider(
    manifest: &ProviderManifest,
) -> Result<Box<dyn IntelligenceProvider>, String> {
    if manifest.acquisition.acq_type != sincy_intel::pulse::AcquisitionType::BuiltinFeed {
        return Err(format!(
            "acquisition type {:?} 需要自定义 adapter (Phase 2); 当前内置只支持 builtin_feed",
            manifest.acquisition.acq_type.as_str()
        ));
    }
    let cfg = crate::intel_collector::CollectorConfig::from_env();
    let mut feeds = Vec::new();
    for ds in &manifest.datasets {
        let label = ds
            .feed_label
            .clone()
            .ok_or_else(|| format!("dataset {} 缺 feed_label", ds.id))?;
        let fc = cfg
            .feeds
            .iter()
            .find(|f| f.label == label)
            .ok_or_else(|| format!("feed label '{label}' not found in collector config"))?;
        feeds.push((ds.id.clone(), fc.source_id.clone(), fc.label.clone()));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent("SincyIntelBot/1.0 (+read-only public feeds; contact: admin@sincy.dev)")
        .build()
        .map_err(|e| e.to_string())?;
    let fetcher = {
        let cfg = crate::intel_collector::CollectorConfig::from_env();
        Arc::new(move |source_id: &str, label: &str| {
            let cfg = cfg.clone();
            let client = client.clone();
            let source_id = source_id.to_string();
            let label = label.to_string();
            Box::pin(async move {
                let f = cfg
                    .feeds
                    .iter()
                    .find(|f| f.source_id == source_id && f.label == label)
                    .ok_or_else(|| format!("feed {source_id}/{label} not found"))?
                    .clone();
                crate::intel_collector::fetch_feed_items(&client, &f).await
            })
                as std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = Result<Vec<(String, String, Value)>, String>,
                            > + Send,
                    >,
                >
        }) as sincy_intel::pulse::FeedFetcher
    };
    Ok(Box::new(BuiltinFeedProvider::new(
        manifest.clone(),
        feeds,
        fetcher,
    )))
}

/// 执行一次 provider 全部 dataset: 拉取 → 归一化 → 聚合入 pulse_entities + observations。
pub async fn run_provider_once(pool: &sqlx::PgPool, provider_id: &str) -> Result<Value, String> {
    let row = sqlx::query("SELECT * FROM pulse_providers WHERE id = $1 AND enabled")
        .bind(provider_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("provider {provider_id} not found or disabled"))?;
    let manifest_v: Value = row.get("manifest");
    let manifest = ProviderManifest::parse(&manifest_v)?;
    let legal: String = row.get("legal_status");
    if !matches!(legal.as_str(), "public" | "commercial_ok") {
        return Err(format!(
            "provider {provider_id} legal_status={legal}, 红线不允许调度"
        ));
    }
    let provider = build_provider(&manifest).await?;
    let reliability: f64 = row
        .get::<rust_decimal::Decimal, _>("reliability")
        .try_into()
        .unwrap_or(0.5);
    let mut total_fetched = 0usize;
    let mut total_new = 0usize;
    let mut errors = Vec::new();
    for ds in &manifest.datasets {
        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO pulse_provider_runs (id, provider_id, dataset_id, status) VALUES ($1, $2, $3, 'running')",
        )
        .bind(run_id)
        .bind(provider_id)
        .bind(&ds.id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        match provider.fetch_dataset(&ds.id).await {
            Ok(entities) => {
                let fetched = entities.len();
                let mut new = 0usize;
                for e in entities {
                    if upsert_entity(pool, provider_id, &ds.id, &e, reliability).await? {
                        new += 1;
                    }
                }
                total_fetched += fetched;
                total_new += new;
                sqlx::query(
                    "UPDATE pulse_provider_runs SET status='ok', items_fetched=$1, items_new=$2, finished_at=now() WHERE id=$3",
                )
                .bind(fetched as i32)
                .bind(new as i32)
                .bind(run_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
            Err(e) => {
                errors.push(format!("{}: {e}", ds.id));
                sqlx::query(
                    "UPDATE pulse_provider_runs SET status='failed', error=$1, finished_at=now() WHERE id=$2",
                )
                .bind(&e)
                .bind(run_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(json!({
        "provider_id": provider_id,
        "fetched": total_fetched,
        "new": total_new,
        "errors": errors,
    }))
}

/// upsert 一个实体 + 其 observations, 返回是否新建实体。
async fn upsert_entity(
    pool: &sqlx::PgPool,
    provider_id: &str,
    dataset_id: &str,
    e: &sincy_intel::pulse::IntelligenceEntity,
    reliability: f64,
) -> Result<bool, String> {
    // 实体消解: 同 (entity_type, canonical_key) 合并已有 observations 再聚合
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM pulse_entities WHERE entity_type = $1 AND canonical_key = $2",
    )
    .bind(&e.entity_type)
    .bind(&e.canonical_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    // 取已有 observations 数值 + 新 observations 一起聚合
    let mut all_obs: Vec<sincy_intel::pulse::Observation> = e.observations.clone();
    let mut rel_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    rel_map.insert(provider_id.to_string(), reliability);
    let entity_id = match existing {
        Some(id) => {
            let rows = sqlx::query(
                "SELECT provider_id, metric, value FROM pulse_observations WHERE entity_id = $1",
            )
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
            for r in rows {
                let pid: String = r.get("provider_id");
                rel_map.entry(pid.clone()).or_insert_with(|| {
                    // 历史 provider 可能已删, 给默认可靠性
                    0.5
                });
                all_obs.push(sincy_intel::pulse::Observation {
                    provider_id: pid,
                    dataset_id: dataset_id.to_string(),
                    metric: r.get("metric"),
                    value: r.get("value"),
                });
            }
            id
        }
        None => Uuid::new_v4(),
    };
    let agg = aggregate(&e.entity_type, &all_obs, &|p| {
        rel_map.get(p).copied().unwrap_or(0.5)
    });
    // 趋势: 用 views/velocity 类指标估算 (轻量 Phase-3)
    let velocity = agg
        .metrics
        .get("views")
        .or_else(|| agg.metrics.get("stars"))
        .and_then(|m| m["estimated"].as_f64())
        .map(|v| v / 24.0)
        .unwrap_or(0.0);
    let trend = json!({"velocity": velocity, "acceleration": 0.0, "classification": classify_trend(velocity)});

    let is_new = existing.is_none();
    sqlx::query(
        "INSERT INTO pulse_entities (id, entity_type, platform, canonical_key, title, payload, metrics, trend, confidence)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (entity_type, canonical_key) DO UPDATE SET
           title = EXCLUDED.title, payload = EXCLUDED.payload, metrics = EXCLUDED.metrics,
           trend = EXCLUDED.trend, confidence = EXCLUDED.confidence, updated_at = now()",
    )
    .bind(entity_id)
    .bind(&e.entity_type)
    .bind(&e.platform)
    .bind(&e.canonical_key)
    .bind(&e.title)
    .bind(&e.payload)
    .bind(serde_json::to_value(&agg.metrics).map_err(|e| e.to_string())?)
    .bind(&trend)
    .bind(&agg.confidence)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    for o in &e.observations {
        sqlx::query(
            "INSERT INTO pulse_observations (id, entity_id, provider_id, dataset_id, metric, value)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(entity_id)
        .bind(&o.provider_id)
        .bind(&o.dataset_id)
        .bind(&o.metric)
        .bind(&o.value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(is_new)
}

async fn run_provider(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<String>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let out = run_provider_once(pool, &id)
        .await
        .map_err(|e| app_error_response(sincy_common::AppError::retryable("pulse_run", &e)))?;
    Ok(Json(out))
}

// ---------- 产品层读取 ----------

#[derive(Debug, serde::Deserialize)]
struct EntitiesQuery {
    #[serde(rename = "type")]
    entity_type: Option<String>,
    q: Option<String>,
    classification: Option<String>,
    limit: Option<i64>,
}

async fn list_entities(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Query(q): Query<EntitiesQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = sqlx::query(
        "SELECT id, entity_type, platform, canonical_key, title, payload, metrics, trend, confidence, updated_at \
         FROM pulse_entities \
         WHERE ($1::text IS NULL OR entity_type = $1) \
           AND ($2::text IS NULL OR title ILIKE '%' || $2 || '%' OR canonical_key ILIKE '%' || $2 || '%') \
           AND ($3::text IS NULL OR trend->>'classification' = $3) \
         ORDER BY updated_at DESC LIMIT $4",
    )
    .bind(q.entity_type.as_deref())
    .bind(q.q.as_deref())
    .bind(q.classification.as_deref())
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "entity_type": r.get::<String, _>("entity_type"),
                "platform": r.get::<String, _>("platform"),
                "canonical_key": r.get::<String, _>("canonical_key"),
                "title": r.get::<String, _>("title"),
                "payload": r.get::<Value, _>("payload"),
                "metrics": r.get::<Value, _>("metrics"),
                "trend": r.get::<Value, _>("trend"),
                "confidence": r.get::<Value, _>("confidence"),
                "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_observations(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT provider_id, dataset_id, metric, value, observed_at FROM pulse_observations \
         WHERE entity_id = $1 ORDER BY observed_at DESC",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    Ok(Json(json!({ "items": rows.iter().map(|r| json!({
        "provider_id": r.get::<String, _>("provider_id"),
        "dataset_id": r.get::<String, _>("dataset_id"),
        "metric": r.get::<String, _>("metric"),
        "value": r.get::<Value, _>("value"),
        "observed_at": r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at"),
    })).collect::<Vec<_>>() })))
}

async fn health(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let providers =
        sqlx::query("SELECT id, name, legal_status, enabled FROM pulse_providers ORDER BY id")
            .fetch_all(pool)
            .await
            .map_err(err)?;
    let mut items = Vec::new();
    for p in &providers {
        let pid: String = p.get("id");
        let runs = sqlx::query(
            "SELECT status, started_at, error FROM pulse_provider_runs WHERE provider_id = $1 \
             AND started_at > now() - interval '24 hours' ORDER BY started_at DESC",
        )
        .bind(&pid)
        .fetch_all(pool)
        .await
        .map_err(err)?;
        let runs_24h = runs.len() as i64;
        let ok = runs
            .iter()
            .filter(|r| r.get::<String, _>("status") == "ok")
            .count() as i64;
        let last = runs.first();
        let status = if runs_24h == 0 {
            "never"
        } else if ok == runs_24h {
            "healthy"
        } else if ok > 0 {
            "degraded"
        } else {
            "failed"
        };
        items.push(json!({
            "provider_id": pid,
            "name": p.get::<String, _>("name"),
            "legal_status": p.get::<String, _>("legal_status"),
            "enabled": p.get::<bool, _>("enabled"),
            "status": status,
            "runs_24h": runs_24h,
            "ok_rate": if runs_24h > 0 { ok as f64 / runs_24h as f64 } else { 0.0 },
            "last_run_at": last.map(|r| r.get::<chrono::DateTime<chrono::Utc>, _>("started_at")),
            "last_error": last.and_then(|r| r.get::<Option<String>, _>("error")),
        }));
    }
    let entities: i64 = sqlx::query_scalar("SELECT count(*)::int8 FROM pulse_entities")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let obs: i64 = sqlx::query_scalar("SELECT count(*)::int8 FROM pulse_observations")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    Ok(Json(
        json!({ "providers": items, "entities_total": entities, "observations_total": obs }),
    ))
}

// ---------- 种子 + 调度器 ----------

/// 把现有 collector feed 包装成 3 个内置 Pulse provider (Phase 2 首批, 均公开只读)。
pub fn seed_builtin_providers() -> Vec<Value> {
    vec![
        json!({
            "id": "pulse-hotrank", "name": "GitHub Trending (HotRank)", "platform": "github",
            "category": "trend_intelligence",
            "capabilities": ["repo_ranking", "trending_topics", "emerging_projects"],
            "acquisition": {"type": "builtin_feed", "strategy": "json_api"},
            "datasets": [
                {"id": "gh_ai_agent", "update_frequency_secs": 3600, "entity_type": "trend", "feed_label": "github trending (ai agent)"},
                {"id": "gh_llm", "update_frequency_secs": 3600, "entity_type": "trend", "feed_label": "github trending (llm)"},
                {"id": "gh_agent_fw", "update_frequency_secs": 3600, "entity_type": "trend", "feed_label": "github trending (agent framework)"}
            ],
            "entity_types": ["trend"],
            "reliability": 0.85,
            "legal_status": "public"
        }),
        json!({
            "id": "pulse-hn", "name": "Hacker News (Algolia)", "platform": "hn",
            "category": "social_intelligence",
            "capabilities": ["post_ranking", "viral_content", "topic_trends"],
            "acquisition": {"type": "builtin_feed", "strategy": "json_api"},
            "datasets": [
                {"id": "hn_front", "update_frequency_secs": 1800, "entity_type": "social_post", "feed_label": "hn: front page"},
                {"id": "hn_recent", "update_frequency_secs": 1800, "entity_type": "social_post", "feed_label": "hn: recent stories"},
                {"id": "hn_ai_search", "update_frequency_secs": 3600, "entity_type": "social_post", "feed_label": "hn: search AI"}
            ],
            "entity_types": ["social_post"],
            "reliability": 0.9,
            "legal_status": "public"
        }),
        json!({
            "id": "pulse-gnews", "name": "Google News RSS", "platform": "web",
            "category": "news_intelligence",
            "capabilities": ["global_trends", "topic_news"],
            "acquisition": {"type": "builtin_feed", "strategy": "rss"},
            "datasets": [
                {"id": "gnews_ai_en", "update_frequency_secs": 1800, "entity_type": "news", "feed_label": "gnews: AI agent (en)"},
                {"id": "gnews_llm_en", "update_frequency_secs": 1800, "entity_type": "news", "feed_label": "gnews: large language model (en)"},
                {"id": "gnews_ai_zh", "update_frequency_secs": 1800, "entity_type": "news", "feed_label": "gnews: AI 智能体 (zh)"}
            ],
            "entity_types": ["news"],
            "reliability": 0.8,
            "legal_status": "public"
        }),
    ]
}

/// 启动时把内置 provider 种子写入 pulse_providers (幂等)。
pub async fn seed_providers(pool: &sqlx::PgPool) {
    for m in seed_builtin_providers() {
        let manifest = match ProviderManifest::parse(&m) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "bad seed manifest");
                continue;
            }
        };
        let legal = manifest.legal_status.as_str();
        let enabled = matches!(legal, "public" | "commercial_ok");
        let _ = sqlx::query(
            "INSERT INTO pulse_providers (id, name, platform, category, manifest, legal_status, reliability, enabled)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&manifest.id)
        .bind(&manifest.name)
        .bind(&manifest.platform)
        .bind(&manifest.category)
        .bind(serde_json::to_value(&manifest).unwrap_or(json!({})))
        .bind(legal)
        .bind(rust_decimal::Decimal::try_from(manifest.reliability).unwrap_or(rust_decimal::Decimal::new(5, 1)))
        .bind(enabled)
        .execute(pool)
        .await;
    }
}

/// 调度循环: 每 60s 扫描到期 dataset 并执行 (红线: 只跑 enabled 且 legal 合规的 provider)。
pub async fn run_scheduler(pool: sqlx::PgPool) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let providers = match sqlx::query(
            "SELECT id, manifest FROM pulse_providers WHERE enabled AND legal_status IN ('public','commercial_ok')",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "pulse scheduler: list providers failed");
                continue;
            }
        };
        for p in providers {
            let pid: String = p.get("id");
            let manifest_v: Value = p.get("manifest");
            let manifest = match ProviderManifest::parse(&manifest_v) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for ds in &manifest.datasets {
                let due = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
                    "SELECT max(started_at) FROM pulse_provider_runs WHERE provider_id = $1 AND dataset_id = $2",
                )
                .bind(&pid)
                .bind(&ds.id)
                .fetch_one(&pool)
                .await
                .ok()
                .flatten();
                let due = match due {
                    None => true,
                    Some(t) => {
                        chrono::Utc::now()
                            > t + chrono::Duration::seconds(ds.update_frequency_secs as i64)
                    }
                };
                if !due {
                    continue;
                }
                tracing::info!(provider = %pid, dataset = %ds.id, "pulse scheduler: running dataset");
                if let Err(e) = run_provider_once(&pool, &pid).await {
                    tracing::warn!(provider = %pid, error = %e, "pulse run failed");
                }
            }
        }
    }
}
