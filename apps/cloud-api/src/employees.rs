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
use sincy_intel::model::{Opportunity, Trend};
use sincy_intel::opportunity::{pillar_match, score_opportunity};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::api::{NewPackage, PublishTaskBody, app_error_response, insert_package, publish_core};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/employees", get(list_employees).post(create_employee))
        .route(
            "/employees/{id}",
            get(get_employee)
                .patch(patch_employee)
                .delete(delete_employee),
        )
        .route("/employees/{id}/start", post(transition_employee))
        .route("/employees/{id}/pause", post(transition_employee))
        .route("/employees/{id}/resume", post(transition_employee))
        .route("/employees/{id}/stop", post(transition_employee))
        .route("/employees/{id}/unarchive", post(transition_employee))
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
        .route("/employees/{id}/feed", get(employee_feed))
        .route("/employees/{id}/intel-board", get(intel_board))
        .route("/employees/{id}/plugins", get(list_plugins))
}

/// GET /api/v1/employees/{id}/plugins — 插件清单:该员工的内置 agents + 已绑定 skills,
/// 每个条目都带 title/description,便于人理解"这个员工到底能干什么"。
async fn list_plugins(
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
    let skill_rows = sqlx::query(
        "SELECT s.*, b.enabled, b.bound_at FROM employee_skill_bindings b \
         JOIN skills s ON s.id = b.skill_id WHERE b.employee_id = $1 ORDER BY s.domain, s.name",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let what_map: std::collections::HashMap<&str, &str> =
        sincy_employee::handlers::skill_what_it_does()
            .into_iter()
            .collect();
    let skills: Vec<Value> = skill_rows
        .iter()
        .map(|r| {
            let tools: Value = r.get("tools");
            let name = r.get::<String, _>("name");
            json!({
                "type": "skill",
                "id": r.get::<Uuid, _>("id"),
                "domain": r.get::<String, _>("domain"),
                "name": name,
                "version": r.get::<i32, _>("version"),
                "title": r.get::<String, _>("title"),
                "description": r.get::<String, _>("description"),
                "what_it_does": what_map
                    .get(name.as_str())
                    .copied()
                    .map(Value::from)
                    .unwrap_or(Value::Null),
                "tools": tools.as_array().cloned().unwrap_or_default(),
                "handler": sincy_employee::handlers::handler_skill_names()
                    .contains(&name.as_str()),
                "enabled": r.get::<bool, _>("enabled"),
                "bound_at": r.get::<chrono::DateTime<chrono::Utc>, _>("bound_at"),
            })
        })
        .collect();
    Ok(Json(json!({
        "employee": {
            "id": e.id,
            "name": e.name,
            "role": e.role,
            "status": e.status.as_str(),
        },
        "agents": AgentKind::all_plugin_json(),
        "skills": skills,
    })))
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
            voice_profile: json!({}),
            focus: json!({"scope":"general"}),
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
    /// 领域画像 (WS1, 必填): {scope:'niche'|'general', industry?, sub_verticals?, topics?, audience?, excluded_topics?}
    focus: Option<Value>,
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
    // WS1: focus 必填; niche 时 industry 必填且须在分类法内 (422)
    let focus = match &body.focus {
        Some(f) => {
            crate::focus::validate_focus(f).map_err(|e| {
                app_error_response(sincy_common::AppError::non_retryable("invalid_focus", e))
            })?;
            f.clone()
        }
        None => {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "focus_required",
                "focus is required: {scope:'niche'|'general', industry?, sub_verticals?, topics?, audience?, excluded_topics?}",
            )));
        }
    };
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
    identity["focus"] = focus;
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
    // 绑定全部 active skills (可在 /employees/{id}/skills 里逐个停用)
    crate::skills::bind_all_active_skills(pool, id).await?;
    Ok(Json(json!({ "id": id, "status": "READY" })))
}

async fn list_employees(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT e.*, (SELECT v.identity->>'language' FROM employee_identity_versions v
                      WHERE v.employee_id = e.id AND v.status = 'active' ORDER BY v.version DESC LIMIT 1) AS language,
                      (SELECT v.identity->'focus' FROM employee_identity_versions v
                      WHERE v.employee_id = e.id AND v.status = 'active' ORDER BY v.version DESC LIMIT 1) AS focus
         FROM digital_employees e WHERE e.agency_id = $1 ORDER BY e.created_at DESC",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows.iter().map(|r| {
        let e = row_to_employee(r);
        let lang: Option<String> = r.try_get("language").ok().flatten();
        let focus: Option<Value> = r.try_get("focus").ok().flatten();
        json!({"id": e.id, "name": e.name, "role": e.role, "status": e.status.as_str(), "channel": e.channel, "auto_publish": e.auto_publish, "brand_id": e.brand_id, "config_version": e.config_version, "language": lang.unwrap_or_else(default_language), "focus": focus.unwrap_or_else(|| json!({"scope":"general"}))})
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
    let ident: Value = sqlx::query_scalar(
        "SELECT identity FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(err)?
    .unwrap_or_else(|| json!({}));
    Ok(Json(
        json!({"id": e.id, "name": e.name, "role": e.role, "status": e.status.as_str(), "channel": e.channel, "auto_publish": e.auto_publish, "brand_id": e.brand_id, "config_version": e.config_version, "language": lang.unwrap_or_else(default_language), "focus": ident.get("focus").cloned().unwrap_or_else(|| json!({"scope":"general"})), "identity": ident, "created_at": e.created_at}),
    ))
}

#[derive(Debug, Deserialize)]
struct PatchBody {
    name: Option<String>,
    role: Option<String>,
    channel: Option<String>,
    auto_publish: Option<bool>,
    /// 修改领域画像 — 校验后写入新的 identity 版本
    focus: Option<Value>,
    /// 修改输出语言 — 会写入新的 identity 版本 (DE §6 identity 版本化)
    language: Option<String>,
    /// identity 局部字段合并 (personality/voice/voice_profile/audience/expertise/values/boundaries) — 写入新 identity 版本
    identity: Option<Value>,
    /// 使命 statement 等局部字段合并 — 写入新的 mission 版本
    mission: Option<Value>,
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
           role = COALESCE($4, role),
           channel = COALESCE($5, channel),
           auto_publish = COALESCE($6, auto_publish),
           config_version = config_version + 1,
           updated_at = now()
         WHERE id = $1 AND agency_id = $2",
    )
    .bind(id)
    .bind(auth.agency_id.as_uuid())
    .bind(&body.name)
    .bind(&body.role)
    .bind(&body.channel)
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
    if let Some(f) = &body.focus {
        crate::focus::validate_focus(f).map_err(|e| {
            app_error_response(sincy_common::AppError::non_retryable("invalid_focus", e))
        })?;
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
            None => (1, json!({})),
        };
        identity["focus"] = f.clone();
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
    // identity 局部合并 (voice/voice_profile/personality/audience/…) — 版本化写入
    if let Some(patch) = &body.identity {
        if !patch.is_object() {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "identity must be a JSON object",
            )));
        }
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
            None => (1, json!({})),
        };
        for (k, v) in patch.as_object().unwrap() {
            identity[k] = v.clone();
        }
        if let Some(f) = identity.get("focus") {
            crate::focus::validate_focus(f).map_err(|e| {
                app_error_response(sincy_common::AppError::non_retryable("invalid_focus", e))
            })?;
        }
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
    // mission 局部合并 — 版本化写入
    if let Some(patch) = &body.mission {
        if !patch.is_object() {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "mission must be a JSON object",
            )));
        }
        let cur = sqlx::query(
            "SELECT version, mission FROM employee_missions WHERE employee_id = $1 AND active ORDER BY version DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(err)?;
        let (version, mut mission) = match cur {
            Some(r) => (r.get::<i32, _>("version") + 1, r.get::<Value, _>("mission")),
            None => (1, json!({})),
        };
        for (k, v) in patch.as_object().unwrap() {
            mission[k] = v.clone();
        }
        sqlx::query(
            "UPDATE employee_missions SET active = false WHERE employee_id = $1 AND active",
        )
        .bind(id)
        .execute(pool)
        .await
        .map_err(err)?;
        sqlx::query("INSERT INTO employee_missions (employee_id, version, mission, active) VALUES ($1, $2, $3, true)")
            .bind(id)
            .bind(version)
            .bind(&mission)
            .execute(pool)
            .await
            .map_err(err)?;
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

/// DELETE /employees/{id} — 软删除 (归档为 ARCHIVED, 可 POST /employees/{id}/unarchive 恢复为 READY; 数据全留)。RUNNING 的员工先 stop 再删。
async fn delete_employee(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query("SELECT status FROM digital_employees WHERE id = $1 AND agency_id = $2")
        .bind(id)
        .bind(auth.agency_id.as_uuid())
        .fetch_optional(pool)
        .await
        .map_err(err)?;
    let Some(r) = row else {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    };
    let status: String = r.get("status");
    if status == "RUNNING" {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "conflict",
            "employee is RUNNING — stop it first (POST /employees/{id}/stop), then delete",
        )));
    }
    let upd = sqlx::query("UPDATE digital_employees SET status = 'ARCHIVED', config_version = config_version + 1, updated_at = now() WHERE id = $1 AND agency_id = $2")
        .bind(id)
        .bind(auth.agency_id.as_uuid())
        .execute(pool)
        .await
        .map_err(err)?;
    if upd.rows_affected() == 0 {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    Ok(Json(
        json!({ "id": id, "status": "ARCHIVED", "deleted": true }),
    ))
}

/// start/pause/resume/stop/unarchive — lifecycle transitions (DE §5.1), illegal ones 409.
/// unarchive: ARCHIVED → READY (soft-delete 恢复), 之后再 start 进入 RUNNING.
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
        "unarchive" => EmployeeStatus::Ready,
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
    let (emp, _identity, _mission, _strategy) = load_full(pool, id).await?;
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
    let trigger = body.trigger.unwrap_or_else(|| "manual".into());
    let outcome = run_cycle_for(
        pool,
        auth.agency_id,
        id,
        &trigger,
        &idem,
        state.allow_mock_adapters,
        state.jwt_secret(),
    )
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

/// 手动执行一个 skill handler (POST /skills/run): 真调用, 返回可审计结果。
/// employee_id 可选 —— 带上时用该员工的身份/策略生成 persona prompt。
pub(crate) async fn run_skill_by_name(
    pool: &sqlx::PgPool,
    agency: sincy_common::AgencyId,
    employee_id: Option<Uuid>,
    name: &str,
    input: Value,
    allow_mock: bool,
    jwt_secret: &[u8],
) -> Result<Value, String> {
    let (emp, identity, mission, strategy) = match employee_id {
        Some(id) => {
            let (emp, identity, mission, strategy) = load_full(pool, id)
                .await
                .map_err(|e| format!("load employee: {e:?}"))?;
            if emp.agency_id != *agency.as_uuid() {
                return Err("not your employee".into());
            }
            (emp, identity, mission, strategy)
        }
        None => {
            // 无员工上下文: 用占位员工 + 默认人设 (仍可跑 intel/llm/stats 类 handler)
            let emp = Employee {
                id: Uuid::nil(),
                agency_id: *agency.as_uuid(),
                name: "adhoc".into(),
                role: "content_operator".into(),
                status: sincy_employee::EmployeeStatus::Running,
                config_version: 1,
                brand_id: None,
                channel: "x".into(),
                auto_publish: false,
                created_at: chrono::Utc::now(),
            };
            (
                emp,
                Identity {
                    version: 1,
                    name: "adhoc".into(),
                    role: "content_operator".into(),
                    personality: json!({}),
                    voice: "casual, human".into(),
                    voice_profile: json!({}),
                    focus: json!({"scope":"general"}),
                    language: "zh-CN".into(),
                    audience: vec![],
                    boundaries: vec![],
                    expertise: vec![],
                    values: vec![],
                },
                Mission {
                    statement: "adhoc skill run".into(),
                    domain: "content".into(),
                    audience: vec![],
                    value_proposition: String::new(),
                    constraints: vec![],
                    success_definition: String::new(),
                },
                Strategy::default(),
            )
        }
    };
    let persona = json!({
        "name": identity.name, "role": identity.role,
        "mission": mission.statement, "audience": identity.audience,
        "voice": identity.voice, "pillars": strategy.pillars,
        "voice_block": sincy_content::voice::compile_voice_prompt(&identity.voice_profile, 3),
        "language": identity.language,
    });
    // 品牌 no_go 列表
    let no_go: Vec<String> = match emp.brand_id {
        Some(bid) => sqlx::query("SELECT config->'no_go' FROM brands WHERE id = $1")
            .bind(bid)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.try_get::<Value, _>(0).ok())
            .and_then(|v| v.as_array().cloned())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let deps = CycleDeps {
        store: Box::new(PgCycleStore { pool: pool.clone() }),
        intel: Box::new(PgIntelPort { pool: pool.clone() }),
        llm: Box::new(PgLlmPort { allow_mock }),
        publish: Box::new(PgPublishPort {
            pool: pool.clone(),
            agency,
            allow_mock,
            jwt_secret: jwt_secret.to_vec(),
        }),
        stats: Box::new(PgStatsPort { pool: pool.clone() }),
    };
    let hctx = sincy_employee::handlers::SkillRunCtx {
        deps: &deps,
        employee: &emp,
        persona: persona.clone(),
        no_go,
    };
    let run = sincy_employee::handlers::run_skill(name, &input, &hctx).await?;
    Ok(json!({
        "skill": run.skill,
        "duration_ms": run.duration_ms,
        "output": run.output,
    }))
}

/// 执行一个运营周期的共享实现 (HTTP 手工触发 + 后台调度器共用)。
pub(crate) async fn run_cycle_for(
    pool: &sqlx::PgPool,
    agency: sincy_common::AgencyId,
    id: Uuid,
    trigger: &str,
    idem: &str,
    allow_mock: bool,
    jwt_secret: &[u8],
) -> Result<sincy_employee::CycleOutcome, String> {
    let (emp, identity, mission, strategy) = load_full(pool, id)
        .await
        .map_err(|e| format!("load employee: {e:?}"))?;
    if emp.agency_id != *agency.as_uuid() {
        return Err("not your employee".into());
    }
    if emp.status != EmployeeStatus::Running {
        return Err(format!(
            "employee status is {}; start it first",
            emp.status.as_str()
        ));
    }
    let deps = CycleDeps {
        store: Box::new(PgCycleStore { pool: pool.clone() }),
        intel: Box::new(PgIntelPort { pool: pool.clone() }),
        llm: Box::new(PgLlmPort { allow_mock }),
        publish: Box::new(PgPublishPort {
            pool: pool.clone(),
            agency,
            allow_mock,
            jwt_secret: jwt_secret.to_vec(),
        }),
        stats: Box::new(PgStatsPort { pool: pool.clone() }),
    };
    let skills = crate::skills::load_bound_skills(pool, id)
        .await
        .map_err(|e| format!("load skills: {e:?}"))?;
    run_cycle(
        &deps, &emp, &identity, &mission, &strategy, &skills, trigger, idem,
    )
    .await
}

/// 后台 drain 循环 (b): 到期的 cycle_measurements 读 device_tasks 真实终态,
/// 写回 metrics + insight (喂 Learn), 并产出一条数据驱动 adjustment → 下周期消费。
/// 与员工调度器并行 spawn; 表不走 RLS, 显式按 employee_id scope。
pub async fn measurement_drain_loop(pool: sqlx::PgPool) {
    let interval_secs: u64 = std::env::var("SINCY_MEASURE_DRAIN_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    loop {
        tick.tick().await;
        let rows = match sqlx::query(
            "SELECT id, cycle_id, employee_id, package_id, task_id FROM cycle_measurements \
             WHERE status = 'pending' AND due_at <= now() ORDER BY due_at LIMIT 50",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(r) => r,
            Err(_) => continue,
        };
        for row in rows {
            let mid: Uuid = row.get("id");
            let cycle_id: Uuid = row.get("cycle_id");
            let employee_id: Uuid = row.get("employee_id");
            let task_id: Option<Uuid> = row.get("task_id");
            let store = PgCycleStore { pool: pool.clone() };
            // 读发布任务真实终态 (device_tasks), 不信提交时的状态
            let task_state = match task_id {
                Some(tid) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT state FROM device_tasks WHERE id = $1",
                )
                .bind(tid)
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .flatten(),
                None => None,
            };
            // patch-B: Δ粉丝作为 reward 并入 metrics (account_growth 读 account_metric_samples 窗口增量)
            let growth = sincy_employee::StatsPort::account_growth(
                &PgStatsPort { pool: pool.clone() },
                employee_id,
                7,
            )
            .await
            .unwrap_or_else(|_| json!({}));
            let followers_delta = growth["delta"].as_i64();
            let metrics = json!({
                "task_id": task_id,
                "task_state": task_state,
                // engagement 指标接入点: device_tasks 回执/平台侧拉取 engagement
                // (likes/comments/shares) 在此并入 metrics, 例如从 adapter evidence 或平台 API 回采
                "engagement": null,
                "followers_now": growth["followers_now"],
                "followers_delta_7d": followers_delta,
            });
            let insight =
                format!("delayed measurement of cycle {cycle_id}: task state = {task_state:?}");
            store
                .add_insight(employee_id, cycle_id, &insight, &metrics, 0.6)
                .await
                .ok();
            // 数据驱动 adjustment: 真实结果驱动下一轮 (覆盖 in-cycle 临时值)
            // patch-B: 真·涨粉信号驱动角度 — Δ>0 强化 hot_take, Δ<=0 换 how_to (数据驱动, 非"发出去没")
            let adjustment = json!({
                "angle": match (task_state.as_deref(), followers_delta) {
                    (Some("FAILED") | Some("FAILED_FINAL"), _) => "how_to",
                    (_, Some(d)) if d > 0 => "hot_take",
                    (_, Some(_)) => "how_to",
                    (Some("COMPLETED"), None) => "hot_take",
                    _ => "explanation",
                },
                "pillar_boost": {},
                "source": "delayed measurement + follower delta",
                "metrics": metrics,
            });
            store
                .add_experiment_adjustment(
                    employee_id,
                    &format!("delayed measurement result: task_state={task_state:?}"),
                    "delayed_measurement",
                    &adjustment,
                )
                .await
                .ok();
            store
                .complete_measurement(mid, &metrics, &insight)
                .await
                .ok();
            tracing::info!(measurement = %mid, employee = %employee_id, task_state = ?task_state, "cycle measurement drained");
        }
    }
}

/// 后台调度器 (DE: strategy.cadence_per_week 决定周期节奏)。
/// 每 SINCY_EMPLOYEE_SCHEDULER_INTERVAL_SECS (默认 60s) 扫描一次 RUNNING 员工,
/// 上次周期(或创建时间)超过 cadence 间隔的就自动跑一个 trigger=scheduled 的周期。
/// 幂等键按小时分桶: sched-<emp>-<yyyymmddhh>, 同一小时不会重复触发。
/// SINCY_EMPLOYEE_SCHEDULER_ENABLED=false 可关闭。
pub fn spawn_scheduler(state: crate::state::AppState) {
    let interval_secs: u64 = std::env::var("SINCY_EMPLOYEE_SCHEDULER_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tick.tick().await;
            let pool = match state.pool() {
                Ok(p) => p.clone(),
                Err(_) => continue,
            };
            let due: Vec<Uuid> = match sqlx::query_scalar(
                "SELECT e.id FROM digital_employees e \
                 JOIN employee_strategies s ON s.employee_id = e.id AND s.status = 'ACTIVE' \
                 WHERE e.status = 'RUNNING' \
                   AND COALESCE( \
                       (SELECT c.created_at FROM operating_cycles c WHERE c.employee_id = e.id ORDER BY c.created_at DESC LIMIT 1), \
                       e.created_at) \
                     + make_interval(secs => (604800.0 / GREATEST(COALESCE(NULLIF(s.strategy->>'cadence_per_week','')::int, 7), 1))::float8) \
                   <= now()",
            )
            .fetch_all(&pool)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(error = %e, "employee scheduler: due query failed");
                    continue;
                }
            };
            for emp_id in due {
                let bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
                let idem = format!("sched-{emp_id}-{bucket}");
                let agency_row: Option<(Uuid,)> =
                    sqlx::query_as("SELECT agency_id FROM digital_employees WHERE id = $1")
                        .bind(emp_id)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten();
                let Some((agency_uuid,)) = agency_row else {
                    continue;
                };
                let agency = sincy_common::AgencyId::from_uuid(agency_uuid);
                tracing::info!(%emp_id, %idem, "employee scheduler: running scheduled cycle");
                match run_cycle_for(
                    &pool,
                    agency,
                    emp_id,
                    "scheduled",
                    &idem,
                    state.allow_mock_adapters,
                    state.jwt_secret(),
                )
                .await
                {
                    Ok(out) => {
                        tracing::info!(%emp_id, cycle = %out.cycle_id, state = %out.state, topic = ?out.selected_opportunity, "scheduled cycle done");
                    }
                    Err(e) => {
                        tracing::warn!(%emp_id, error = %e, "scheduled cycle failed");
                    }
                }
            }
        }
    });
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

pub(crate) struct PgCycleStore {
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

    async fn add_experiment(
        &self,
        employee_id: Uuid,
        hypothesis: &str,
        variable: &str,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO employee_experiments (id, employee_id, hypothesis, variable, status) \
             VALUES ($1, $2, $3, $4, 'PROPOSED')",
        )
        .bind(id)
        .bind(employee_id)
        .bind(hypothesis)
        .bind(variable)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(id)
    }

    async fn add_experiment_adjustment(
        &self,
        employee_id: Uuid,
        hypothesis: &str,
        variable: &str,
        adjustment: &Value,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO employee_experiments (id, employee_id, hypothesis, variable, status, adjustment) \
             VALUES ($1, $2, $3, $4, 'PROPOSED', $5)",
        )
        .bind(id)
        .bind(employee_id)
        .bind(hypothesis)
        .bind(variable)
        .bind(adjustment)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(id)
    }

    async fn take_pending_adjustment(&self, employee_id: Uuid) -> Result<Option<Value>, String> {
        // 原子取用: 标 applied_at 防止重复消费 (并发周期安全)
        let row = sqlx::query(
            "UPDATE employee_experiments SET applied_at = now() \
             WHERE id = (SELECT id FROM employee_experiments \
                         WHERE employee_id = $1 AND adjustment IS NOT NULL AND applied_at IS NULL \
                         ORDER BY created_at DESC LIMIT 1) \
             RETURNING adjustment",
        )
        .bind(employee_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.and_then(|r| r.get::<Option<Value>, _>("adjustment")))
    }

    async fn enqueue_measurement(
        &self,
        m: &sincy_employee::cycle::MeasurementRequest,
    ) -> Result<(), String> {
        // agency_id 落库供显式 scope (表不走 RLS, 后台 drain 无租户上下文)
        let agency: Option<Uuid> =
            sqlx::query_scalar("SELECT agency_id FROM digital_employees WHERE id = $1")
                .bind(m.employee_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| e.to_string())?
                .flatten();
        sqlx::query(
            "INSERT INTO cycle_measurements (cycle_id, employee_id, agency_id, package_id, task_id, due_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(m.cycle_id)
        .bind(m.employee_id)
        .bind(agency)
        .bind(m.package_id)
        .bind(m.task_id)
        .bind(m.due_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    async fn complete_measurement(
        &self,
        id: Uuid,
        metrics: &Value,
        insight: &str,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE cycle_measurements SET status = 'done', metrics = $2, insight = $3, measured_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(metrics)
        .bind(insight)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    async fn account_voice_profile(
        &self,
        employee_id: uuid::Uuid,
        platform: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let row = sqlx::query(
            "SELECT voice_profile FROM employee_accounts WHERE employee_id = $1 AND platform = $2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(employee_id)
        .bind(platform)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(row.and_then(|r| r.try_get::<serde_json::Value, _>("voice_profile").ok()))
    }
    async fn review_rule(
        &self,
        platform: &str,
    ) -> Result<sincy_employee::cycle::PlatformReviewRule, String> {
        use sincy_employee::cycle::PlatformReviewRule;
        let row = sqlx::query(
            "SELECT platform, weighted_limit, on_over_limit FROM platform_review_rules WHERE platform = $1",
        )
        .bind(platform)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let row = match row {
            Some(r) => r,
            None => sqlx::query(
                "SELECT platform, weighted_limit, on_over_limit FROM platform_review_rules WHERE platform = 'default'",
            )
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no default review rule".to_string())?,
        };
        use sqlx::Row;
        Ok(PlatformReviewRule {
            platform: row.get("platform"),
            weighted_limit: row.get::<i32, _>("weighted_limit") as usize,
            on_over_limit: row.get("on_over_limit"),
        })
    }

    async fn set_thread_chunks(&self, package_id: Uuid, chunks: &[String]) -> Result<(), String> {
        sqlx::query("UPDATE publishing_packages SET thread_chunks = $1 WHERE id = $2")
            .bind(serde_json::to_value(chunks).map_err(|e| e.to_string())?)
            .bind(package_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(crate) struct PgIntelPort {
    pool: sqlx::PgPool,
}

#[async_trait]
impl IntelPort for PgIntelPort {
    /// 品牌感知的机会选择 (WI §20 adapter: 只给建议): 拉一批候选, 用员工的
    /// 支柱/受众对每个 topic 重新算 pillar_match 并重打分, 低相关直接过滤,
    /// 证据解析成可读的 "标题 (来源) url" 喂给内容生产 prompt。
    async fn opportunities(
        &self,
        ctx: &EmployeeContext,
        limit: usize,
    ) -> Result<Vec<Opportunity>, String> {
        let rows = sqlx::query(
            "SELECT topic, score, rationale, evidence FROM intel_opportunities WHERE status = 'new' AND created_at > now() - interval '7 days' ORDER BY score DESC LIMIT 400",
        )
        .fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
        let mut out: Vec<Opportunity> = Vec::new();
        // WS2: 领域画像 → 现算 per-员工相关性 (原则1: 不烤死在实体上)
        let focus = ctx
            .focus
            .clone()
            .map(crate::focus::focus_from_value)
            .unwrap_or_default();
        let niche = !focus.is_general();
        let mut pillars = ctx.pillars.clone();
        if pillars.is_empty() {
            pillars = focus.pillars(); // 空 pillars 用 focus 填满, 避免中性 0.5
        }
        let cp = focus.customer_profile();
        for r in &rows {
            let topic: String = r.get("topic");
            let rationale: Value = r.get("rationale");
            let title = rationale
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            // 品牌相关性: topic + 标题 vs 支柱/受众
            let haystack = format!("{topic} {title}");
            // WS2 硬过滤: 负向主题直接丢弃
            if niche && focus.excluded_hit(&haystack) {
                continue;
            }
            // 候选行业标签: rationale.industry_tags ∪ 证据 intel_items.industries
            let keys0: Vec<String> = r
                .get::<Value, _>("evidence")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut tags: Vec<String> = rationale
                .get("industry_tags")
                .and_then(|t| t.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !keys0.is_empty() {
                let rows2 = sqlx::query(
                    "SELECT DISTINCT jsonb_array_elements_text(industries) AS t FROM intel_items WHERE dedup_key = ANY($1) AND industries <> '[]'::jsonb",
                )
                .bind(&keys0)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
                for r2 in &rows2 {
                    tags.push(r2.get::<String, _>("t"));
                }
            }
            if niche && !tags.is_empty() && !focus.tags_intersect(&tags) {
                continue; // industry_tags 与 focus 无交集 → 丢弃
            }
            let pm = pillar_match(&haystack, &pillars, &ctx.audience);
            if pm < 0.45 {
                continue; // 与品牌支柱不沾边的直接不给员工看 (单关键词蹭分已不够)
            }
            // 用 rationale 里的趋势分量重打分
            let velocity = rationale
                .get("velocity")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);
            let volume = rationale
                .get("volume")
                .and_then(|v| v.as_i64())
                .unwrap_or(600);
            let freshness = rationale
                .get("freshness_hours")
                .and_then(|v| v.as_f64())
                .unwrap_or(24.0);
            let trend = Trend {
                topic: topic.clone(),
                platforms: rationale
                    .get("platforms")
                    .and_then(|p| p.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                volume,
                velocity,
                sentiment: 0.5,
                window_hours: 24,
            };
            // WS2: 保留者用 relevance_score 参与打分 (per-员工现算)
            // relevance_score 返回 0..100, 归一化到 0..1
            let rel = if niche {
                (sincy_intel::industry::relevance_score(&tags, &haystack, &cp) / 100.0)
                    .clamp(0.0, 1.0)
            } else {
                0.5
            };
            let mut score = score_opportunity(pm, &trend, 0.2, freshness) * (0.4 + 0.6 * rel);
            // 噪声信源降权: 只有 wikipedia-rc 这类百科流水的 topic 衰减 50%
            let plats = rationale
                .get("platforms")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default();
            let only_noise =
                !plats.is_empty() && plats.iter().all(|p| p.as_str() == Some("wikipedia-rc"));
            if only_noise {
                score *= 0.5;
            }
            let mut rat = rationale.clone();
            rat["focus_relevance"] = json!(rel);
            rat["pillar_match"] = json!(pm);
            // 证据解析成可读条目
            let keys: Vec<String> = r
                .get::<Value, _>("evidence")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut evidence: Vec<String> = Vec::new();
            if !keys.is_empty() {
                let items = sqlx::query(
                    "SELECT source_id, kind, payload FROM intel_items WHERE dedup_key = ANY($1) LIMIT 10",
                )
                .bind(&keys)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
                for it in &items {
                    let p: Value = it.get("payload");
                    let t = p.get("title").and_then(|t| t.as_str()).unwrap_or("");
                    let u = p.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    let s: String = it.get("source_id");
                    if !t.is_empty() {
                        evidence.push(format!("{t} ({s}) {u}"));
                    }
                }
            }
            if evidence.is_empty() {
                evidence = keys;
            }
            out.push(Opportunity {
                topic,
                score,
                rationale: rat,
                evidence,
                status: "new".into(),
            });
        }
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.topic.cmp(&b.topic))
        });
        out.truncate(limit.max(10));
        Ok(out)
    }

    /// skill handler 用: 全文检索 intel_items (research 域 social_search/topic_research/fact_checking)
    async fn search_items(&self, query: &str, limit: usize) -> Result<Vec<Value>, String> {
        let rows = sqlx::query(
            "SELECT source_id, kind, dedup_key, payload, collected_at FROM intel_items \
             WHERE payload::text ILIKE '%' || $1 || '%' \
             ORDER BY collected_at DESC LIMIT $2",
        )
        .bind(query)
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                json!({
                    "source_id": r.get::<String, _>("source_id"),
                    "kind": r.get::<String, _>("kind"),
                    "dedup_key": r.get::<String, _>("dedup_key"),
                    "title": r.get::<Value, _>("payload").get("title").cloned().unwrap_or(Value::Null),
                    "url": r.get::<Value, _>("payload").get("url").cloned().unwrap_or(Value::Null),
                    "collected_at": r.get::<chrono::DateTime<chrono::Utc>, _>("collected_at"),
                })
            })
            .collect())
    }

    /// skill handler 用: 趋势聚合 (trend_discovery), 口径与 GET /intel/trends 一致
    async fn trending(&self, limit: usize) -> Result<Vec<Value>, String> {
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
                      count(DISTINCT source_id)::int8 AS platforms,
                      array_agg(DISTINCT source_id) AS sources
               FROM t
               WHERE length(topic) > 2
               GROUP BY topic
               HAVING count(*) >= 3
               ORDER BY h6 DESC, total DESC
               LIMIT $1"#,
        )
        .bind(limit.max(1) as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .iter()
            .map(|r| {
                json!({
                    "topic": r.get::<String, _>("topic"),
                    "total_24h": r.get::<i64, _>("total"),
                    "last_6h": r.get::<i64, _>("h6"),
                    "platforms": r.get::<i64, _>("platforms"),
                    "sources": r.get::<Vec<String>, _>("sources"),
                })
            })
            .collect())
    }
}

/// skill handler 用: analytics 域的真实运行指标 (devices/tasks/packages/intel)。
pub(crate) struct PgStatsPort {
    pool: PgPool,
}

#[async_trait]
impl sincy_employee::StatsPort for PgStatsPort {
    async fn account_growth(&self, employee_id: Uuid, window_days: i64) -> Result<Value, String> {
        let rows = sqlx::query(
            "SELECT followers, observed_at FROM account_metric_samples
             WHERE employee_id = $1 AND followers IS NOT NULL
             ORDER BY observed_at DESC LIMIT 200",
        )
        .bind(employee_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let latest = rows.first();
        let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days.clamp(1, 365));
        let baseline = rows
            .iter()
            .filter(|r| r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at") <= cutoff)
            .last();
        let now = latest.and_then(|r| r.get::<Option<i64>, _>("followers"));
        let base = baseline.and_then(|r| r.get::<Option<i64>, _>("followers"));
        Ok(json!({
            "employee_id": employee_id,
            "window_days": window_days,
            "samples": rows.len(),
            "followers_now": now,
            "followers_base": base,
            "delta": match (now, base) { (Some(n), Some(b)) => json!(n - b), _ => Value::Null },
            "delta_pct": match (now, base) {
                (Some(n), Some(b)) if b != 0 => json!(((n - b) as f64 / b as f64 * 100.0).round() / 100.0),
                _ => Value::Null,
            },
        }))
    }

    async fn metrics(&self) -> Result<Value, String> {
        let tasks = sqlx::query(
            "SELECT state, count(*)::int8 AS n FROM device_tasks GROUP BY state ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let pkgs = sqlx::query(
            "SELECT approval_status, count(*)::int8 AS n FROM publishing_packages GROUP BY approval_status ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let devices = sqlx::query(
            "SELECT state, count(*)::int8 AS n FROM devices GROUP BY state ORDER BY n DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        let intel_items: i64 = sqlx::query_scalar("SELECT count(*) FROM intel_items")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        let jobs_ran: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM intel_collection_jobs WHERE last_run_at IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(json!({
            "device_tasks_by_state": tasks.iter().map(|r| json!({"state": r.get::<String,_>("state"), "n": r.get::<i64,_>("n")})).collect::<Vec<_>>(),
            "packages_by_approval": pkgs.iter().map(|r| json!({"status": r.get::<String,_>("approval_status"), "n": r.get::<i64,_>("n")})).collect::<Vec<_>>(),
            "devices_by_state": devices.iter().map(|r| json!({"state": r.get::<String,_>("state"), "n": r.get::<i64,_>("n")})).collect::<Vec<_>>(),
            "intel_items_total": intel_items,
            "intel_jobs_ran": jobs_ran,
            "collected_at": chrono::Utc::now(),
        }))
    }
}

pub(crate) struct PgLlmPort {
    allow_mock: bool,
}

impl PgLlmPort {
    pub(crate) fn new(allow_mock: bool) -> Self {
        Self { allow_mock }
    }
}

#[async_trait]
impl LlmPort for PgLlmPort {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        // same provider policy as /ai/generate (mock only when explicitly allowed)
        let gw = sincy_ai::gateway_from_env(self.allow_mock)?;
        let req = sincy_ai::GenerationRequest {
            images: Vec::new(),
            prompt: prompt.to_string(),
            model: std::env::var("SINCY_LLM_MODEL").unwrap_or_else(|_| "mock-1".into()),
            max_tokens,
            content_item_id: None,
        };
        let r = gw.generate(&req).await.map_err(|e| e.to_string())?;
        Ok(r.text)
    }
}

pub(crate) struct PgPublishPort {
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

// ---------- WS2: 员工专属 feed (按 focus 过滤 signals + 机会; general = 全量) ----------
async fn employee_feed(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let identity = load_identity_of(pool, *auth.agency_id.as_uuid(), id).await?;
    let focus = crate::focus::focus_from_identity(&identity);
    // signals: 近 24h, 按 focus 过滤
    let sig_rows = sqlx::query(
        "SELECT signal_id, signal_type, industry_id, topic_id, strength, importance, observed_at, payload FROM intelligence_signals WHERE observed_at > now() - interval '24 hours' ORDER BY importance DESC LIMIT 300",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut signals: Vec<Value> = Vec::new();
    for r in &sig_rows {
        let payload: Value = r.get("payload");
        let industry: Option<String> = r.try_get("industry_id").ok().flatten();
        let text = format!(
            "{} {}",
            payload.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            payload
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        if !focus.is_general() {
            if focus.excluded_hit(&text) {
                continue;
            }
            let tags: Vec<String> = industry.iter().cloned().collect();
            let has_tag_info = !tags.is_empty();
            if has_tag_info && !focus.tags_intersect(&tags) {
                continue;
            }
            if !has_tag_info && !focus.keyword_hit(&text) {
                continue;
            }
        }
        signals.push(json!({
            "signal_id": r.get::<String, _>("signal_id"),
            "signal_type": r.get::<String, _>("signal_type"),
            "industry_id": industry,
            "topic_id": r.try_get::<Option<String>, _>("topic_id").ok().flatten(),
            "strength": r.get::<f64, _>("strength"),
            "importance": r.get::<f64, _>("importance"),
            "observed_at": r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at"),
            "payload": payload,
        }));
        if signals.len() >= 50 {
            break;
        }
    }
    // 机会: 近 7 天未处理, 同样按 focus 硬过滤
    let opp_rows = sqlx::query(
        "SELECT topic, score, rationale, evidence FROM intel_opportunities WHERE status = 'new' AND created_at > now() - interval '7 days' ORDER BY score DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut opps: Vec<Value> = Vec::new();
    for r in &opp_rows {
        let topic: String = r.get("topic");
        let rationale: Value = r.get("rationale");
        let title = rationale
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let haystack = format!("{topic} {title}");
        if !focus.is_general() {
            if focus.excluded_hit(&haystack) {
                continue;
            }
            let keys: Vec<String> = r
                .get::<Value, _>("evidence")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut tags: Vec<String> = rationale
                .get("industry_tags")
                .and_then(|t| t.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if !keys.is_empty()
                && let Ok(rows2) = sqlx::query("SELECT DISTINCT jsonb_array_elements_text(industries) AS t FROM intel_items WHERE dedup_key = ANY($1) AND industries <> '[]'::jsonb").bind(&keys).fetch_all(pool).await {
                    for r2 in &rows2 { tags.push(r2.get::<String, _>("t")); }
                }
            if !tags.is_empty() && !focus.tags_intersect(&tags) {
                continue;
            }
        }
        opps.push(json!({
            "topic": topic,
            "score": r.get::<rust_decimal::Decimal, _>("score").to_string(),
            "rationale": rationale,
        }));
        if opps.len() >= 30 {
            break;
        }
    }
    Ok(Json(json!({
        "employee_id": id,
        "focus": focus.to_json(),
        "filtered": !focus.is_general(),
        "signals": signals,
        "opportunities": opps,
    })))
}

/// 读取员工 active identity (agency 隔离)
async fn load_identity_of(
    pool: &sqlx::PgPool,
    agency: Uuid,
    id: Uuid,
) -> Result<Value, axum::response::Response> {
    let row = sqlx::query(
        "SELECT v.identity FROM digital_employees e JOIN employee_identity_versions v ON v.employee_id = e.id AND v.status = 'active' WHERE e.id = $1 AND e.agency_id = $2 ORDER BY v.version DESC LIMIT 1",
    )
    .bind(id)
    .bind(agency)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    match row {
        Some(r) => Ok(r.get::<Value, _>("identity")),
        None => Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        ))),
    }
}

// ---------- WS3: 领域情报仪表板 ----------
#[derive(Debug, Deserialize)]
struct BoardQuery {
    window: Option<String>,
}

async fn intel_board(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<BoardQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let window = q.window.unwrap_or_else(|| "24h".into());
    if !["8h", "16h", "24h"].contains(&window.as_str()) {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "bad_request",
            "window must be 8h|16h|24h",
        )));
    }
    let identity = load_identity_of(pool, *auth.agency_id.as_uuid(), id).await?;
    let focus = crate::focus::focus_from_identity(&identity);
    let interval = format!("interval '{window}'");
    // 热帖/趋势: 窗口内 signals 按 importance 排序
    let hot_sql = format!(
        "SELECT signal_id, signal_type, industry_id, strength, importance, observed_at, payload FROM intelligence_signals WHERE observed_at > now() - {interval} ORDER BY importance DESC LIMIT 300"
    );
    let hot_rows = sqlx::query(&hot_sql).fetch_all(pool).await.map_err(err)?;
    let mut hot: Vec<Value> = Vec::new();
    let mut viral: Vec<Value> = Vec::new();
    for r in &hot_rows {
        let payload: Value = r.get("payload");
        let industry: Option<String> = r.try_get("industry_id").ok().flatten();
        let stype: String = r.get("signal_type");
        let text = format!(
            "{} {}",
            payload.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            payload
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        if !focus.is_general() {
            if focus.excluded_hit(&text) {
                continue;
            }
            let tags: Vec<String> = industry.iter().cloned().collect();
            if !tags.is_empty() && !focus.tags_intersect(&tags) {
                continue;
            }
            if tags.is_empty() && !focus.keyword_hit(&text) {
                continue;
            }
        }
        let item = json!({"signal_id": r.get::<String,_>("signal_id"), "signal_type": stype, "importance": r.get::<f64,_>("importance"), "observed_at": r.get::<chrono::DateTime<chrono::Utc>,_>("observed_at"), "title": payload.get("title").cloned().unwrap_or(Value::Null), "url": payload.get("url").cloned().unwrap_or(Value::Null)});
        if [
            "trend_rising",
            "content_going_viral",
            "competitor_activity",
            "topic_emerging",
        ]
        .contains(&stype.as_str())
        {
            viral.push(item.clone());
        }
        hot.push(item);
        if hot.len() >= 20 {
            break;
        }
    }
    // Top20 KOL: social_account/creator 实体; 无数据 = provider 未配置, 明确占位
    let kol_rows = if focus.is_general() {
        sqlx::query("SELECT entity_id, title, payload, industry_tags FROM intelligence_entities WHERE entity_type IN ('social_account','creator') ORDER BY updated_at DESC LIMIT 20").fetch_all(pool).await.map_err(err)?
    } else {
        let keys = focus.keys();
        sqlx::query("SELECT entity_id, title, payload, industry_tags FROM intelligence_entities WHERE entity_type IN ('social_account','creator') AND industry_tags ?| $1 ORDER BY updated_at DESC LIMIT 20").bind(&keys).fetch_all(pool).await.map_err(err)?
    };
    let kols: Vec<Value> = kol_rows
        .iter()
        .map(|r| {
            json!({
                "entity_id": r.get::<String,_>("entity_id"),
                "title": r.get::<String,_>("title"),
                "industry_tags": r.get::<Value,_>("industry_tags"),
                "payload": r.get::<Value,_>("payload"),
            })
        })
        .collect();
    let kol_block = if kols.is_empty() {
        json!({"status": "not_configured", "items": [], "note": "pulse creator_ranking/account_growth provider 未配置, 或本领域暂无 KOL 实体"})
    } else {
        json!({"status": "ok", "items": kols})
    };
    // 选题缺口: 窗口内热点话题 ⊖ 该员工近期已发
    let recent_rows = sqlx::query(
        "SELECT p.text_content FROM publishing_packages p JOIN digital_employees e ON e.brand_id = p.brand_id WHERE e.id = $1 ORDER BY p.created_at DESC LIMIT 20",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let recent: Vec<String> = recent_rows.iter().map(|r| r.get::<String, _>(0)).collect();
    let mut gaps: Vec<Value> = Vec::new();
    for h in &hot {
        if let Some(t) = h.get("title").and_then(|v| v.as_str())
            && !t.is_empty()
            && !recent.iter().any(|r| r.contains(t))
        {
            gaps.push(h.clone());
        }
        if gaps.len() >= 10 {
            break;
        }
    }
    Ok(Json(json!({
        "employee_id": id,
        "window": window,
        "focus": focus.to_json(),
        "filtered": !focus.is_general(),
        "hot": hot,
        "viral": viral,
        "kols": kol_block,
        "topic_gaps": gaps,
    })))
}
