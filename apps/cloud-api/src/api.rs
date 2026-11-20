//! /api/v1 route surface (TDD 12.2).
//!
//! WP-01 shipped health/version; this slice wires the Cloud core loop:
//! create agency -> capability matrix -> route -> quota reserve -> execute
//! (mock adapter until real platform审核 passes) -> receipt -> route audit.
//! Tenant context comes from the authenticated principal; in this dev slice
//! it is the `x-agency-id` header (JWT wiring lands with WP-03 auth routes).

use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sincy_common::{AgencyId, AppError, ErrorKind, RequestId};
use sincy_execution::killswitch::{KillScope, is_paused};
use sincy_platform::matrix::PgCapabilityMatrix;
use sincy_platform::quota::QuotaLedger;
use sincy_platform::youtube_mock::MockYouTubeAdapter;
use sincy_routing::action::{ChannelAction, ExecCtx, ExecOutcome, PublishingPackageRef};
use sincy_routing::capability::Route;
use sincy_routing::router::{
    Preconditions, Router as ExecutionRouter, RoutingOverrides, fallback_decision,
};
use sqlx::Row;
use uuid::Uuid;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/agent/connect", get(agent_connect_placeholder))
        .route("/agencies", post(create_agency))
        .route("/capabilities", get(list_capabilities))
        .route("/tasks/publish", post(publish_task))
        .route("/tasks/{id}/route", get(get_task_route))
        .route("/kill-switches", post(create_kill_switch))
        .route("/kill-switches/{id}", delete(remove_kill_switch))
        .layer(axum::middleware::from_fn(request_id_middleware))
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "status": "ok", "ts": chrono::Utc::now().to_rfc3339() }))
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "service": "cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_schema": sincy_protocol::PROTOCOL_SCHEMA_VERSION,
    }))
}

/// WP-08 replaces this with the real WSS gateway (device auth, envelope
/// validation, heartbeat).
async fn agent_connect_placeholder() -> Response {
    app_error_response(
        AppError::new(
            ErrorKind::NonRetryable,
            "not_implemented",
            "agent WSS gateway lands in WP-08",
        )
        .with_request_id(RequestId::new().to_string()),
    )
}

pub async fn not_found() -> Response {
    app_error_response(
        AppError::non_retryable("not_found", "no such route")
            .with_request_id(RequestId::new().to_string()),
    )
}

// ---------------------------------------------------------------------------
// agencies
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateAgencyBody {
    name: String,
}

#[derive(Debug, Serialize)]
struct AgencyDto {
    id: Uuid,
    name: String,
}

async fn create_agency(
    State(state): State<AppState>,
    Json(body): Json<CreateAgencyBody>,
) -> Result<Json<AgencyDto>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(&body.name)
        .execute(pool)
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "agency_create_failed",
                e.to_string(),
            ))
        })?;
    Ok(Json(AgencyDto {
        id,
        name: body.name,
    }))
}

// ---------------------------------------------------------------------------
// capabilities (platform-level, read-only here; Admin writes in WP-05 admin)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CapabilityQuery {
    platform: String,
    #[serde(default)]
    #[allow(dead_code)] // reserved for action filter (WP-05 admin UI)
    action: Option<String>,
}

async fn list_capabilities(
    State(state): State<AppState>,
    Query(q): Query<CapabilityQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT platform, action, route, status, constraints, priority, verified_at, evidence_ref
         FROM channel_capabilities WHERE platform = $1 ORDER BY action, priority",
    )
    .bind(&q.platform)
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "platform": r.get::<String, _>("platform"),
                "action": r.get::<String, _>("action"),
                "route": r.get::<String, _>("route"),
                "status": r.get::<String, _>("status"),
                "constraints": r.get::<Value, _>("constraints"),
                "priority": r.get::<i32, _>("priority"),
                "verified_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("verified_at"),
                "evidence_ref": r.get::<Option<String>, _>("evidence_ref"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

// ---------------------------------------------------------------------------
// publish task: route -> quota -> execute -> receipt (Cloud MVP loop)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PublishTaskBody {
    agency_id: Uuid,
    brand_id: Uuid,
    platform: String,
    action: String,
    package_id: Uuid,
    package_version: u32,
    idempotency_key: String,
}

#[derive(Debug, Serialize)]
struct PublishTaskResponse {
    task_id: Uuid,
    route: String,
    status: String,
    external_content_id: Option<String>,
    receipt: Option<Value>,
    fallback: Option<String>,
}

async fn publish_task(
    State(state): State<AppState>,
    Json(body): Json<PublishTaskBody>,
) -> Result<Json<PublishTaskResponse>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency = AgencyId::from_uuid(body.agency_id);
    let task_id = Uuid::new_v4();

    // 0. idempotency: duplicate key => return recorded result, never re-execute
    if let Some(row) = sqlx::query(
        "SELECT id AS task_id, state FROM device_tasks WHERE agency_id = $1 AND idempotency_key = $2",
    )
    .bind(agency.as_uuid())
    .bind(&body.idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    {
        return Ok(Json(PublishTaskResponse {
            task_id: row.get("task_id"),
            route: "idempotent_replay".into(),
            status: row.get("state"),
            external_content_id: None,
            receipt: None,
            fallback: None,
        }));
    }

    // 1. kill switch check (FR-025) — global + tenant rows
    let ks_rows = sqlx::query(
        "SELECT scope, target_id, reason FROM kill_switches
         WHERE active AND (agency_id IS NULL OR agency_id = $1)",
    )
    .bind(agency.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let mut store_hits = Vec::new();
    for r in &ks_rows {
        let scope = match r.get::<String, _>("scope").as_str() {
            "brand" => KillScope::Brand,
            "client" => KillScope::Client,
            "agency" => KillScope::Agency,
            _ => KillScope::Global,
        };
        store_hits.push(sincy_execution::killswitch::ActiveKillSwitch {
            scope,
            target_id: r.get("target_id"),
            reason: r.get::<Option<String>, _>("reason").unwrap_or_default(),
        });
    }
    struct VecStore(Vec<sincy_execution::killswitch::ActiveKillSwitch>);
    impl sincy_execution::killswitch::KillSwitchStore for VecStore {
        fn active(&self) -> Vec<sincy_execution::killswitch::ActiveKillSwitch> {
            self.0.clone()
        }
    }
    if let Some(reason) = is_paused(
        &VecStore(store_hits),
        body.agency_id,
        None,
        Some(body.brand_id),
    ) {
        return Err(app_error_response(AppError::needs_user(
            "kill_switch_active",
            format!("任务被急停: {reason}"),
        )));
    }

    // 2. capability matrix + route selection
    let matrix = PgCapabilityMatrix::new(pool);
    let snap = matrix
        .snapshot(&body.platform, &body.action)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let router = ExecutionRouter::new(&snap);

    // quota availability decides api_ready
    let quota_row = sqlx::query(
        "SELECT qp.id AS pool_id, qp.limit_value - qp.used_value AS avail, pa.platform
         FROM quota_pools qp JOIN platform_apps pa ON pa.id = qp.platform_app_id
         WHERE pa.platform = $1 AND qp.quota_kind = 'videos.insert'",
    )
    .bind(&body.platform)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let (quota_pool_id, quota_avail) = match quota_row {
        Some(r) => (Some(r.get::<Uuid, _>("pool_id")), r.get::<i64, _>("avail")),
        None => (None, 0),
    };
    let cost: i64 = 1600; // videos.insert cost (mock; real adapter reads constraints)
    // route-time check is coarse (any headroom); the ledger reserve is authoritative
    let pre = Preconditions {
        api_ready: quota_avail > 0,
        api_block_reason: Some(format!("quota available {quota_avail} < cost {cost}")),
        device_ready: false,
        device_block_reason: Some("no device bound in this slice".into()),
        assisted_ready: true,
        assisted_block_reason: None,
    };
    let decision = router.select(
        &body.platform,
        &body.action,
        &RoutingOverrides::default(),
        &pre,
    );

    let Some(decision) = decision else {
        // no usable route: audit skipped reasons
        let skipped = router.audit_skipped(
            &body.platform,
            &body.action,
            &RoutingOverrides::default(),
            &pre,
        );
        return Err(app_error_response(
            AppError::no_side_effect(
                "no_route_available",
                format!("no usable route: {skipped:?}"),
            )
            .with_request_id(RequestId::new().to_string()),
        ));
    };

    // record task + route audit (tenant-scoped tx)
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(&format!(
        "SET LOCAL app.current_agency_id = '{}'",
        agency.as_uuid()
    ))
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO device_tasks (id, agency_id, brand_id, idempotency_key, state, route_id)
         VALUES ($1, $2, $3, $4, 'EXECUTING', $5)",
    )
    .bind(task_id)
    .bind(agency.as_uuid())
    .bind(body.brand_id)
    .bind(&body.idempotency_key)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO execution_routes (agency_id, task_id, platform, action, selected_route, capability_status, priority, skipped)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(agency.as_uuid())
    .bind(task_id)
    .bind(&body.platform)
    .bind(&body.action)
    .bind(decision.reason.selected.as_str())
    .bind(format!("{:?}", decision.reason.capability_status).to_lowercase())
    .bind(decision.reason.priority)
    .bind(serde_json::to_value(&decision.reason.skipped).unwrap_or_default())
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    tx.commit()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;

    // 3. execute on the selected route
    let ctx = ExecCtx {
        agency_id: agency.to_string(),
        brand_id: body.brand_id.to_string(),
        account_id: Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        route_id: Uuid::new_v4().to_string(),
        idempotency_key: body.idempotency_key.clone(),
    };
    let pkg = PublishingPackageRef {
        package_id: body.package_id.to_string(),
        version: body.package_version,
        platform: body.platform.clone(),
        action: body.action.clone(),
    };

    match decision.reason.selected {
        Route::Api => {
            let pool_id = quota_pool_id.ok_or_else(|| {
                app_error_response(AppError::no_side_effect(
                    "no_quota_pool",
                    "platform app not configured",
                ))
            })?;
            let ledger = QuotaLedger::new(pool);
            match ledger.reserve(agency, pool_id, cost, task_id).await {
                Ok(reservation) => {
                    let adapter = MockYouTubeAdapter {
                        quota_pool_id: pool_id,
                        quota_cost: cost,
                        simulate_quota_exhausted: false,
                    };
                    let outcome = adapter.execute(&ctx, &pkg).await;
                    match outcome {
                        ExecOutcome::Success {
                            external_content_id,
                            receipt,
                        } => {
                            let _ = ledger.confirm(reservation.reservation_id).await;
                            Ok(Json(PublishTaskResponse {
                                task_id,
                                route: "api".into(),
                                status: "PUBLISHED".into(),
                                external_content_id,
                                receipt: Some(receipt),
                                fallback: None,
                            }))
                        }
                        ExecOutcome::Uncertain(e) => Err(app_error_response(AppError::uncertain(
                            "verification_pending",
                            e.to_string(),
                        ))),
                        ExecOutcome::NoSideEffectFailure(e) => {
                            let _ = ledger.release(reservation.reservation_id).await;
                            let remaining = [Route::Assisted];
                            let fb =
                                fallback_decision(&ExecOutcome::NoSideEffectFailure(e), &remaining);
                            Ok(Json(PublishTaskResponse {
                                task_id,
                                route: "api".into(),
                                status: "FAILED_NO_SIDE_EFFECT".into(),
                                external_content_id: None,
                                receipt: None,
                                fallback: Some(format!("{fb:?}")),
                            }))
                        }
                    }
                }
                Err(_) => {
                    // QUOTA_WAIT semantics: no side effect, degrade to assisted
                    Ok(Json(PublishTaskResponse {
                        task_id,
                        route: "api".into(),
                        status: "QUOTA_WAIT".into(),
                        external_content_id: None,
                        receipt: None,
                        fallback: Some("Degrade(Assisted)".into()),
                    }))
                }
            }
        }
        Route::Assisted => Ok(Json(PublishTaskResponse {
            task_id,
            route: "assisted".into(),
            status: "ASSISTED_READY".into(),
            external_content_id: None,
            receipt: None,
            fallback: None,
        })),
        Route::Device => Ok(Json(PublishTaskResponse {
            task_id,
            route: "device".into(),
            status: "DEVICE_QUEUED".into(),
            external_content_id: None,
            receipt: None,
            fallback: None,
        })),
    }
}

async fn get_task_route(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query(
        "SELECT task_id, platform, action, selected_route, capability_status, priority, skipped, decided_at
         FROM execution_routes WHERE task_id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| app_error_response(AppError::non_retryable("not_found", "no route for task")))?;
    Ok(Json(json!({
        "task_id": row.get::<Uuid, _>("task_id"),
        "platform": row.get::<String, _>("platform"),
        "action": row.get::<String, _>("action"),
        "selected_route": row.get::<String, _>("selected_route"),
        "capability_status": row.get::<String, _>("capability_status"),
        "priority": row.get::<i32, _>("priority"),
        "skipped": row.get::<Value, _>("skipped"),
        "decided_at": row.get::<chrono::DateTime<chrono::Utc>, _>("decided_at"),
    })))
}

// ---------------------------------------------------------------------------
// kill switches (FR-025)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreateKillSwitchBody {
    agency_id: Option<Uuid>,
    scope: String,
    target_id: Option<Uuid>,
    reason: String,
}

async fn create_kill_switch(
    State(state): State<AppState>,
    Json(body): Json<CreateKillSwitchBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO kill_switches (id, agency_id, scope, target_id, reason, created_by)
         VALUES ($1, $2, $3, $4, $5, 'api')",
    )
    .bind(id)
    .bind(body.agency_id)
    .bind(&body.scope)
    .bind(body.target_id)
    .bind(&body.reason)
    .execute(pool)
    .await
    .map_err(|e| {
        app_error_response(AppError::non_retryable(
            "kill_switch_create_failed",
            e.to_string(),
        ))
    })?;
    Ok(Json(json!({ "id": id, "active": true })))
}

async fn remove_kill_switch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let res = sqlx::query("UPDATE kill_switches SET active = false WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "kill_switch_update_failed",
                e.to_string(),
            ))
        })?;
    if res.rows_affected() == 0 {
        return Err(app_error_response(AppError::non_retryable(
            "not_found",
            "kill switch not found",
        )));
    }
    Ok(Json(json!({ "id": id, "active": false })))
}

// ---------------------------------------------------------------------------
// middleware + error conversion
// ---------------------------------------------------------------------------

/// Middleware: attach a fresh request_id header + log line for every request.
pub async fn request_id_middleware(req: Request, next: axum::middleware::Next) -> Response {
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| RequestId::new().to_string());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    tracing::info_span!("http", %rid, %method, %path);
    let mut res = next.run(req).await;
    if let Ok(v) = axum::http::HeaderValue::from_str(&rid) {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}

/// Convert an AppError into the unified JSON error response (TDD 12.1).
pub fn app_error_response(err: AppError) -> Response {
    let status =
        StatusCode::from_u16(err.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut body = err.to_body();
    if body.error.request_id.is_empty() {
        body.error.request_id = RequestId::new().to_string();
    }
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn state() -> AppState {
        AppState {
            pool: None,
            env: "local".into(),
        }
    }

    #[tokio::test]
    async fn healthz_ok() {
        let app = routes().with_state(state());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_uses_unified_error() {
        let res = not_found().await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn db_endpoints_report_unavailable_without_pool() {
        let app = routes().with_state(state());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/capabilities?platform=youtube")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "database_unavailable");
    }

    #[tokio::test]
    async fn agent_placeholder_shape() {
        let res = agent_connect_placeholder().await;
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "not_implemented");
    }
}
