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
    routing::{delete, get, patch, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sincy_ai::{Analytics, BudgetChecker, CostRecord, GenerationRequest, MetricImport};
use sincy_common::{AgencyId, AppError, RequestId};
use sincy_device::DeviceManager;
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
        .route("/settings/platform-review-rules", get(list_review_rules))
        .route(
            "/settings/platform-review-rules/{platform}",
            put(upsert_review_rule),
        )
        .route("/auth/login", post(crate::auth::login))
        .route("/agent/connect", get(crate::gateway::agent_connect))
        .route("/agent/tasks/{id}", get(get_agent_task))
        .route("/agencies", post(create_agency))
        .route("/agencies/me", get(get_my_agency))
        .route("/capabilities", get(list_capabilities))
        .route("/tasks", get(list_tasks))
        .route("/tasks/publish", post(publish_task))
        .route("/tasks/{id}/route", get(get_task_route))
        .route("/tasks/{id}/retry", post(retry_task))
        .route("/devices", get(list_devices))
        .route("/devices/{id}/revoke", post(revoke_device))
        .route("/devices/{id}/rotate", post(rotate_device))
        .route("/devices/register/start", post(start_device_registration))
        .route("/devices/pairing-codes", post(create_pairing_code))
        .route("/devices/pair", post(pair_with_code))
        .route("/setup/run", post(run_setup))
        .route("/setup/reply_auto", post(reply_auto))
        .route("/setup/apps", get(list_setup_apps))
        .route("/setup/apps/{app_key}", patch(update_setup_app))
        .route(
            "/devices/register/complete",
            post(complete_device_registration),
        )
        .route(
            "/kill-switches",
            get(list_kill_switches).post(create_kill_switch),
        )
        .route("/kill-switches/{id}", delete(remove_kill_switch))
        .route("/analytics/metrics", post(import_metrics))
        .route("/ai/generate", post(ai_generate))
        .route("/clients", get(list_clients).post(create_client))
        .route("/brands", get(list_brands).post(create_brand))
        .route("/brands/{id}", patch(update_brand))
        .route("/packages", get(list_packages).post(create_package))
        .route("/packages/{id}/approve", post(approve_package))
        .route("/metrics", get(crate::metrics::metrics_handler))
        .merge(crate::architecture::routes())
        .merge(crate::intel::routes())
        .merge(crate::employees::routes())
        .merge(crate::pulse::routes())
        .merge(crate::industry::routes())
        .merge(crate::pipeline::routes())
        .merge(crate::factory::routes())
        .merge(crate::account_metrics::routes())
        .merge(crate::skills::routes())
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

async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "ts": chrono::Utc::now().to_rfc3339(),
        // 一眼看出这是 demo 还是真环境: 生产环境下 env!=local 且 mock 全关
        "env": state.env,
        "allow_mock_adapters": state.allow_mock_adapters,
        "llm_provider": std::env::var("SINCY_LLM_PROVIDER").unwrap_or_else(|_| "mock".into()),
        "database": state.pool.is_some(),
    }))
}

/// GET /settings/platform-review-rules — 各平台审核门(加权字数上限 + 超限行为)。
async fn list_review_rules(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT platform, weighted_limit, on_over_limit, note, updated_at FROM platform_review_rules ORDER BY platform",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "platform": r.get::<String, _>("platform"),
                "weighted_limit": r.get::<i32, _>("weighted_limit"),
                "on_over_limit": r.get::<String, _>("on_over_limit"),
                "note": r.get::<String, _>("note"),
                "updated_at": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            })
        })
        .collect();
    Ok(Json(json!({"items": items})))
}

/// PUT /settings/platform-review-rules/{platform} — 修改某平台审核门。
/// body: {weighted_limit, on_over_limit: "reject"|"thread", note?}
async fn upsert_review_rule(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(platform): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let limit = body
        .get("weighted_limit")
        .and_then(|v| v.as_i64())
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            app_error_response(AppError::non_retryable(
                "bad_request",
                "weighted_limit (positive int) required",
            ))
        })?;
    let on_over = body
        .get("on_over_limit")
        .and_then(|v| v.as_str())
        .filter(|v| *v == "reject" || *v == "thread")
        .ok_or_else(|| {
            app_error_response(AppError::non_retryable(
                "bad_request",
                "on_over_limit must be 'reject' or 'thread'",
            ))
        })?;
    let note = body.get("note").and_then(|v| v.as_str()).unwrap_or("");
    sqlx::query(
        "INSERT INTO platform_review_rules (platform, weighted_limit, on_over_limit, note, updated_at)
         VALUES ($1, $2, $3, $4, now())
         ON CONFLICT (platform) DO UPDATE SET weighted_limit = EXCLUDED.weighted_limit,
           on_over_limit = EXCLUDED.on_over_limit, note = EXCLUDED.note, updated_at = now()",
    )
    .bind(&platform)
    .bind(limit as i32)
    .bind(on_over)
    .bind(note)
    .execute(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(
        json!({"platform": platform, "weighted_limit": limit, "on_over_limit": on_over, "note": note}),
    ))
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
pub(crate) struct PublishTaskBody {
    /// 兼容旧调用; 受保护路由上 agency 一律以 JWT 为准, 此字段若存在必须一致
    pub(crate) agency_id: Option<Uuid>,
    pub(crate) brand_id: Uuid,
    pub(crate) platform: String,
    pub(crate) action: String,
    pub(crate) package_id: Uuid,
    pub(crate) package_version: u32,
    pub(crate) idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PublishTaskResponse {
    pub(crate) task_id: Uuid,
    pub(crate) route: String,
    pub(crate) status: String,
    pub(crate) external_content_id: Option<String>,
    pub(crate) receipt: Option<Value>,
    pub(crate) fallback: Option<String>,
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
    publish_core(pool, auth.agency_id, body, state.allow_mock_adapters).await
}

/// 核心发布逻辑: REST handler 与数字员工的 PublishPort 共用同一套
/// (急停/审批/路由/配额/审计), 保证员工自动发布和人工发布行为一致。
pub(crate) async fn publish_core(
    pool: &sqlx::PgPool,
    agency: AgencyId,
    body: PublishTaskBody,
    allow_mock: bool,
) -> Result<Json<PublishTaskResponse>, Response> {
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

    // 1.5 the referenced package must exist for this agency and be approved
    //     (FR: only approved packages are published; AI content needs human review first)
    let pkg_row = sqlx::query(
        "SELECT approval_status, thread_chunks FROM publishing_packages
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
            "publishing package not found for this agency",
        ))
    })?;
    let approval: String = pkg_row.get("approval_status");
    // thread: 超限内容被审核门拆成多帖, 每帖一个任务(幂等键加 :tN 后缀)
    let thread_chunks: Vec<String> = pkg_row
        .get::<serde_json::Value, _>("thread_chunks")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let thread_total = thread_chunks.len().max(1);
    if !matches!(approval.as_str(), "approved" | "auto_approved") {
        return Err(app_error_response(AppError::needs_user(
            "package_not_approved",
            format!("package approval_status is {approval}; approve it first"),
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
    let quota_kind = quota_kind_for(&body.platform, &body.action);
    let quota_row = sqlx::query(
        "SELECT qp.id AS pool_id, qp.limit_value - qp.used_value AS avail, pa.platform
         FROM quota_pools qp JOIN platform_apps pa ON pa.id = qp.platform_app_id
         WHERE pa.platform = $1 AND qp.quota_kind = $2",
    )
    .bind(&body.platform)
    .bind(quota_kind)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let (quota_pool_id, quota_avail) = match quota_row {
        Some(r) => (Some(r.get::<Uuid, _>("pool_id")), r.get::<i64, _>("avail")),
        None => (None, 0),
    };
    let cost: i64 = match quota_cost_from_matrix(pool, &body.platform, &body.action).await {
        Some(c) => c,
        None => quota_cost_for(&body.platform, &body.action),
    };
    // device_ready is dynamic, not just "a device exists": pick the healthiest
    // bound device — online (recent heartbeat) > low open-task backlog > low
    // recent failure rate — and route to it explicitly. If every device is
    // offline or drowning in WAITING_USER backlog, device route is blocked with
    // a concrete reason (评估报告 §7: 路由决策必须含设备/账号动态状态)。
    let best_device: Option<(Uuid, bool, i64, Option<f64>)> = sqlx::query(
        r#"SELECT d.id,
                  COALESCE(d.last_seen > now() - interval '120 seconds', false) AS online,
                  (SELECT count(*)::int8 FROM device_tasks t
                    WHERE t.assigned_device_id = d.id
                      AND t.state IN ('QUEUED','EXECUTING','WAITING_USER')) AS backlog,
                  (SELECT count(*) FILTER (WHERE t.state = 'FAILED')::float8
                       / NULLIF(count(*), 0)
                     FROM device_tasks t
                    WHERE t.assigned_device_id = d.id
                      AND t.created_at > now() - interval '1 hour') AS fail_rate
             FROM devices d
            WHERE d.agency_id = $1 AND d.state = 'active'
            ORDER BY online DESC, backlog ASC, fail_rate ASC NULLS FIRST, d.last_seen DESC
            LIMIT 1"#,
    )
    .bind(agency.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .map(|r| {
        use sqlx::Row;
        (
            r.get::<Uuid, _>("id"),
            r.get::<bool, _>("online"),
            r.get::<i64, _>("backlog"),
            r.get::<Option<f64>, _>("fail_rate"),
        )
    });
    let (device_ready, picked_device, device_block) = match &best_device {
        Some((id, true, _, _)) => (true, Some(*id), None),
        Some((id, false, backlog, fail_rate)) => (
            false,
            None,
            Some(format!(
                "device {id} offline (last heartbeat stale); backlog={backlog}, fail_rate={fail_rate:?}"
            )),
        ),
        None => (
            false,
            None,
            Some("no active device bound for this agency".into()),
        ),
    };
    // route-time check is coarse (any headroom); the ledger reserve is authoritative
    let pre = Preconditions {
        api_ready: quota_avail > 0,
        api_block_reason: Some(format!("quota available {quota_avail} < cost {cost}")),
        device_ready,
        device_block_reason: device_block,
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
    // Device-routed tasks are QUEUED so the gateway dispatches them to a
    // connected agent over WSS; api/assisted execute inline (EXECUTING).
    let task_state = match decision.reason.selected {
        Route::Device => "QUEUED",
        _ => "EXECUTING",
    };
    // device 路由: 显式指派给健康设备, 网关按 assigned_device_id 派发
    let assigned = if matches!(decision.reason.selected, Route::Device) {
        picked_device
    } else {
        None
    };
    for t in 0..thread_total {
        let chunk_key = if thread_total > 1 {
            format!("{}:t{t}", body.idempotency_key)
        } else {
            body.idempotency_key.clone()
        };
        let ctx = if thread_total > 1 {
            serde_json::json!({"thread_index": t, "thread_total": thread_total})
        } else {
            serde_json::Value::Null
        };
        sqlx::query(
            "INSERT INTO device_tasks (id, agency_id, brand_id, idempotency_key, state, route_id, package_id, package_version, assigned_device_id, context)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(if thread_total > 1 { Uuid::new_v4() } else { task_id })
        .bind(agency.as_uuid())
        .bind(body.brand_id)
        .bind(&chunk_key)
        .bind(task_state)
        .bind(Uuid::new_v4())
        .bind(body.package_id)
        .bind(body.package_version as i32)
        .bind(assigned)
        .bind(ctx)
        .execute(&mut *tx)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    }
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
                    let outcome = if allow_mock {
                        let adapter: Box<dyn ChannelAction> = Box::new(MockYouTubeAdapter {
                            quota_pool_id: pool_id,
                            quota_cost: cost,
                            simulate_quota_exhausted: false,
                            platform: body.platform.clone(),
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
                            // api 路线内联执行成功: 任务行落 COMPLETED
                            let _ = sqlx::query(
                                "UPDATE device_tasks SET state = 'COMPLETED', updated_at = now() WHERE id = $1",
                            )
                            .bind(task_id)
                            .execute(pool)
                            .await;
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

/// POST /tasks/{id}/retry — WAITING_USER / FAILED 的任务重新排队 (attempt+1)。
/// 用于"人工处理完挑战界面后让 agent 继续"的场景。
#[derive(Debug, Deserialize)]
struct RetryBody {
    #[serde(default)]
    force: bool,
}

async fn retry_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    body: Option<Json<RetryBody>>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let force: bool = body.as_ref().map(|b| b.force).unwrap_or(false);
    // FAILED_FINAL (side_effect 已发生) 禁止重试, 防止重复发布; force=true 仅人工显式确认
    let guard = sqlx::query_scalar::<_, bool>(
        "SELECT side_effect_done FROM device_tasks WHERE id = $1 AND agency_id = $2 AND state IN ('WAITING_USER', 'FAILED', 'FAILED_FINAL')",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if guard == Some(true) && !force {
        return Err(app_error_response(AppError::non_retryable(
            "failed_final",
            "task is FAILED_FINAL (side effect already happened); republish with a new idempotency key, or pass {\"force\": true} to explicitly override",
        )));
    }
    let row = sqlx::query(
        "UPDATE device_tasks SET state = 'QUEUED', assigned_device_id = NULL, attempt = attempt + 1, updated_at = now()
         WHERE id = $1 AND agency_id = $2 AND state IN ('WAITING_USER', 'FAILED', 'FAILED_FINAL')
         RETURNING id, state, attempt",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "not_retryable",
            "only WAITING_USER/FAILED/FAILED_FINAL tasks of this agency can be retried",
        ))
    })?;
    sqlx::query("INSERT INTO task_events (agency_id, task_id, event, from_state, to_state, detail) VALUES ($1, $2, 'task.retry', 'WAITING_USER', 'QUEUED', '{}')")
        .bind(auth.agency_id.as_uuid())
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(json!({
        "task_id": row.get::<Uuid, _>("id"),
        "state": row.get::<String, _>("state"),
        "attempt": row.get::<i32, _>("attempt"),
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
    if err.code == "forbidden" {
        status = StatusCode::FORBIDDEN;
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

/// Device-authenticated task detail for the agent: package payload + the
/// signed adapter version to execute. Device token (JWT) in ?token= — the
/// same credential the WSS gateway authenticates (WP-07).
#[derive(Debug, Deserialize)]
struct AgentTaskQuery {
    token: String,
}

async fn get_agent_task(
    State(state): State<AppState>,
    Query(q): Query<AgentTaskQuery>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = DeviceManager::new(pool, state.jwt_secret());
    let (_device_id, agency_id) = mgr.authenticate(&q.token).await.map_err(|e| {
        app_error_response(AppError::non_retryable("device_auth_failed", e.to_string()))
    })?;

    let task = sqlx::query(
        "SELECT t.id, t.state, t.package_id, t.package_version, t.context, r.platform, r.action
         FROM device_tasks t LEFT JOIN execution_routes r ON r.task_id = t.id
         WHERE t.id = $1 AND t.agency_id = $2",
    )
    .bind(id)
    .bind(agency_id.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "not_found",
            "task not found for this agency",
        ))
    })?;

    let platform: String = task.get("platform");
    let action: String = task.get("action");

    // adapter 选择: system 平台按 action 推导 (setup_install[:app] -> system.setup_install[.app]);
    // 其他平台仍由 device 路由 capability 的 constraints 指定
    let adapter_id = if platform == "system" {
        if action == "setup_install" {
            "system.setup_install".to_string()
        } else if let Some(app) = action.strip_prefix("setup_install:") {
            format!("system.setup_install.{app}")
        } else if action == "agentic" {
            "system.agentic".to_string()
        } else {
            return Err(app_error_response(AppError::non_retryable(
                "no_adapter",
                format!("未知 system action: {action}"),
            )));
        }
    } else {
        let cap = sqlx::query(
            "SELECT constraints FROM channel_capabilities
             WHERE platform = $1 AND action = $2 AND route = 'device'",
        )
        .bind(&platform)
        .bind(&action)
        .fetch_optional(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
        cap.and_then(|c| {
            c.get::<serde_json::Value, _>("constraints")["adapter"]
                .as_str()
                .map(String::from)
        })
        .ok_or_else(|| {
            app_error_response(AppError::non_retryable(
                "no_adapter",
                "device route has no signed adapter bound",
            ))
        })?
    };

    // latest active signed version of that adapter (platform-level registry)
    let av = sqlx::query(
        "SELECT v.version, v.definition, v.signature, v.signature_scheme, a.app_package
         FROM app_adapter_versions v JOIN app_adapters a ON a.id = v.adapter_id
         WHERE v.adapter_id = $1 AND v.state = 'active'
         ORDER BY v.version DESC LIMIT 1",
    )
    .bind(&adapter_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "adapter_not_registered",
            format!("adapter {adapter_id} has no active signed version"),
        ))
    })?;

    // package payload (text to post). Media stays in object storage (never in
    // the control channel) — the agent pulls it via the transfer service.
    let package_id: Option<Uuid> = task.get("package_id");
    let package_version: Option<i32> = task.get("package_version");
    // thread 任务: context.thread_index 指定取哪一帖的文本
    let thread_index: Option<usize> = task
        .try_get::<serde_json::Value, _>("context")
        .ok()
        .and_then(|v| {
            v.as_object()
                .and_then(|o| o.get("thread_index"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
        });
    let text = match (package_id, package_version) {
        (Some(pid), Some(pv)) => {
            let row = sqlx::query(
                "SELECT text_content, thread_chunks FROM publishing_packages WHERE id = $1 AND version = $2",
            )
            .bind(pid)
            .bind(pv)
            .fetch_optional(pool)
            .await
            .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
            match row {
                Some(r) => {
                    let chunks: Vec<String> = r
                        .get::<serde_json::Value, _>("thread_chunks")
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    match thread_index {
                        Some(i) if !chunks.is_empty() => chunks
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| r.get::<String, _>("text_content")),
                        _ => r.get::<String, _>("text_content"),
                    }
                }
                None => String::new(),
            }
        }
        _ => String::new(),
    };

    // the adapter travels as its canonical JSON; the device re-serializes and
    // verifies the HMAC signature before running anything.
    let definition: serde_json::Value = av.get("definition");
    let adapter_yaml = serde_json::to_string_pretty(&definition)
        .map_err(|e| app_error_response(AppError::retryable("serde", e.to_string())))?;

    // package_text 并入 task_context (数据, 非签名): 设备端验签后才把
    // flow 里的 "package.text" 占位符替换成它, 签名完整性不受影响
    let mut task_context = task
        .try_get::<serde_json::Value, _>("context")
        .unwrap_or(serde_json::Value::Null);
    if let Some(obj) = task_context.as_object_mut() {
        obj.insert(
            "package_text".into(),
            serde_json::Value::String(text.clone()),
        );
    }
    Ok(Json(json!({
        "task_id": task.get::<Uuid, _>("id"),
        "state": task.get::<String, _>("state"),
        "platform": platform,
        "action": action,
        "package_text": text,
        "app_package": av.get::<String, _>("app_package"),
        "adapter_id": adapter_id,
        "adapter_version": av.get::<i32, _>("version"),
        "adapter_yaml": adapter_yaml,
        "signature": av.get::<String, _>("signature"),
        "signature_scheme": av.get::<String, _>("signature_scheme"),
        "task_context": task_context,
    })))
}

/// Device pairing step 1: an authenticated owner starts registration; the
/// one-time device secret is returned ONCE and only its hash is stored
/// (WP-07 challenge-response). agency_id comes from the JWT only.
#[derive(Debug, Deserialize)]
struct RegisterStartBody {
    platform: String,
    name: Option<String>,
}

async fn start_device_registration(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<RegisterStartBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency_id = auth.agency_id;
    let mgr = DeviceManager::new(pool, state.jwt_secret());
    let ch = mgr
        .start_registration(agency_id, &body.platform, body.name.as_deref())
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "registration_failed",
                e.to_string(),
            ))
        })?;
    Ok(Json(json!({
        "device_id": ch.device_id,
        "nonce": ch.nonce,
        "device_secret": ch.device_secret,   // shown ONCE; only the hash is stored
    })))
}

/// Device pairing step 2: proof = HMAC-style sha256(sha256(secret) + nonce);
/// returns the long-lived device session token the agent uses on WSS/HTTP.
#[derive(Debug, Deserialize)]
struct RegisterCompleteBody {
    device_id: Uuid,
    nonce: String,
    proof: String,
}

/// POST /devices/pairing-codes (JWT) — 控制台生成一次性 4 位数字配对码, 给手机 app 输入。
#[derive(Debug, Deserialize)]
struct PairingCodeBody {
    ttl_minutes: Option<i64>,
}

async fn create_pairing_code(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<PairingCodeBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = DeviceManager::new(pool, state.jwt_secret());
    let ttl = chrono::Duration::minutes(body.ttl_minutes.unwrap_or(10).clamp(1, 60));
    let code = mgr
        .create_pairing_code(auth.agency_id, auth.member_id, ttl)
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "pairing_code_failed",
                e.to_string(),
            ))
        })?;
    Ok(Json(json!({
        "code": code,
        "expires_at": chrono::Utc::now() + ttl,
    })))
}

/// POST /devices/pair (公开) — 手机只凭配对码完成配对, 不需要账号密码。
#[derive(Debug, Deserialize)]
struct PairBody {
    code: String,
    platform: Option<String>,
    name: Option<String>,
}

async fn pair_with_code(
    State(state): State<AppState>,
    Json(body): Json<PairBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = DeviceManager::new(pool, state.jwt_secret());
    let sess = mgr
        .pair_with_code(
            &body.code,
            body.platform.as_deref().unwrap_or("android"),
            body.name.as_deref(),
        )
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable("pairing_failed", e.to_string()))
        })?;
    Ok(Json(
        json!({ "device_id": sess.device_id, "agency_id": sess.agency_id.as_uuid(), "token": sess.token }),
    ))
}

/// GET /setup/apps (JWT) — 可引导安装的应用目录 (setup_app_catalog)。
async fn list_setup_apps(
    State(state): State<AppState>,
    axum::extract::Extension(_auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT app_key, display_name, target_package, download_url, sha256 FROM setup_app_catalog WHERE active ORDER BY app_key",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "app_key": r.get::<String, _>("app_key"),
                "display_name": r.get::<String, _>("display_name"),
                "target_package": r.get::<String, _>("target_package"),
                "download_url": r.get::<String, _>("download_url"),
                "sha256": r.get::<String, _>("sha256"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// PATCH /setup/apps/{app_key} (JWT, owner) — 更新应用目录里的下载地址/SHA-256/签名证书。
/// 安装包必须来自白名单渠道: 设备端下载后会强制校验 sha256, 不匹配则零副作用失败。
#[derive(Debug, Deserialize)]
struct UpdateSetupAppBody {
    download_url: Option<String>,
    sha256: Option<String>,
    signer_cert_sha256: Option<String>,
    display_name: Option<String>,
    active: Option<bool>,
}

async fn update_setup_app(
    State(state): State<AppState>,
    Path(app_key): Path<String>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<UpdateSetupAppBody>,
) -> Result<Json<Value>, Response> {
    if !auth.roles.iter().any(|r| r == "owner") {
        return Err(app_error_response(AppError::non_retryable(
            "forbidden",
            "only owner can update the setup app catalog",
        )));
    }
    if body
        .download_url
        .as_deref()
        .is_some_and(|u| !u.starts_with("https://"))
    {
        return Err(app_error_response(AppError::non_retryable(
            "bad_url",
            "download_url must be https:// (whitelisted channel only)",
        )));
    }
    if body
        .sha256
        .as_deref()
        .is_some_and(|s| s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(app_error_response(AppError::non_retryable(
            "bad_sha256",
            "sha256 must be 64 hex chars",
        )));
    }
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query(
        "UPDATE setup_app_catalog SET
           download_url = COALESCE($2, download_url),
           sha256 = COALESCE($3, sha256),
           signer_cert_sha256 = COALESCE($4, signer_cert_sha256),
           display_name = COALESCE($5, display_name),
           active = COALESCE($6, active)
         WHERE app_key = $1
         RETURNING app_key, display_name, target_package, download_url, sha256, signer_cert_sha256, active",
    )
    .bind(&app_key)
    .bind(&body.download_url)
    .bind(&body.sha256)
    .bind(&body.signer_cert_sha256)
    .bind(&body.display_name)
    .bind(body.active)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| app_error_response(AppError::non_retryable("not_found", "app_key not in catalog")))?;
    Ok(Json(json!({
        "app_key": row.get::<String, _>("app_key"),
        "display_name": row.get::<String, _>("display_name"),
        "target_package": row.get::<String, _>("target_package"),
        "download_url": row.get::<String, _>("download_url"),
        "sha256": row.get::<String, _>("sha256"),
        "signer_cert_sha256": row.get::<Option<String>, _>("signer_cert_sha256"),
        "active": row.get::<bool, _>("active"),
    })))
}

/// POST /setup/run (JWT) — 设备引导任务: 创建一条 platform=system 的 device 路由任务,
/// 由在线 agent 通过签名 system.* adapter 原生执行 (如下载安装 X)。
/// 与内容发布同样受急停开关约束; 不需要内容包。
#[derive(Debug, Deserialize)]
struct SetupRunBody {
    action: String, // "setup_install" | "agentic"
    /// 目标应用目录 key (setup_app_catalog.app_key), 如 "x"; 缺省走通用 system.setup_install
    app: Option<String>,
    /// action=agentic 时必填: 自然语言目标 (如 "打开设置确认版本号"), 由设备端 LLM 闭环执行
    goal: Option<String>,
}

async fn run_setup(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<SetupRunBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency = auth.agency_id;
    // 急停检查 (global +租户)
    let ks = sqlx::query(
        "SELECT reason FROM kill_switches WHERE active AND (agency_id IS NULL OR agency_id = $1) LIMIT 1",
    )
    .bind(agency.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if let Some(r) = ks {
        return Err(app_error_response(AppError::needs_user(
            "kill_switch_active",
            format!(
                "被急停: {}",
                r.get::<Option<String>, _>("reason").unwrap_or_default()
            ),
        )));
    }
    // system 平台: adapter_id 由 action + app 推导 (system.setup_install[.app]),
    // 目录条目见 setup_app_catalog; 仍要求存在 device 路由 capability 兜底
    let cap = sqlx::query(
        "SELECT priority FROM channel_capabilities
         WHERE platform = 'system' AND action = 'setup_install' AND route = 'device'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "no_capability",
            "system.setup_install 没有 device 路由 capability",
        ))
    })?;
    if body.action != "setup_install" && body.action != "agentic" {
        return Err(app_error_response(AppError::non_retryable(
            "unknown_setup_action",
            format!("不支持的 setup action: {}", body.action),
        )));
    }
    if body.action == "agentic" && body.goal.as_deref().map(str::is_empty).unwrap_or(true) {
        return Err(app_error_response(AppError::non_retryable(
            "missing_goal",
            "agentic 任务必须带 goal",
        )));
    }
    let adapter_id = if body.action == "agentic" {
        "system.agentic".to_string()
    } else {
        match &body.app {
            Some(app) => format!("system.setup_install.{app}"),
            None => "system.setup_install".to_string(),
        }
    };
    // app 指定时必须在目录里登记
    if let Some(app) = &body.app {
        let row = sqlx::query("SELECT 1 FROM setup_app_catalog WHERE app_key = $1 AND active")
            .bind(app)
            .fetch_optional(pool)
            .await
            .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
        if row.is_none() {
            return Err(app_error_response(AppError::non_retryable(
                "app_not_in_catalog",
                format!("应用 {app} 未登记到 setup_app_catalog"),
            )));
        }
    }
    let av = sqlx::query(
        "SELECT v.version FROM app_adapter_versions v WHERE v.adapter_id = $1 AND v.state = 'active' ORDER BY v.version DESC LIMIT 1",
    )
    .bind(&adapter_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "adapter_not_registered",
            format!("adapter {adapter_id} 无 active 签名版本"),
        ))
    })?;

    let task_id = Uuid::new_v4();
    let route_id = Uuid::new_v4();
    let route_action = match &body.app {
        Some(app) if body.action == "setup_install" => format!("setup_install:{app}"),
        _ => body.action.clone(),
    };
    // agentic 任务把目标 app 的包名放进 context, 设备端 launch_app 恢复前台时用
    let catalog_pkg = match &body.app {
        Some(app) => sqlx::query(
            "SELECT target_package FROM setup_app_catalog WHERE app_key = $1 AND active",
        )
        .bind(app)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<String, _>("target_package").ok()),
        None => None,
    };
    let task_context = if body.action == "agentic" {
        let mut ctx = serde_json::json!({ "goal": body.goal.clone().unwrap_or_default() });
        if let Some(pkg) = catalog_pkg {
            ctx["app_package"] = serde_json::Value::String(pkg);
        }
        ctx
    } else {
        serde_json::json!({})
    };
    let idem = format!("setup-{}-{}", route_action, task_id);
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO execution_routes (id, agency_id, task_id, platform, action, selected_route, capability_status, priority)
         VALUES ($1, $2, $3, 'system', $4, 'device', 'verified', $5)",
    )
    .bind(route_id)
    .bind(agency.as_uuid())
    .bind(task_id)
    .bind(&route_action)
    .bind(cap.get::<i32, _>("priority"))
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO device_tasks (id, agency_id, idempotency_key, state, route_id, adapter_version, context)
         VALUES ($1, $2, $3, 'QUEUED', $4, $5, $6)",
    )
    .bind(task_id)
    .bind(agency.as_uuid())
    .bind(idem)
    .bind(route_id)
    .bind(av.get::<i32, _>("version"))
    .bind(&task_context)
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    tx.commit()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(
        json!({ "task_id": task_id, "action": route_action, "adapter": adapter_id, "state": "QUEUED", "route": "device" }),
    ))
}

async fn complete_device_registration(
    State(state): State<AppState>,
    Json(body): Json<RegisterCompleteBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = DeviceManager::new(pool, state.jwt_secret());
    let session = mgr
        .complete_registration(body.device_id, &body.nonce, &body.proof)
        .await
        .map_err(|e| {
            app_error_response(AppError::non_retryable(
                "registration_failed",
                e.to_string(),
            ))
        })?;
    Ok(Json(
        json!({ "device_id": body.device_id, "token": session.token }),
    ))
}

#[derive(Debug, Deserialize)]
struct ReplyAutoBody {
    /// 目标应用目录 key, 如 "x"
    app: Option<String>,
    /// 找帖主题 (来自 opportunity / 手工输入)
    topic: String,
    brand_id: Uuid,
}

/// POST /setup/reply_auto (JWT) — 自动回帖流水线:
/// ① agentic 找帖 (X 搜索主题 → 打开最相关帖子详情页 → done 时把帖子全文带回 ANSWER)
/// ② 云端 LLM 读帖子全文 + 品牌语气 生成人类口吻回复 (回帖免人工审核, package 直接 approved)
/// ③ 签名 adapter x.reply_open_post 在已打开的详情页上发送回复
/// 立即返回 find_task_id, 后台异步跑全程; 最终发布任务出现在 tasks 列表。
async fn reply_auto(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<ReplyAutoBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let agency = auth.agency_id;
    let topic = body.topic.trim().to_string();
    if topic.is_empty() || topic.chars().count() > 120 {
        return Err(app_error_response(AppError::non_retryable(
            "bad_topic",
            "topic required, <=120 chars",
        )));
    }
    // brand 归属检查
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM brands WHERE id = $1 AND agency_id = $2)",
    )
    .bind(body.brand_id)
    .bind(agency.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if !owns {
        return Err(app_error_response(AppError::non_retryable(
            "brand_not_found",
            "brand does not belong to this agency",
        )));
    }
    // 急停检查
    let ks = sqlx::query(
        "SELECT reason FROM kill_switches WHERE active AND (agency_id IS NULL OR agency_id = $1) LIMIT 1",
    )
    .bind(agency.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if let Some(r) = ks {
        return Err(app_error_response(AppError::needs_user(
            "kill_switch_active",
            format!(
                "被急停: {}",
                r.get::<Option<String>, _>("reason").unwrap_or_default()
            ),
        )));
    }
    let find_task_id = create_agentic_find_task(pool, agency, &body.app, &topic).await?;
    let pool2 = pool.clone();
    let agency2 = agency;
    let brand2 = body.brand_id;
    let topic2 = topic.clone();
    let allow_mock = state.allow_mock_adapters;
    tokio::spawn(async move {
        if let Err(e) =
            run_reply_auto(pool2, agency2, brand2, topic2, find_task_id, allow_mock).await
        {
            tracing::warn!("reply_auto pipeline failed: {e}");
        }
    });
    Ok(Json(json!({
        "find_task_id": find_task_id,
        "topic": topic,
        "state": "RUNNING",
        "note": "找帖 agentic 任务已下发; 完成后自动生成回复并通过 x.reply_open_post 发布, 最终任务见 tasks 列表",
    })))
}

/// 创建 system.agentic 找帖任务 (与 run_setup 的 agentic 分支同构)
async fn create_agentic_find_task(
    pool: &sqlx::PgPool,
    agency: AgencyId,
    app: &Option<String>,
    topic: &str,
) -> Result<Uuid, Response> {
    let task_id = Uuid::new_v4();
    let route_id = Uuid::new_v4();
    let cap = sqlx::query(
        "SELECT priority FROM channel_capabilities
         WHERE platform = 'system' AND action = 'setup_install' AND route = 'device'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "no_capability",
            "system.setup_install 没有 device 路由 capability",
        ))
    })?;
    let av = sqlx::query(
        "SELECT v.version FROM app_adapter_versions v WHERE v.adapter_id = 'system.agentic' AND v.state = 'active' ORDER BY v.version DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "no_adapter",
            "system.agentic 没有 active 版本",
        ))
    })?;
    let catalog_pkg = match app {
        Some(a) => sqlx::query(
            "SELECT target_package FROM setup_app_catalog WHERE app_key = $1 AND active",
        )
        .bind(a)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<String, _>("target_package").ok()),
        None => None,
    };
    let goal = format!(
        "Find ONE post on X (Twitter) about this topic: \"{topic}\"\n\
         Steps: tap the search magnifier icon at the top-right of the Home screen (tap_xy around 851,99). \
         Type \"{topic}\" into the search box and submit. In the results, prefer the 最新/Latest or 帖子/Top tab. \
         Scan results (screenshot + text) and tap the TEXT of the first clearly relevant post to open its detail page. \
         Do NOT tap ads. Do NOT reply, post, like, repost, or follow anyone — this task only FINDS and OPENS the post.\n\
         When the post detail page is open, reply {{\"action\":\"done\",\"answer\":\"AUTHOR: <author handle>\\nPOST: <the full post text copied exactly>\"}}.\n\
         If after ~20 steps no relevant post is found, reply {{\"action\":\"done\",\"answer\":\"NOTFOUND\"}}. Max 30 steps."
    );
    let mut ctx = serde_json::json!({ "goal": goal });
    if let Some(pkg) = catalog_pkg {
        ctx["app_package"] = serde_json::Value::String(pkg);
    }
    let idem = format!("reply-auto-find-{task_id}");
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO execution_routes (id, agency_id, task_id, platform, action, selected_route, capability_status, priority)
         VALUES ($1, $2, $3, 'system', 'agentic', 'device', 'verified', $4)",
    )
    .bind(route_id)
    .bind(agency.as_uuid())
    .bind(task_id)
    .bind(cap.get::<i32, _>("priority"))
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    sqlx::query(
        "INSERT INTO device_tasks (id, agency_id, idempotency_key, state, route_id, adapter_version, context)
         VALUES ($1, $2, $3, 'QUEUED', $4, $5, $6)",
    )
    .bind(task_id)
    .bind(agency.as_uuid())
    .bind(idem)
    .bind(route_id)
    .bind(av.get::<i32, _>("version"))
    .bind(&ctx)
    .execute(&mut *tx)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    tx.commit()
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(task_id)
}

/// 后台流水线: 等找帖任务结束 → 取帖子全文 → LLM 生成人类口吻回复 → approved package → 签名发布
const REPLY_AUTO_TERMINAL: [&str; 5] = [
    "COMPLETED",
    "FAILED",
    "VERIFICATIONPENDING",
    "WAITING_USER",
    "CANCELLED",
];

async fn run_reply_auto(
    pool: sqlx::PgPool,
    agency: AgencyId,
    brand_id: Uuid,
    topic: String,
    find_task_id: Uuid,
    allow_mock: bool,
) -> Result<(), String> {
    // 1. 等找帖任务终态 (最多 240s)
    let mut final_state = String::new();
    for _ in 0..48 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let st: Option<String> = sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
            .bind(find_task_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(s) = st
            && REPLY_AUTO_TERMINAL.contains(&s.as_str())
        {
            final_state = s;
            break;
        }
    }
    if final_state != "COMPLETED" {
        return Err(format!("find task ended as {final_state}"));
    }
    // 2. 取帖子全文 (ANSWER 事件)
    let ev: Option<String> = sqlx::query_scalar(
        "SELECT detail::text FROM task_events WHERE task_id = $1 AND detail::text LIKE '%ANSWER: %' ORDER BY id DESC LIMIT 1",
    )
    .bind(find_task_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let raw = ev.ok_or("no ANSWER event from find task")?;
    let answer = raw
        .split("ANSWER: ")
        .last()
        .unwrap_or("")
        .trim_matches('"')
        .trim()
        .to_string();
    if answer.is_empty() || answer == "NOTFOUND" {
        return Err("find task found no relevant post".into());
    }
    tracing::info!(task = %find_task_id, "reply_auto found post: {}", &answer[..answer.len().min(200)]);

    // 3. 品牌语气 + no_go
    let brand = sqlx::query("SELECT name, language, config FROM brands WHERE id = $1")
        .bind(brand_id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("brand not found")?;
    let bname: String = brand.get("name");
    let blang: Option<String> = brand.get("language");
    let bcfg: serde_json::Value = brand.get::<serde_json::Value, _>("config");
    let voice = bcfg
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("casual, human");
    let audience = bcfg
        .get("audience")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let no_go = bcfg
        .get("no_go")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let no_go_hit = no_go.iter().any(|ng| {
        let ng = ng.to_lowercase();
        topic.to_lowercase().contains(&ng) || answer.to_lowercase().contains(&ng)
    });
    if no_go_hit {
        return Err(format!("topic/post hits brand no_go list: {no_go:?}"));
    }

    // 4. LLM 生成人类口吻回复
    let blang = blang.unwrap_or_else(|| "en".into());
    let reply_prompt = format!(
        "You are the social media account \"{bname}\" (language: {blang}). Voice: {voice}. Audience: {audience}.\n\
         Topic we care about: {topic}\n\n\
         Here is a real post on X that we want to reply to:\n\"\"\"\n{answer}\n\"\"\"\n\n\
         Write a SHORT reply (1-2 sentences, max 240 characters) that a real human would write in the comments. \
         Requirements: sound like a genuine person, not marketing; reference a SPECIFIC detail from the post above; \
         agree, add a sharp observation, or ask a real question; NO hashtags, NO \"Great post!\", NO self-promotion, NO links. \
         ALWAYS reply in the account's configured language: {blang}.\n\
         Output ONLY the reply text, nothing else."
    );
    let gw = sincy_ai::gateway_from_env(allow_mock).map_err(|e| e.to_string())?;
    let req = sincy_ai::GenerationRequest {
        prompt: reply_prompt,
        model: std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "mistral-medium-latest".into()),
        max_tokens: 192,
        content_item_id: None,
        images: vec![],
    };
    let gen_out = gw.generate(&req).await.map_err(|e| e.to_string())?;
    let reply_text = gen_out.text.trim().trim_matches('"').to_string();
    if reply_text.is_empty() || reply_text.chars().count() > 280 {
        return Err(format!(
            "generated reply invalid (len {}): {reply_text}",
            reply_text.chars().count()
        ));
    }
    tracing::info!(task = %find_task_id, "reply_auto generated reply: {reply_text}");

    // 5. 自动 approved package (回帖免人工审核) + 签名发布
    let pid = insert_package(
        &pool,
        agency,
        NewPackage {
            brand_id,
            platform: "x",
            action: "reply_open_post",
            text: &reply_text,
            source: "agent_reply",
            provenance: serde_json::json!({
                "origin": "reply_auto_pipeline",
                "topic": topic,
                "find_task_id": find_task_id,
                "target_post": &answer[..answer.len().min(500)],
            }),
            approval_status: "approved",
        },
    )
    .await
    .map_err(|e| format!("insert_package: {e:?}"))?;
    let pub_out = publish_core(
        &pool,
        agency,
        PublishTaskBody {
            agency_id: None,
            brand_id,
            platform: "x".into(),
            action: "reply_open_post".into(),
            package_id: pid,
            package_version: 1,
            idempotency_key: format!("reply-auto-{find_task_id}"),
        },
        allow_mock,
    )
    .await
    .map_err(|e| format!("publish_core: {e:?}"))?;
    tracing::info!(task = %find_task_id, package = %pid, publish_task = %pub_out.task_id, "reply_auto published");
    Ok(())
}

/// POST /devices/{id}/revoke — 撤销设备: state=revoked + 所有会话撤销, token 立即失效。
async fn revoke_device(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = sincy_device::DeviceManager::new(pool, state.jwt_secret());
    mgr.revoke(auth.agency_id, id)
        .await
        .map_err(|e| app_error_response(AppError::non_retryable("revoke_failed", e.to_string())))?;
    Ok(Json(json!({ "device_id": id, "state": "revoked" })))
}

/// POST /devices/{id}/rotate — 轮换设备 token: 旧会话全部撤销, 返回新 token (60 天有效)。
async fn rotate_device(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let mgr = sincy_device::DeviceManager::new(pool, state.jwt_secret());
    let st = mgr
        .rotate(auth.agency_id, id)
        .await
        .map_err(|e| app_error_response(AppError::non_retryable("rotate_failed", e.to_string())))?;
    Ok(Json(json!({
        "device_id": st.device_id,
        "token": st.token,
        "agency_id": st.agency_id,
        "expires_in_days": 60,
    })))
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
    /// 生成后直接保存为发布包（内容来源: AI 生成页 → 内容包 → 发布）
    save_as_package: Option<SaveAsPackage>,
    /// base64 图片 (jpeg/png), 多模态/视觉请求 — 设备 agent 每步截图走这里
    images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SaveAsPackage {
    brand_id: Uuid,
    platform: String,
    action: String,
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
    // 估算成本: 约 $0.002/1k tokens (保守估计, 按 max_tokens 全额计) + 每张图约 $0.0005 (视觉 token)
    let images = body.images.unwrap_or_default();
    if images.len() > 4 {
        return Err(app_error_response(AppError::non_retryable(
            "too_many_images",
            "at most 4 images per request",
        )));
    }
    for img in &images {
        // base64 解码后不超过 2MB, 且必须是合法 base64
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(img)
            .map_err(|e| {
                app_error_response(AppError::non_retryable(
                    "bad_image",
                    format!("invalid base64: {e}"),
                ))
            })?;
        if decoded.len() > 2 * 1024 * 1024 {
            return Err(app_error_response(AppError::non_retryable(
                "image_too_large",
                "each image must decode to <= 2MB",
            )));
        }
    }
    let max_tokens = body.max_tokens.unwrap_or(1024).max(1);
    let est_usd = max_tokens as f64 / 1000.0 * 0.002 + images.len() as f64 * 0.0005;
    let est = rust_decimal::Decimal::try_from(est_usd).unwrap_or(rust_decimal::Decimal::ZERO);
    if let Err(e) = checker.check(auth.agency_id, None, est).await {
        crate::metrics::Metrics::inc(&crate::metrics::metrics().budget_blocked_total);
        return Err(app_error_response(AppError::needs_user(
            "budget_exceeded",
            e.to_string(),
        )));
    }
    let req = GenerationRequest {
        prompt: body.prompt,
        model: body.model.unwrap_or_else(|| {
            std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "mock-1".into())
        }),
        max_tokens,
        content_item_id: body.content_item_id.map(|u| u.to_string()),
        images,
    };
    let gw = sincy_ai::gateway_from_env(state.allow_mock_adapters)
        .map_err(|e| app_error_response(AppError::non_retryable("llm_provider", e)))?;
    let result = gw
        .generate(&req)
        .await
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
                    result.provider.as_str()
                },
                model: &result.model,
                kind: "llm",
                tokens_in: result.tokens_in as i64,
                tokens_out: result.tokens_out as i64,
                cost_usd: est,
            },
        )
        .await;
    // AI 生成内容落为内容包: 带 provenance (FR-024), 状态 pending 待人工审核
    let mut package_id: Option<Uuid> = None;
    if let Some(save) = &body.save_as_package {
        let pid = insert_package(
            pool,
            auth.agency_id,
            NewPackage {
                brand_id: save.brand_id,
                platform: &save.platform,
                action: &save.action,
                text: &result.text,
                source: "ai",
                provenance: json!({"model": result.model, "tools": [format!("sincy-ai/{}", result.model)], "mock": result.mock}),
                approval_status: "pending",
            },
        )
        .await?;
        package_id = Some(pid);
    }
    Ok(Json(json!({
        "text": result.text,
        "model": result.model,
        "tokens_in": result.tokens_in,
        "tokens_out": result.tokens_out,
        "mock": result.mock,
        "estimated_cost_usd": est.to_string(),
        "package_id": package_id,
    })))
}

/// Shared insert input for manual + AI-sourced publishing packages.
pub(crate) struct NewPackage<'a> {
    pub(crate) brand_id: Uuid,
    pub(crate) platform: &'a str,
    pub(crate) action: &'a str,
    pub(crate) text: &'a str,
    /// "manual" (直接 approved) | "ai" (pending 人工审核, FR-024)
    pub(crate) source: &'a str,
    pub(crate) provenance: serde_json::Value,
    pub(crate) approval_status: &'a str,
}

/// Shared insert for manual + AI-sourced publishing packages.
pub(crate) async fn insert_package(
    pool: &sqlx::PgPool,
    agency: AgencyId,
    pkg: NewPackage<'_>,
) -> Result<Uuid, Response> {
    let NewPackage {
        brand_id,
        platform,
        action,
        text,
        source,
        provenance,
        approval_status,
    } = pkg;
    // brand must belong to this agency (防越权)
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM brands WHERE id = $1 AND agency_id = $2)",
    )
    .bind(brand_id)
    .bind(agency.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if !owns {
        return Err(app_error_response(AppError::non_retryable(
            "brand_not_found",
            "brand does not belong to this agency",
        )));
    }
    // platform limits (X 280 chars) enforced at content time, not publish time
    let max_chars = match platform {
        "x" => 280,
        _ => 40_000,
    };
    if text.chars().count() > max_chars {
        return Err(app_error_response(AppError::needs_user(
            "text_too_long",
            format!(
                "text is {} chars, platform {platform} allows {max_chars}",
                text.chars().count()
            ),
        )));
    }
    let pid = Uuid::new_v4();
    // Sincy-04: 内容按所属品牌的行业画像打 tag
    let profile: Option<Value> =
        sqlx::query_scalar("SELECT profile FROM brand_intel_profiles WHERE brand_id = $1")
            .bind(brand_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let p = profile
        .map(|v| sincy_intel::industry::CustomerProfile::from_json(&v))
        .unwrap_or_default();
    let industry_tags = p.industries();
    sqlx::query(
        "INSERT INTO publishing_packages
         (id, version, agency_id, brand_id, platform, account_id, action, text_content, constraints, approval_status, industry_tags)
         VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(pid)
    .bind(agency.as_uuid())
    .bind(brand_id)
    .bind(platform)
    .bind(brand_id) // account_id: 默认绑定品牌账号 (社交账号绑定 WP 后续细化)
    .bind(action)
    .bind(text)
    .bind(json!({"source": source, "provenance": provenance}))
    .bind(approval_status)
    .bind(serde_json::to_value(&industry_tags).unwrap_or(json!([])))
    .execute(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(pid)
}

/// GET /api/v1/brands — 当前机构的品牌列表 (发布表单下拉框的数据源).
async fn list_brands(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query("SELECT id, name, client_id, language, created_at FROM brands WHERE agency_id = $1 ORDER BY created_at DESC")
        .bind(auth.agency_id.as_uuid())
        .fetch_all(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "client_id": r.get::<Uuid, _>("client_id"),
                "language": r.get::<String, _>("language"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// GET /api/v1/clients — 客户列表 (品牌归属客户).
async fn list_clients(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query("SELECT id, name, status, created_at FROM clients WHERE agency_id = $1 ORDER BY created_at DESC")
        .bind(auth.agency_id.as_uuid())
        .fetch_all(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<Uuid, _>("id"),
                "name": r.get::<String, _>("name"),
                "status": r.get::<String, _>("status"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct CreateClientBody {
    name: String,
}

/// POST /api/v1/clients — 创建客户 (正式注册路径, 非演示).
async fn create_client(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<CreateClientBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(auth.agency_id.as_uuid())
        .bind(&body.name)
        .execute(pool)
        .await
        .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(
        json!({ "id": id, "name": body.name, "status": "active" }),
    ))
}

/// Allowed output languages (BCP-47 subset); brand default is zh-CN (migration 0020).
pub(crate) const ALLOWED_LANGUAGES: [&str; 9] = [
    "zh-CN", "zh-TW", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "pt-BR",
];

pub(crate) fn validate_language(lang: &str) -> Result<(), AppError> {
    if ALLOWED_LANGUAGES.contains(&lang) {
        Ok(())
    } else {
        Err(AppError::non_retryable(
            "invalid_language",
            format!("language must be one of {ALLOWED_LANGUAGES:?}"),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct CreateBrandBody {
    client_id: Uuid,
    name: String,
    /// 品牌调性/受众/禁区/合规规则 (FR-002)
    config: Option<Value>,
    /// 品牌内容输出语言 (BCP-47), 默认 zh-CN
    language: Option<String>,
}

/// POST /api/v1/brands — 创建品牌, 归属某客户; 数字员工挂载在品牌上.
async fn create_brand(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<CreateBrandBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM clients WHERE id = $1 AND agency_id = $2)",
    )
    .bind(body.client_id)
    .bind(auth.agency_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if !owns {
        return Err(app_error_response(AppError::non_retryable(
            "client_not_found",
            "client does not belong to this agency",
        )));
    }
    let language = body.language.unwrap_or_else(|| "zh-CN".into());
    validate_language(&language).map_err(app_error_response)?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO brands (id, agency_id, client_id, name, config, language) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(body.client_id)
    .bind(&body.name)
    .bind(body.config.unwrap_or_else(|| json!({})))
    .bind(&language)
    .execute(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    Ok(Json(
        json!({ "id": id, "name": body.name, "client_id": body.client_id, "language": language }),
    ))
}

#[derive(Debug, Deserialize)]
struct UpdateBrandBody {
    name: Option<String>,
    config: Option<Value>,
    /// 修改品牌内容输出语言 (BCP-47)
    language: Option<String>,
}

/// PATCH /api/v1/brands/{id} — 更新品牌名称/配置/输出语言.
async fn update_brand(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<UpdateBrandBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    if let Some(l) = &body.language {
        validate_language(l).map_err(app_error_response)?;
    }
    let row = sqlx::query(
        "UPDATE brands SET
           name = COALESCE($3, name),
           config = COALESCE($4, config),
           language = COALESCE($5, language)
         WHERE id = $1 AND agency_id = $2
         RETURNING id, name, client_id, language, created_at",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(&body.name)
    .bind(&body.config)
    .bind(&body.language)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let Some(r) = row else {
        return Err(app_error_response(AppError::non_retryable(
            "brand_not_found",
            "brand does not belong to this agency",
        )));
    };
    Ok(Json(json!({
        "id": r.get::<Uuid, _>("id"),
        "name": r.get::<String, _>("name"),
        "client_id": r.get::<Uuid, _>("client_id"),
        "language": r.get::<String, _>("language"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })))
}

#[derive(Debug, Deserialize)]
struct PackagesQuery {
    platform: Option<String>,
    action: Option<String>,
}

/// GET /api/v1/packages?platform=x&action=publish_post — 内容包列表 (X 内容的来源).
async fn list_packages(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Query(q): Query<PackagesQuery>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT id, version, brand_id, platform, action, text_content, constraints, approval_status, created_at
         FROM publishing_packages WHERE agency_id = $1
           AND ($2::text IS NULL OR platform = $2)
           AND ($3::text IS NULL OR action = $3)
         ORDER BY created_at DESC LIMIT 200",
    )
    .bind(auth.agency_id.as_uuid())
    .bind(&q.platform)
    .bind(&q.action)
    .fetch_all(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let text: String = r.get("text_content");
            let n = text.chars().count().min(120);
            json!({
                "id": r.get::<Uuid, _>("id"),
                "version": r.get::<i32, _>("version"),
                "brand_id": r.get::<Uuid, _>("brand_id"),
                "platform": r.get::<String, _>("platform"),
                "action": r.get::<String, _>("action"),
                "text_preview": text.chars().take(n).collect::<String>(),
                "text_content": text,
                "constraints": r.get::<serde_json::Value, _>("constraints"),
                "approval_status": r.get::<String, _>("approval_status"),
                "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// POST /api/v1/packages — 手工创建内容包 (manual 直接 approved).
#[derive(Debug, Deserialize)]
struct CreatePackageBody {
    brand_id: Uuid,
    platform: String,
    action: String,
    text: String,
}

async fn create_package(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<CreatePackageBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let pid = insert_package(
        pool,
        auth.agency_id,
        NewPackage {
            brand_id: body.brand_id,
            platform: &body.platform,
            action: &body.action,
            text: &body.text,
            source: "manual",
            provenance: json!({}),
            approval_status: "approved",
        },
    )
    .await?;
    Ok(Json(
        json!({ "package_id": pid, "version": 1, "approval_status": "approved" }),
    ))
}

/// POST /api/v1/packages/{id}/approve — 人工审核通过 AI 生成的内容包.
async fn approve_package(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let upd = sqlx::query(
        "UPDATE publishing_packages SET approval_status = 'approved'
         WHERE id = $1 AND agency_id = $2 AND approval_status = 'pending'",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .execute(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?;
    if upd.rows_affected() == 0 {
        return Err(app_error_response(AppError::needs_user(
            "not_pending",
            "package not found or not pending approval",
        )));
    }
    Ok(Json(json!({ "id": id, "approval_status": "approved" })))
}

/// platform+action -> quota kind (FR-020 共享配额池按种类记账)
fn quota_kind_for(platform: &str, action: &str) -> &'static str {
    match (platform, action) {
        ("youtube", "publish_video") => "videos.insert",
        ("x", "publish_post") => "tweets.create",
        ("x", "read_metrics") => "tweets.read",
        ("youtube", "read_metrics") => "youtube.read",
        _ => "generic_units",
    }
}

/// Fallback quota cost, used only when the capability row carries no
/// `constraints.quota_cost`. The authoritative cost is read from the matrix by
/// `quota_cost_from_matrix`; keep these defaults in step with current platform
/// docs (youtube videos.insert ≈ 100 units since the 2026-12-04 revision).
fn quota_cost_for(platform: &str, action: &str) -> i64 {
    match (platform, action) {
        ("youtube", "publish_video") => 100,
        ("x", "publish_post") => 1,
        _ => 1,
    }
}

/// Authoritative quota cost from the capability matrix: the `api`-route
/// capability's `constraints.quota_cost` for this (platform, action). Returns
/// None when there is no api capability or it declares no quota_cost, so the
/// caller falls back to `quota_cost_for`. Reading it from the DB keeps the cost
/// a single source of truth that ops can correct without a code release.
async fn quota_cost_from_matrix(pool: &sqlx::PgPool, platform: &str, action: &str) -> Option<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT (constraints->>'quota_cost')::int8
           FROM channel_capabilities
          WHERE platform = $1 AND action = $2 AND route = 'api'
            AND (constraints ? 'quota_cost')
          ORDER BY priority ASC
          LIMIT 1",
    )
    .bind(platform)
    .bind(action)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
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

    if body.platform == "x" {
        let mut x = sincy_platform::XAdapter::for_account(pool, agency.as_uuid(), &account_id)
            .await
            .map_err(app_error_response)?;
        x.text = text;
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
            x.media = Some(MediaBlob {
                bytes,
                mime: asset.get("mime"),
                filename: asset_id.to_string(),
            });
        }
        let _ = pkg;
        return Ok(Box::new(x));
    }
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
