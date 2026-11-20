//! Digital Employee REST surface (DE guide §18) + PG-backed ports for the
//! operating cycle (DE §5.2). Employee actions always go through the SAME
//! publish core as manual publishing (publish_core) — the employee never
//! bypasses kill switches, approval gates, routing, or quota.

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sincy_employee::agents::AgentKind;
use sincy_employee::cycle::{
    CycleDeps, CycleStep, CycleStore, IntelPort, LlmPort, PublishPort, PublishReceipt, run_cycle,
};
use sincy_employee::domain::{
    Employee, EmployeeStatus, Identity, Mission, Strategy, default_language,
};
use sincy_employee::lifecycle::transition;
use sincy_employee::memory::MemoryItem;
use sincy_intel::adapter::EmployeeContext;
use sincy_intel::model::Opportunity;
use sqlx::Row;
use uuid::Uuid;

use crate::api::{NewPackage, PublishTaskBody, app_error_response, insert_package, publish_core};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/employees", get(list_employees).post(create_employee))
        .route("/employees/{id}", get(get_employee).patch(patch_employee))
        .route("/employees/{id}/start", post(transition_employee))
        .route("/employees/{id}/pause", post(transition_employee))
        .route("/employees/{id}/resume", post(transition_employee))
        .route("/employees/{id}/stop", post(transition_employee))
        .route("/employees/{id}/goals", get(list_goals).post(create_goal))
        .route("/employees/{id}/strategy", get(get_strategy))
        .route(
            "/employees/{id}/strategy/versions",
            post(create_strategy_version),
        )
        .route("/employees/{id}/cycles", get(list_cycles).post(start_cycle))
        .route("/cycles/{id}", get(get_cycle))
        .route("/employees/{id}/memory", get(list_memory))
        .route("/employees/{id}/insights", get(list_insights))
}

// ---------- helpers ----------

fn err(e: sqlx::Error) -> axum::response::Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

fn row_to_employee(r: &sqlx::postgres::PgRow) -> Employee {
    Employee {
        id: r.get("id"),
        agency_id: r.get("agency_id"),
        name: r.get("name"),
        role: r.get("role"),
        status: EmployeeStatus::parse(&r.get::<String, _>("status"))
            .unwrap_or(EmployeeStatus::Created),
        config_version: r.get("config_version"),
        brand_id: r.get("brand_id"),
        channel: r.get("channel"),
        auto_publish: r.get("auto_publish"),
        created_at: r.get("created_at"),
    }
}

async fn load_full(
    pool: &sqlx::PgPool,
    id: Uuid,
) -> Result<(Employee, Identity, Mission, Strategy), axum::response::Response> {
    let r = sqlx::query("SELECT * FROM digital_employees WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "employee not found",
            ))
        })?;
    let emp = row_to_employee(&r);
    let ir = sqlx::query(
        "SELECT version, identity FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    let identity: Identity = match ir {
        Some(r) => serde_json::from_value(r.get::<Value, _>("identity")).map_err(|e| {
            app_error_response(sincy_common::AppError::retryable("serde", e.to_string()))
        })?,
        None => Identity {
            version: 1,
            name: emp.name.clone(),
            role: emp.role.clone(),
            personality: json!({}),
            voice: "neutral".into(),
            language: default_language(),
            audience: vec![],
            expertise: vec![],
            values: vec![],
            boundaries: vec![],
        },
    };
    let mr = sqlx::query("SELECT mission FROM employee_missions WHERE employee_id = $1 AND active ORDER BY version DESC LIMIT 1")
        .bind(id).fetch_optional(pool).await.map_err(err)?;
    let mission: Mission = match mr {
        Some(r) => serde_json::from_value(r.get::<Value, _>("mission")).map_err(|e| {
            app_error_response(sincy_common::AppError::retryable("serde", e.to_string()))
        })?,
        None => Mission {
            statement: format!("Operate {} channel for the brand", emp.channel),
            domain: "content".into(),
            audience: identity.audience.clone(),
            value_proposition: "consistent, on-brand content".into(),
            constraints: vec![],
            success_definition: "engagement and reach growth".into(),
        },
    };
    let sr = sqlx::query("SELECT version, status, strategy FROM employee_strategies WHERE employee_id = $1 AND status = 'ACTIVE' ORDER BY version DESC LIMIT 1")
        .bind(id).fetch_optional(pool).await.map_err(err)?;
    let strategy: Strategy = match sr {
        Some(r) => {
            let mut s: Strategy =
                serde_json::from_value(r.get::<Value, _>("strategy")).map_err(|e| {
                    app_error_response(sincy_common::AppError::retryable("serde", e.to_string()))
                })?;
            s.version = r.get("version");
            s
        }
        None => Strategy::default(),
    };
    Ok((emp, identity, mission, strategy))
}

// ---------- employee CRUD + lifecycle (DE §18) ----------

#[derive(Debug, Deserialize)]
struct CreateEmployeeBody {
    name: String,
    role: Option<String>,
    brand_id: Option<Uuid>,
    channel: Option<String>,
    auto_publish: Option<bool>,
    identity: Option<Value>,
    mission: Option<Value>,
    strategy: Option<Value>,
}

async fn create_employee(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Json(body): Json<CreateEmployeeBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    // brand language becomes the employee's default output language (identity.language)
    let mut brand_language: Option<String> = None;
    if let Some(b) = body.brand_id {
        let row = sqlx::query("SELECT EXISTS(SELECT 1 FROM brands WHERE id = $1 AND agency_id = $2) AS owns, language FROM brands WHERE id = $1 AND agency_id = $2")
            .bind(b)
            .bind(auth.agency_id.as_uuid())
            .fetch_optional(pool)
            .await
            .map_err(err)?;
        match row {
            Some(r) if r.get::<bool, _>("owns") => {
                brand_language = r.try_get::<String, _>("language").ok();
            }
            _ => {
                return Err(app_error_response(sincy_common::AppError::non_retryable(
                    "brand_not_found",
                    "brand not owned by agency",
                )));
            }
        }
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO digital_employees (id, agency_id, name, role, brand_id, channel, auto_publish, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'READY')",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(&body.name)
    .bind(body.role.clone().unwrap_or_else(|| "content_operator".into()))
    .bind(body.brand_id)
    .bind(body.channel.clone().unwrap_or_else(|| "x".into()))
    .bind(body.auto_publish.unwrap_or(false))
    .execute(pool)
    .await
    .map_err(err)?;
    // identity version 1 — language defaults to the brand's language, then zh-CN
    let mut identity = body.identity.clone().unwrap_or_else(|| json!({
        "name": body.name, "role": body.role.clone().unwrap_or_else(|| "content_operator".into()),
        "personality": {}, "voice": "neutral", "audience": [], "expertise": [], "values": [], "boundaries": []
    }));
    if identity.get("language").and_then(|v| v.as_str()).is_none() {
        identity["language"] = json!(brand_language.unwrap_or_else(default_language));
    }
    sqlx::query("INSERT INTO employee_identity_versions (employee_id, version, identity, status) VALUES ($1, 1, $2, 'active')")
        .bind(id).bind(&identity).execute(pool).await.map_err(err)?;
    let mission = body.mission.clone().unwrap_or_else(|| json!({
        "statement": format!("Operate the {} channel", body.channel.clone().unwrap_or_else(|| "x".into())),
        "domain": "content", "audience": [], "value_proposition": "consistent on-brand content",
        "constraints": [], "success_definition": "engagement growth"
    }));
    sqlx::query("INSERT INTO employee_missions (employee_id, mission) VALUES ($1, $2)")
        .bind(id)
        .bind(&mission)
        .execute(pool)
        .await
        .map_err(err)?;
    let strategy = body.strategy.clone().unwrap_or_else(|| json!({
        "pillars": [], "cadence_per_week": 7, "formats": ["text"], "risk_tolerance": 0.3, "exploration_budget": 0.2
    }));
    sqlx::query("INSERT INTO employee_strategies (employee_id, version, strategy, status) VALUES ($1, 1, $2, 'ACTIVE')")
        .bind(id).bind(&strategy).execute(pool).await.map_err(err)?;
    Ok(Json(json!({ "id": id, "status": "READY" })))
}

async fn list_employees(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT e.*, (SELECT v.identity->>'language' FROM employee_identity_versions v
                      WHERE v.employee_id = e.id AND v.status = 'active' ORDER BY v.version DESC LIMIT 1) AS language
         FROM digital_employees e WHERE e.agency_id = $1 ORDER BY e.created_at DESC",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| {
        let e = row_to_employee(r);
        let lang: Option<String> = r.try_get("language").ok().flatten();
        json!({"id": e.id, "name": e.name, "role": e.role, "status": e.status.as_str(), "channel": e.channel, "auto_publish": e.auto_publish, "brand_id": e.brand_id, "config_version": e.config_version, "language": lang.unwrap_or_else(default_language)})
    }).collect();
    Ok(Json(json!({ "items": items })))
}

async fn get_employee(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let r = sqlx::query("SELECT * FROM digital_employees WHERE id = $1 AND agency_id = $2")
        .bind(id)
        .bind(auth.agency_id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "employee not found",
            ))
        })?;
    let e = row_to_employee(&r);
    let lang: Option<String> = sqlx::query_scalar(
        "SELECT identity->>'language' FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(err)?
    .flatten();
    Ok(Json(
        json!({"id": e.id, "name": e.name, "role": e.role, "status": e.status.as_str(), "channel": e.channel, "auto_publish": e.auto_publish, "brand_id": e.brand_id, "config_version": e.config_version, "language": lang.unwrap_or_else(default_language), "created_at": e.created_at}),
    ))
}

#[derive(Debug, Deserialize)]
struct PatchBody {
    name: Option<String>,
    auto_publish: Option<bool>,
    /// 修改输出语言 — 会写入新的 identity 版本 (DE §6 identity 版本化)
    language: Option<String>,
}

async fn patch_employee(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let upd = sqlx::query(
        "UPDATE digital_employees SET
           name = COALESCE($3, name),
           auto_publish = COALESCE($4, auto_publish),
           config_version = config_version + 1,
           updated_at = now()
         WHERE id = $1 AND agency_id = $2",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(&body.name)
    .bind(body.auto_publish)
    .execute(pool)
    .await
    .map_err(err)?;
    if upd.rows_affected() == 0 {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    if let Some(lang) = &body.language {
        crate::api::validate_language(lang).map_err(crate::api::app_error_response)?;
        // version the identity change: supersede current active, insert new version
        let cur = sqlx::query(
            "SELECT version, identity FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(err)?;
        let (version, mut identity) = match cur {
            Some(r) => (
                r.get::<i32, _>("version") + 1,
                r.get::<Value, _>("identity"),
            ),
            None => (1, json!({"name": body.name.clone().unwrap_or_default()})),
        };
        identity["language"] = json!(lang);
        sqlx::query("UPDATE employee_identity_versions SET status = 'superseded' WHERE employee_id = $1 AND status = 'active'")
            .bind(id)
            .execute(pool)
            .await
            .map_err(err)?;
        sqlx::query("INSERT INTO employee_identity_versions (employee_id, version, identity, status) VALUES ($1, $2, $3, 'active')")
            .bind(id)
            .bind(version)
            .bind(&identity)
            .execute(pool)
            .await
            .map_err(err)?;
    }
    Ok(Json(json!({ "id": id, "patched": true })))
}

/// start/pause/resume/stop — lifecycle transitions (DE §5.1), illegal ones 409.
async fn transition_employee(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    uri: axum::http::Uri,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let target = match uri.path().rsplit('/').next().unwrap_or("") {
        "start" => EmployeeStatus::Running,
        "pause" => EmployeeStatus::Paused,
        "resume" => EmployeeStatus::Running,
        "stop" => EmployeeStatus::Stopped,
        other => {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "bad_action",
                format!("unknown lifecycle action {other}"),
            )));
        }
    };
    let row = sqlx::query("SELECT status FROM digital_employees WHERE id = $1 AND agency_id = $2")
        .bind(id)
        .bind(auth.agency_id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .ok_or_else(|| {
            app_error_response(sincy_common::AppError::non_retryable(
                "not_found",
                "employee not found",
            ))
        })?;
    let from = EmployeeStatus::parse(&row.get::<String, _>("status")).unwrap();
    let to = transition(from, target).map_err(|e| {
        app_error_response(sincy_common::AppError::non_retryable(
            "illegal_transition",
            e.to_string(),
        ))
    })?;
    sqlx::query("UPDATE digital_employees SET status = $1, updated_at = now() WHERE id = $2")
        .bind(to.as_str())
        .bind(id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(Json(json!({ "id": id, "status": to.as_str() })))
}

// ---------- goals & strategy ----------

async fn list_goals(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2)",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if !owns {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let rows = sqlx::query("SELECT id, kind, target, deadline, status FROM employee_goals WHERE employee_id = $1 ORDER BY created_at")
        .bind(id).fetch_all(pool).await.map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.get::<Uuid, _>("id"), "kind": r.get::<String, _>("kind"),
        "target": r.get::<Value, _>("target"), "deadline": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("deadline"),
        "status": r.get::<String, _>("status"),
    })).collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct CreateGoalBody {
    kind: String,
    target: Value,
    deadline: Option<chrono::DateTime<chrono::Utc>>,
}

async fn create_goal(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateGoalBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2)",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if !owns {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let gid = Uuid::new_v4();
    sqlx::query("INSERT INTO employee_goals (id, employee_id, kind, target, deadline) VALUES ($1, $2, $3, $4, $5)")
        .bind(gid).bind(id).bind(&body.kind).bind(&body.target).bind(body.deadline)
        .execute(pool).await.map_err(err)?;
    Ok(Json(json!({ "id": gid, "status": "active" })))
}

async fn get_strategy(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2)",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if !owns {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let rows = sqlx::query("SELECT version, status, strategy FROM employee_strategies WHERE employee_id = $1 ORDER BY version DESC")
        .bind(id).fetch_all(pool).await.map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({
        "version": r.get::<i32, _>("version"), "status": r.get::<String, _>("status"), "strategy": r.get::<Value, _>("strategy"),
    })).collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct NewStrategyBody {
    strategy: Value,
}

async fn create_strategy_version(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Json(body): Json<NewStrategyBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2)",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if !owns {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let v: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM employee_strategies WHERE employee_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    // new version becomes ACTIVE, old ones SUPERSEDED (DE §7)
    sqlx::query("UPDATE employee_strategies SET status = 'SUPERSEDED' WHERE employee_id = $1 AND status = 'ACTIVE'")
        .bind(id).execute(pool).await.map_err(err)?;
    sqlx::query("INSERT INTO employee_strategies (employee_id, version, strategy, status) VALUES ($1, $2, $3, 'ACTIVE')")
        .bind(id).bind(v).bind(&body.strategy).execute(pool).await.map_err(err)?;
    Ok(Json(json!({ "version": v, "status": "ACTIVE" })))
}

// ---------- cycles ----------

#[derive(Debug, Deserialize)]
struct CyclesQuery {
    limit: Option<i64>,
}

async fn list_cycles(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Query(q): Query<CyclesQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT c.id, c.state, c.trigger, c.summary, c.started_at, c.finished_at, c.created_at
         FROM operating_cycles c JOIN digital_employees e ON e.id = c.employee_id
         WHERE c.employee_id = $1 AND e.agency_id = $2 ORDER BY c.created_at DESC LIMIT $3",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(q.limit.unwrap_or(50))
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.get::<Uuid, _>("id"), "state": r.get::<String, _>("state"), "trigger": r.get::<String, _>("trigger"),
        "summary": r.get::<Option<Value>, _>("summary"),
        "started_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("started_at"),
        "finished_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finished_at"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })).collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct StartCycleBody {
    trigger: Option<String>,
    idempotency_key: Option<String>,
}

/// POST /employees/{id}/cycles — run one operating cycle now (manual trigger).
/// The cycle itself is resumable via cycle_tasks; the publish step lands in
/// the existing pipeline (device route for X), verified by task state.
async fn start_cycle(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Json(body): Json<StartCycleBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let (emp, identity, mission, strategy) = load_full(pool, id).await?;
    // ownership check
    if emp.agency_id != *auth.agency_id.as_uuid() {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "forbidden",
            "not your employee",
        )));
    }
    if emp.status != EmployeeStatus::Running {
        return Err(app_error_response(sincy_common::AppError::needs_user(
            "not_running",
            format!("employee status is {}; start it first", emp.status.as_str()),
        )));
    }
    let idem = body
        .idempotency_key
        .clone()
        .unwrap_or_else(|| format!("cycle-{}", Uuid::new_v4()));
    let allow_mock = state.allow_mock_adapters;
    let jwt_secret = state.jwt_secret().to_vec();

    let deps = CycleDeps {
        store: Box::new(PgCycleStore { pool: pool.clone() }),
        intel: Box::new(PgIntelPort { pool: pool.clone() }),
        llm: Box::new(PgLlmPort { allow_mock }),
        publish: Box::new(PgPublishPort {
            pool: pool.clone(),
            agency: auth.agency_id,
            allow_mock,
            jwt_secret,
        }),
    };
    let trigger = body.trigger.unwrap_or_else(|| "manual".into());
    let outcome = run_cycle(&deps, &emp, &identity, &mission, &strategy, &trigger, &idem)
        .await
        .map_err(|e| app_error_response(sincy_common::AppError::retryable("cycle", e)))?;
    Ok(Json(json!({
        "cycle_id": outcome.cycle_id,
        "state": outcome.state,
        "selected_opportunity": outcome.selected_opportunity,
        "package_id": outcome.package_id,
        "task_id": outcome.task_id,
        "task_state": outcome.task_state,
        "summary": outcome.summary,
    })))
}

async fn get_cycle(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let r = sqlx::query(
        "SELECT c.id, c.employee_id, c.state, c.trigger, c.summary, c.started_at, c.finished_at, c.created_at
         FROM operating_cycles c JOIN digital_employees e ON e.id = c.employee_id
         WHERE c.id = $1 AND e.agency_id = $2",
    )
    .bind(id).bind(auth.agency_id.as_uuid()).fetch_optional(pool).await.map_err(err)?
    .ok_or_else(|| app_error_response(sincy_common::AppError::non_retryable("not_found", "cycle not found")))?;
    let tasks = sqlx::query("SELECT id, agent, step, state, output FROM cycle_tasks WHERE cycle_id = $1 ORDER BY created_at")
        .bind(id).fetch_all(pool).await.map_err(err)?;
    Ok(Json(json!({
        "id": r.get::<Uuid, _>("id"),
        "employee_id": r.get::<Uuid, _>("employee_id"),
        "state": r.get::<String, _>("state"),
        "trigger": r.get::<String, _>("trigger"),
        "summary": r.get::<Option<Value>, _>("summary"),
        "tasks": tasks.iter().map(|t| json!({
            "id": t.get::<Uuid, _>("id"), "agent": t.get::<String, _>("agent"),
            "step": t.get::<String, _>("step"), "state": t.get::<String, _>("state"),
            "output": t.get::<Option<Value>, _>("output"),
        })).collect::<Vec<_>>(),
    })))
}

// ---------- memory & insights ----------

async fn list_memory(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT m.id, m.type, m.content, m.evidence, m.confidence, m.created_at
         FROM memory_items m JOIN digital_employees e ON e.id = m.employee_id
         WHERE m.employee_id = $1 AND e.agency_id = $2 ORDER BY m.created_at DESC LIMIT 100",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.get::<Uuid, _>("id"), "type": r.get::<String, _>("type"), "content": r.get::<String, _>("content"),
        "evidence": r.get::<Value, _>("evidence"), "confidence": r.get::<rust_decimal::Decimal, _>("confidence"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })).collect();
    Ok(Json(json!({ "items": items })))
}

async fn list_insights(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT i.id, i.cycle_id, i.content, i.evidence, i.confidence, i.created_at
         FROM employee_insights i JOIN digital_employees e ON e.id = i.employee_id
         WHERE i.employee_id = $1 AND e.agency_id = $2 ORDER BY i.created_at DESC LIMIT 100",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| json!({
        "id": r.get::<Uuid, _>("id"), "cycle_id": r.get::<Option<Uuid>, _>("cycle_id"), "content": r.get::<String, _>("content"),
        "evidence": r.get::<Value, _>("evidence"), "confidence": r.get::<rust_decimal::Decimal, _>("confidence"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })).collect();
    Ok(Json(json!({ "items": items })))
}

// ---------- PG ports ----------

struct PgCycleStore {
    pool: sqlx::PgPool,
}

#[async_trait]
impl CycleStore for PgCycleStore {
    async fn begin_cycle(
        &self,
        employee_id: Uuid,
        trigger: &str,
        idem: &str,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO operating_cycles (id, employee_id, state, trigger, idempotency_key, started_at)
             VALUES ($1, $2, 'RUNNING', $3, $4, now())
             ON CONFLICT (employee_id, idempotency_key) DO UPDATE SET state = 'RUNNING' RETURNING id",
        )
        .bind(id).bind(employee_id).bind(trigger).bind(idem)
        .fetch_one(&self.pool).await.map(|r| r.get(0)).map_err(|e| e.to_string())
    }
    async fn finish_cycle(
        &self,
        cycle_id: Uuid,
        state: &str,
        summary: &Value,
    ) -> Result<(), String> {
        sqlx::query("UPDATE operating_cycles SET state = $1, summary = $2, finished_at = now() WHERE id = $3")
            .bind(state).bind(summary).bind(cycle_id)
            .execute(&self.pool).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn begin_task(
        &self,
        cycle_id: Uuid,
        agent: AgentKind,
        step: CycleStep,
        idem: &str,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO cycle_tasks (id, cycle_id, agent, step, state, idempotency_key)
             VALUES ($1, $2, $3, $4, 'RUNNING', $5)
             ON CONFLICT (cycle_id, idempotency_key) DO UPDATE SET state = 'RUNNING' RETURNING id",
        )
        .bind(id)
        .bind(cycle_id)
        .bind(agent.as_str())
        .bind(step.as_str())
        .bind(idem)
        .fetch_one(&self.pool)
        .await
        .map(|r| r.get(0))
        .map_err(|e| e.to_string())
    }
    async fn finish_task(&self, task_id: Uuid, state: &str, output: &Value) -> Result<(), String> {
        sqlx::query("UPDATE cycle_tasks SET state = $1, output = $2 WHERE id = $3")
            .bind(state)
            .bind(output)
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    async fn add_memory(&self, item: &MemoryItem) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO memory_items (id, employee_id, type, content, evidence, confidence, created_at, last_validated)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(item.id).bind(item.employee_id).bind(item.r#type.as_str()).bind(&item.content)
        .bind(json!(item.evidence)).bind(rust_decimal::Decimal::try_from(item.confidence).unwrap_or(rust_decimal::Decimal::new(5, 1)))
        .bind(item.created_at).bind(item.last_validated)
        .execute(&self.pool).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn add_insight(
        &self,
        employee_id: Uuid,
        cycle_id: Uuid,
        content: &str,
        evidence: &Value,
        confidence: f64,
    ) -> Result<(), String> {
        sqlx::query("INSERT INTO employee_insights (employee_id, cycle_id, content, evidence, confidence) VALUES ($1, $2, $3, $4, $5)")
            .bind(employee_id).bind(cycle_id).bind(content).bind(evidence)
            .bind(rust_decimal::Decimal::try_from(confidence).unwrap_or(rust_decimal::Decimal::new(5, 1)))
            .execute(&self.pool).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn recent_topics(&self, employee_id: Uuid, limit: usize) -> Result<Vec<String>, String> {
        // recent content of this employee's brand = anti-template signal
        let rows = sqlx::query(
            "SELECT p.text_content FROM publishing_packages p
             JOIN digital_employees e ON e.brand_id = p.brand_id
             WHERE e.id = $1 ORDER BY p.created_at DESC LIMIT $2",
        )
        .bind(employee_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.iter().map(|r| r.get::<String, _>(0)).collect())
    }
    async fn mark_opportunity(&self, topic: &str, status: &str) -> Result<(), String> {
        sqlx::query("UPDATE intel_opportunities SET status = $1 WHERE topic = $2")
            .bind(status)
            .bind(topic)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

struct PgIntelPort {
    pool: sqlx::PgPool,
}

#[async_trait]
impl IntelPort for PgIntelPort {
    async fn opportunities(
        &self,
        _ctx: &EmployeeContext,
        limit: usize,
    ) -> Result<Vec<Opportunity>, String> {
        let rows = sqlx::query(
            "SELECT topic, score, rationale, evidence FROM intel_opportunities WHERE status = 'new' ORDER BY score DESC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                let evidence: Vec<String> = r
                    .get::<Value, _>("evidence")
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Opportunity {
                    topic: r.get("topic"),
                    score: r
                        .get::<rust_decimal::Decimal, _>("score")
                        .try_into()
                        .unwrap_or(0.0),
                    rationale: r.get("rationale"),
                    evidence,
                    status: "new".into(),
                }
            })
            .collect())
    }
}

struct PgLlmPort {
    allow_mock: bool,
}

#[async_trait]
impl LlmPort for PgLlmPort {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        use sincy_ai::LlmGateway;
        // same provider policy as /ai/generate: mock only when explicitly allowed
        let provider = std::env::var("SINCY_LLM_PROVIDER").unwrap_or_else(|_| "mock".into());
        let req = sincy_ai::GenerationRequest {
            prompt: prompt.to_string(),
            model: std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "mock-1".into()),
            max_tokens,
            content_item_id: None,
        };
        if provider == "mock" {
            if !self.allow_mock {
                return Err("mock LLM disabled outside local".into());
            }
            let r = sincy_ai::MockLlmGateway
                .generate(&req)
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.text)
        } else {
            let gw = sincy_ai::OpenAiCompatibleGateway::new(
                std::env::var("SINCY_LLM_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                std::env::var("SINCY_LLM_API_KEY").unwrap_or_default(),
                std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
            );
            let r = gw.generate(&req).await.map_err(|e| e.to_string())?;
            Ok(r.text)
        }
    }
}

struct PgPublishPort {
    pool: sqlx::PgPool,
    agency: sincy_common::AgencyId,
    allow_mock: bool,
    #[allow(dead_code)]
    jwt_secret: Vec<u8>,
}

#[async_trait]
impl PublishPort for PgPublishPort {
    async fn save_package(
        &self,
        employee: &Employee,
        topic: &str,
        text: &str,
    ) -> Result<(Uuid, String), String> {
        let brand_id = employee.brand_id.ok_or("employee has no brand_id")?;
        let status = if employee.auto_publish {
            "approved"
        } else {
            "pending"
        };
        let pid = insert_package(
            &self.pool,
            self.agency,
            NewPackage {
                brand_id,
                platform: &employee.channel,
                action: if employee.channel == "x" { "publish_post" } else { "publish_video" },
                text,
                source: "ai",
                provenance: json!({"employee_id": employee.id, "topic": topic, "auto_publish": employee.auto_publish}),
                approval_status: status,
            },
        )
        .await
        .map_err(|e| format!("{e:?}"))?;
        Ok((pid, status.to_string()))
    }
    async fn approve_package(&self, package_id: Uuid) -> Result<(), String> {
        sqlx::query("UPDATE publishing_packages SET approval_status = 'approved' WHERE id = $1 AND agency_id = $2 AND approval_status = 'pending'")
            .bind(package_id).bind(self.agency.as_uuid())
            .execute(&self.pool).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn publish(
        &self,
        employee: &Employee,
        package_id: Uuid,
    ) -> Result<PublishReceipt, String> {
        let brand_id = employee.brand_id.ok_or("employee has no brand_id")?;
        let body = PublishTaskBody {
            agency_id: None,
            brand_id,
            platform: employee.channel.clone(),
            action: if employee.channel == "x" {
                "publish_post".into()
            } else {
                "publish_video".into()
            },
            package_id,
            package_version: 1,
            idempotency_key: format!("employee-{}-{}", employee.id, package_id),
        };
        let axum::Json(resp) = publish_core(&self.pool, self.agency, body, self.allow_mock)
            .await
            .map_err(|e| format!("{e:?}"))?;
        Ok(PublishReceipt {
            task_id: resp.task_id,
            route: resp.route,
            status: resp.status,
            package_id,
            approval_status: "approved".into(),
        })
    }
    async fn task_state(&self, task_id: Uuid) -> Result<String, String> {
        sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())
    }
}
