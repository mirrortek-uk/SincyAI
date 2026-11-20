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
use sincy_ai::{
    Analytics, BudgetChecker, CostRecord, GenerationRequest, LlmGateway, MetricImport,
    MockLlmGateway, OpenAiCompatibleGateway,
};
use sincy_common::{AgencyId, AppError, RequestId};
use sincy_execution::killswitch::{KillScope, is_paused};
use sincy_platform::matrix::PgCapabilityMatrix;
use sincy_platform::quota::QuotaLedger;
use sincy_platform::youtube::{MediaBlob, YouTubeAdapter};
use sincy_platform::youtube_mock::MockYouTubeAdapter;
use sincy_routing::action::{ChannelAction, ExecCtx, ExecOutcome, PublishingPackageRef};
use sincy_routing::capability::Route;
use sincy_routing::router::{
    Preconditions, Router as ExecutionRouter, RoutingOverrides, fallback_decision,
};
use sincy_security::password;
use sincy_transfer::store::{LocalObjectStore, ObjectStore};
use sqlx::Row;
use uuid::Uuid;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/auth/login", post(crate::auth::login))
        .route("/agent/connect", get(crate::gateway::agent_connect))
        .route("/agencies", post(create_agency))
        .route("/agencies/me", get(get_my_agency))
        .route("/capabilities", get(list_capabilities))
        .route("/tasks", get(list_tasks))
        .route("/tasks/publish", post(publish_task))
        .route("/tasks/{id}/route", get(get_task_route))
        .route("/devices", get(list_devices))
        .route(
            "/kill-switches",
            get(list_kill_switches).post(create_kill_switch),
        )
        .route("/kill-switches/{id}", delete(remove_kill_switch))
        .route("/analytics/metrics", post(import_metrics))
        .route("/ai/generate", post(ai_generate))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(axum::middleware::from_fn(crate::auth::auth_middleware))
        .layer(cors_layer())
}

/// CORS: origins from SINCY_CORS_ORIGINS (comma-separated); defaults to the
/// local console. Production deployments should serve the console from the
/// same domain (frontend rewrites /api -> cloud-api) and keep this tight.
fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin};
    let origins = std::env::var("SINCY_CORS_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:3000,http://127.0.0.1:3000".to_string());
    let allow_origin: Vec<axum::http::HeaderValue> = origins
        .split(',')
        .filter_map(|o| o.trim().parse::<axum::http::HeaderValue>().ok())
        .collect();
    tower_http::cors::CorsLayer::new()
        .allow_origin(if allow_origin.is_empty() {
            AllowOrigin::any()
        } else {
            AllowOrigin::list(allow_origin)
        })
        .allow_methods(AllowMethods::any())
        .allow_headers(AllowHeaders::any())
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
    /// 正式注册: 创建机构时同时创建 owner 账号 (email 全局唯一)
    owner_email: Option<String>,
    owner_password: Option<String>,
}

async fn create_agency(
    State(state): State<AppState>,
    Json(body): Json<CreateAgencyBody>,
) -> Result<Json<Value>, Response> {
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
    // 正式产品化: 建机构时可同时创建 owner 成员并直接签发登录令牌
    let mut token: Option<String> = None;
    if let (Some(email), Some(password)) = (&body.owner_email, &body.owner_password) {
        if password.len() < 8 {
            return Err(app_error_response(AppError::non_retryable(
                "weak_password",
                "owner password must be at least 8 characters",
            )));
        }
        let hash = password::hash_password(password).map_err(|e| {
            app_error_response(AppError::non_retryable("auth_failed", e.to_string()))
        })?;
        let member_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agency_members (id, agency_id, email, role, password_hash) VALUES ($1, $2, $3, 'owner', $4)",
        )
        .bind(member_id)
        .bind(id)
        .bind(email)
        .bind(&hash)
        .execute(pool)
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "member_create_failed",
                e.to_string(),
            ))
        })?;
        token = Some(
            state
                .jwt()
                .issue(
                    &member_id.to_string(),
                    &id.to_string(),
                    vec!["owner".into()],
                )
                .map_err(|e| {
                    app_error_response(AppError::non_retryable("token_issue_failed", e.to_string()))
                })?,
        );
    }
    Ok(Json(json!({ "id": id, "name": body.name, "token": token })))
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
    /// 兼容旧调用; 受保护路由上 agency 一律以 JWT 为准, 此字段若存在必须一致
    agency_id: Option<Uuid>,
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
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<PublishTaskBody>,
) -> Result<Json<PublishTaskResponse>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    // agency_id 一律来自 JWT, 不信任客户端传参 (Guide 6)
    if let Some(a) = body.agency_id
        && a != *auth.agency_id.as_uuid()
    {
        return Err(app_error_response(AppError::non_retryable(
            "tenant_mismatch",
            "agency_id in body does not match the authenticated tenant",
        )));
    }
    let agency = auth.agency_id;
    let task_id = Uuid::new_v4();
    crate::metrics::Metrics::inc(&crate::metrics::metrics().tasks_published_total);

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
        *agency.as_uuid(),
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
            crate::metrics::Metrics::inc(&crate::metrics::metrics().route_api_total);
            let pool_id = quota_pool_id.ok_or_else(|| {
                app_error_response(AppError::no_side_effect(
                    "no_quota_pool",
                    "platform app not configured",
                ))
            })?;
            let ledger = QuotaLedger::new(pool);
            match ledger.reserve(agency, pool_id, cost, task_id).await {
                Ok(reservation) => {
                    crate::metrics::Metrics::inc(&crate::metrics::metrics().quota_reserved_total);
                    // 真实产品路径: 生产环境(allow_mock_adapters=false)走真实平台
                    // adapter; mock 仅在显式开启的 dev 环境可用 (DoD 红线)
                    let outcome = if state.allow_mock_adapters {
                        let adapter: Box<dyn ChannelAction> = Box::new(MockYouTubeAdapter {
                            quota_pool_id: pool_id,
                            quota_cost: cost,
                            simulate_quota_exhausted: false,
                        });
                        adapter.execute(&ctx, &pkg).await
                    } else {
                        build_real_adapter(pool, agency, &body, &pkg)
                            .await?
                            .execute(&ctx, &pkg)
                            .await
                    };
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
                    crate::metrics::Metrics::inc(&crate::metrics::metrics().quota_exceeded_total);
                    crate::metrics::Metrics::inc(&crate::metrics::metrics().route_degraded_total);
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
        Route::Assisted => {
            crate::metrics::Metrics::inc(&crate::metrics::metrics().route_assisted_total);
            Ok(Json(PublishTaskResponse {
                task_id,
                route: "assisted".into(),
                status: "ASSISTED_READY".into(),
                external_content_id: None,
                receipt: None,
                fallback: None,
            }))
        }
        Route::Device => {
            crate::metrics::Metrics::inc(&crate::metrics::metrics().route_device_total);
            Ok(Json(PublishTaskResponse {
                task_id,
                route: "device".into(),
                status: "DEVICE_QUEUED".into(),
                external_content_id: None,
                receipt: None,
                fallback: None,
            }))
        }
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

async fn list_kill_switches(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Query(_q): Query<ListKillSwitchQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    // 租户一律来自 JWT: 全局急停 + 本租户急停
    let rows = sqlx::query(
        "SELECT id, agency_id, scope, target_id, reason, active, created_at FROM kill_switches
         WHERE active AND (agency_id IS NULL OR agency_id = $1)
         ORDER BY created_at DESC LIMIT 200",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "agency_id": r.get::<Option<Uuid>, _>("agency_id"),
                "scope": r.get::<String, _>("scope"),
                "target_id": r.get::<Option<Uuid>, _>("target_id"),
                "reason": r.get::<String, _>("reason"),
                "active": r.get::<bool, _>("active"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct ListKillSwitchQuery {
    #[allow(dead_code)] // 受保护路由上租户来自 JWT, 该参数被忽略
    agency_id: Option<Uuid>,
}

async fn create_kill_switch(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<CreateKillSwitchBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let id = Uuid::new_v4();
    let jwt_agency = *auth.agency_id.as_uuid();
    if let Some(a) = body.agency_id
        && a != jwt_agency
    {
        return Err(app_error_response(AppError::non_retryable(
            "tenant_mismatch",
            "agency_id in body does not match the authenticated tenant",
        )));
    }
    // scope=agency without explicit target means "this agency"
    let target_id = match body.scope.as_str() {
        "agency" => Some(body.target_id.unwrap_or(jwt_agency)),
        _ => body.target_id,
    };
    sqlx::query(
        "INSERT INTO kill_switches (id, agency_id, scope, target_id, reason, created_by)
         VALUES ($1, $2, $3, $4, $5, 'api')",
    )
    .bind(id)
    .bind(Some(jwt_agency))
    .bind(&body.scope)
    .bind(target_id)
    .bind(&body.reason)
    .execute(pool)
    .await
    .map_err(|e| {
        app_error_response(AppError::non_retryable(
            "kill_switch_create_failed",
            e.to_string(),
        ))
    })?;
    crate::metrics::Metrics::inc(&crate::metrics::metrics().kill_switch_toggles_total);
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
    crate::metrics::Metrics::inc(&crate::metrics::metrics().kill_switch_toggles_total);
    Ok(Json(json!({ "id": id, "active": false })))
}

// ---------------------------------------------------------------------------
// middleware + error conversion
// ---------------------------------------------------------------------------

/// Middleware: attach a fresh request_id header + log line for every request.
pub async fn request_id_middleware(req: Request, next: axum::middleware::Next) -> Response {
    crate::metrics::Metrics::inc(&crate::metrics::metrics().http_requests_total);
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
    let mut status =
        StatusCode::from_u16(err.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    // 认证失败一律 401 (语义化状态码)
    if err.code == "unauthorized" || err.code == "auth_not_initialized" {
        status = StatusCode::UNAUTHORIZED;
    }
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
        AppState::new(None, "local".into())
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
        let st = state();
        let token = st
            .jwt()
            .issue(
                &Uuid::new_v4().to_string(),
                &Uuid::new_v4().to_string(),
                vec!["owner".into()],
            )
            .expect("token");
        let app = routes().with_state(st);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/capabilities?platform=youtube")
                    .header("authorization", format!("Bearer {token}"))
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
}

// ---------------------------------------------------------------------------
// 正式产品化端点: 我的机构 / 任务列表 / 设备列表 / 指标导入 / AI 生成
// ---------------------------------------------------------------------------

async fn get_my_agency(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query("SELECT id, name, plan, created_at FROM agencies WHERE id = $1")
        .bind(auth.agency_id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
        .ok_or_else(|| {
            app_error_response(AppError::non_retryable("not_found", "agency not found"))
        })?;
    Ok(Json(json!({
        "id": row.get::<Uuid, _>("id"),
        "name": row.get::<String, _>("name"),
        "plan": row.get::<String, _>("plan"),
        "role": auth.roles.first().cloned().unwrap_or_default(),
        "member_id": auth.member_id,
    })))
}

async fn list_tasks(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT t.id, t.state, t.idempotency_key, t.route_id, t.package_id, t.package_version, t.created_at,
                r.selected_route, r.platform, r.action
         FROM device_tasks t LEFT JOIN execution_routes r ON r.task_id = t.id
         WHERE t.agency_id = $1 ORDER BY t.created_at DESC LIMIT 100",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "state": r.get::<String, _>("state"),
                "idempotency_key": r.get::<String, _>("idempotency_key"),
                "route": r.get::<Option<String>, _>("selected_route"),
                "platform": r.get::<Option<String>, _>("platform"),
                "action": r.get::<Option<String>, _>("action"),
                "package_id": r.get::<Option<Uuid>, _>("package_id"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_devices(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT id, name, platform, os_version, agent_version, state, last_seen, created_at
         FROM devices WHERE agency_id = $1 ORDER BY created_at DESC LIMIT 200",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<Option<String>, _>("name"),
                "platform": r.get::<String, _>("platform"),
                "os_version": r.get::<Option<String>, _>("os_version"),
                "agent_version": r.get::<Option<String>, _>("agent_version"),
                "state": r.get::<String, _>("state"),
                "last_seen": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_seen"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct ImportMetricsBody {
    platform: String,
    external_content_id: String,
    account_id: Option<Uuid>,
    metric_date: chrono::NaiveDate,
    metrics: Value,
    /// api | device | manual — 统计口径必须标记 (TDD 6)
    source: String,
}

/// POST /api/v1/analytics/metrics — 导入平台指标 (FR-016), 口径标注必填.
async fn import_metrics(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<ImportMetricsBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    if !["api", "device", "manual"].contains(&body.source.as_str()) {
        return Err(app_error_response(AppError::non_retryable(
            "bad_source",
            "source must be one of api|device|manual",
        )));
    }
    let analytics = Analytics::new(pool);
    let id = analytics
        .import_metrics(
            auth.agency_id,
            &MetricImport {
                platform: body.platform,
                external_content_id: body.external_content_id,
                account_id: body.account_id,
                metric_date: body.metric_date,
                metrics: body.metrics,
                source: body.source,
            },
        )
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Debug, Deserialize)]
struct AiGenerateBody {
    prompt: String,
    model: Option<String>,
    max_tokens: Option<u32>,
    /// 关联内容项, 用于成本归因 (FR-026)
    content_item_id: Option<Uuid>,
}

/// POST /api/v1/ai/generate — 受预算熔断保护的 LLM 生成 (FR-026 + WP-14).
/// mock provider 仅在显式允许时可用; 生产走真实 openai-compatible 端点.
async fn ai_generate(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<AiGenerateBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let checker = BudgetChecker::new(
        pool,
        state
            .monthly_budget_usd
            .map(|v| rust_decimal::Decimal::try_from(v).unwrap_or(rust_decimal::Decimal::ZERO)),
    );
    // 估算成本: 约 $0.002/1k tokens (保守估计, 按 max_tokens 全额计)
    let max_tokens = body.max_tokens.unwrap_or(1024).max(1);
    let est_usd = max_tokens as f64 / 1000.0 * 0.002;
    let est = rust_decimal::Decimal::try_from(est_usd).unwrap_or(rust_decimal::Decimal::ZERO);
    if let Err(e) = checker.check(auth.agency_id, None, est).await {
        crate::metrics::Metrics::inc(&crate::metrics::metrics().budget_blocked_total);
        return Err(app_error_response(AppError::needs_user(
            "budget_exceeded",
            e.to_string(),
        )));
    }
    let provider = std::env::var("SINCY_LLM_PROVIDER").unwrap_or_else(|_| "mock".into());
    let req = GenerationRequest {
        prompt: body.prompt,
        model: body.model.unwrap_or_else(|| {
            std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "mock-1".into())
        }),
        max_tokens,
        content_item_id: body.content_item_id.map(|u| u.to_string()),
    };
    let result = if provider == "mock" {
        if !state.allow_mock_adapters {
            return Err(app_error_response(AppError::non_retryable(
                "mock_disabled",
                "mock LLM provider is disabled; set SINCY_LLM_PROVIDER=openai-compatible",
            )));
        }
        MockLlmGateway.generate(&req).await
    } else {
        let gw = OpenAiCompatibleGateway::new(
            std::env::var("SINCY_LLM_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            std::env::var("SINCY_LLM_API_KEY").unwrap_or_default(),
            std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        );
        gw.generate(&req).await
    }
    .map_err(|e| app_error_response(AppError::retryable("llm_failed", e.to_string())))?;
    // 成本落账 (FR-026)
    let _ = checker
        .record_cost(
            auth.agency_id,
            &CostRecord {
                brand_id: None,
                content_item_id: body.content_item_id,
                provider: if result.mock {
                    "mock"
                } else {
                    "openai-compatible"
                },
                model: &result.model,
                kind: "llm",
                tokens_in: result.tokens_in as i64,
                tokens_out: result.tokens_out as i64,
                cost_usd: est,
            },
        )
        .await;
    Ok(Json(json!({
        "text": result.text,
        "model": result.model,
        "tokens_in": result.tokens_in,
        "tokens_out": result.tokens_out,
        "mock": result.mock,
        "estimated_cost_usd": est.to_string(),
    })))
}

/// 为 API 路由构建真实平台 adapter: 从发布包取账号与媒体资产,
/// 媒体字节从对象存储取出 (大文件不走控制面, ADR-005).
async fn build_real_adapter(
    pool: &sqlx::PgPool,
    agency: AgencyId,
    body: &PublishTaskBody,
    pkg: &PublishingPackageRef,
) -> Result<Box<dyn ChannelAction>, Response> {
    let pkg_row = sqlx::query(
        "SELECT account_id, text_content, constraints FROM publishing_packages
         WHERE id = $1 AND version = $2 AND agency_id = $3",
    )
    .bind(body.package_id)
    .bind(body.package_version as i32)
    .bind(agency.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "package_not_found",
            "publishing package not found or not approved",
        ))
    })?;
    let account_id: Uuid = pkg_row.get("account_id");
    let text: String = pkg_row.get("text_content");
    let constraints: Value = pkg_row.get("constraints");

    let mut adapter = YouTubeAdapter::for_account(pool, agency.as_uuid(), &account_id)
        .await
        .map_err(app_error_response)?;
    adapter.title = text.chars().take(100).collect();
    adapter.description = text;
    if let Some(asset_id) = constraints["media_asset_id"].as_str() {
        let asset = sqlx::query(
            "SELECT storage_ref, mime, bytes FROM assets WHERE id = $1 AND agency_id = $2",
        )
        .bind(Uuid::parse_str(asset_id).map_err(|e| {
            app_error_response(AppError::non_retryable("bad_asset_id", e.to_string()))
        })?)
        .bind(agency.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
        .ok_or_else(|| {
            app_error_response(AppError::no_side_effect(
                "asset_not_found",
                "media asset missing",
            ))
        })?;
        let store = LocalObjectStore::new(
            std::env::var("SINCY_OBJECT_STORE_ROOT").unwrap_or_else(|_| "./var/objects".into()),
        );
        let bytes = store
            .get(&asset.get::<String, _>("storage_ref"))
            .await
            .map_err(|e| {
                app_error_response(AppError::no_side_effect("asset_read_failed", e.to_string()))
            })?;
        adapter.media = Some(MediaBlob {
            bytes,
            mime: asset.get("mime"),
            filename: asset_id.to_string(),
        });
    }
    let _ = pkg;
    Ok(Box::new(adapter))
}
