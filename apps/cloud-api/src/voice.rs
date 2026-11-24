//! Voice Profile(语气档案) API — 数字员工「文字习惯 / 语气」的两级存储 + 蒸馏。
//!
//! 设计见 设计书/Sincy-Voice-Profile.md; 存储: 基线 identity->'voice_profile' ⊕ 覆盖 employee_accounts.voice_profile (0047)。
//!
//!   - GET  /voice/employee/{id}            员工基线 (voice 单字段 + voice_profile)
//!   - PUT  /voice/employee/{id}            写基线 (identity 版本化: supersede + 新 active 版本)
//!   - GET  /voice/accounts?employee_id=    本机构账号的语气覆盖列表
//!   - PUT  /voice/account                  写账号覆盖 (upsert employee_accounts.voice_profile)
//!   - GET  /voice/effective?employee_id=&platform=&account_id=   有效语气 + 编译后的 prompt 指令块预览
//!   - POST /voice/distill                  蒸馏: 粘贴采样帖子 → LLM 压成结构化 voice_profile

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;

use crate::auth::AuthCtx;
use crate::state::AppState;

fn err(e: impl ToString) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

fn not_found(msg: &str) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::non_retryable("not_found", msg))
}

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/voice/employee/{id}",
            get(get_employee_voice).put(put_employee_voice),
        )
        .route("/voice/accounts", get(list_account_voices))
        .route("/voice/account", axum::routing::put(put_account_voice))
        .route("/voice/effective", get(get_effective))
        .route("/voice/distill", post(distill))
}

async fn load_employee_identity(
    pool: &sqlx::PgPool,
    agency: uuid::Uuid,
    id: uuid::Uuid,
) -> Result<Value, axum::response::Response> {
    let row = sqlx::query(
        "SELECT e.id, v.identity FROM digital_employees e
         LEFT JOIN LATERAL (
           SELECT identity FROM employee_identity_versions
           WHERE employee_id = e.id AND status = 'active' ORDER BY version DESC LIMIT 1
         ) v ON true
         WHERE e.id = $1 AND e.agency_id = $2",
    )
    .bind(id)
    .bind(agency)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    match row {
        Some(r) => Ok(r.get::<Value, _>("identity")),
        None => Err(not_found("employee not found in this agency")),
    }
}

async fn get_employee_voice(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let identity = load_employee_identity(pool, *auth.agency_id.as_uuid(), id).await?;
    Ok(Json(json!({
        "employee_id": id,
        "voice": identity.get("voice").cloned().unwrap_or(Value::Null),
        "voice_profile": identity.get("voice_profile").cloned().unwrap_or(json!({})),
    })))
}

#[derive(Deserialize)]
struct EmployeeVoicePatch {
    voice: Option<String>,
    voice_profile: Option<Value>,
}

async fn put_employee_voice(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<EmployeeVoicePatch>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    let cur = load_employee_identity(pool, agency, id).await?;
    let mut identity = cur;
    let version = identity
        .get("version")
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as i32;
    if let Some(v) = &body.voice {
        identity["voice"] = json!(v);
    }
    if let Some(v) = &body.voice_profile {
        identity["voice_profile"] = v.clone();
    }
    let mut tx = pool.begin().await.map_err(err)?;
    sqlx::query("UPDATE employee_identity_versions SET status = 'superseded' WHERE employee_id = $1 AND status = 'active'")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("INSERT INTO employee_identity_versions (employee_id, version, identity, status) VALUES ($1, $2, $3, 'active')")
        .bind(id)
        .bind(version + 1)
        .bind(&identity)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(Json(
        json!({ "employee_id": id, "version": version + 1, "voice_profile": identity.get("voice_profile").cloned().unwrap_or(json!({})) }),
    ))
}

#[derive(Deserialize)]
struct AccountsQuery {
    employee_id: Option<uuid::Uuid>,
}

async fn list_account_voices(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<AccountsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    let rows = match q.employee_id {
        Some(eid) => sqlx::query(
            "SELECT id, employee_id, platform, account_id, voice_profile, created_at FROM employee_accounts WHERE agency_id = $1 AND employee_id = $2 ORDER BY platform, account_id",
        )
        .bind(agency)
        .bind(eid)
        .fetch_all(pool)
        .await
        .map_err(err)?,
        None => sqlx::query(
            "SELECT id, employee_id, platform, account_id, voice_profile, created_at FROM employee_accounts WHERE agency_id = $1 ORDER BY platform, account_id",
        )
        .bind(agency)
        .fetch_all(pool)
        .await
        .map_err(err)?,
    };
    Ok(Json(json!({ "items": rows.iter().map(|r| json!({
        "id": r.get::<uuid::Uuid, _>("id"),
        "employee_id": r.get::<uuid::Uuid, _>("employee_id"),
        "platform": r.get::<String, _>("platform"),
        "account_id": r.get::<String, _>("account_id"),
        "voice_profile": r.get::<Option<Value>, _>("voice_profile").unwrap_or(json!({})),
    })).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
struct AccountVoicePut {
    employee_id: Option<uuid::Uuid>,
    platform: String,
    account_id: String,
    voice_profile: Value,
}

async fn put_account_voice(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Json(body): Json<AccountVoicePut>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    // upsert: 已有行(属本机构)直接更新; 不存在则建新行(必须带 employee_id 且属本机构)
    let updated = sqlx::query(
        "UPDATE employee_accounts SET voice_profile = $3 WHERE platform = $1 AND account_id = $2 AND agency_id = $4",
    )
    .bind(&body.platform)
    .bind(&body.account_id)
    .bind(&body.voice_profile)
    .bind(agency)
    .execute(pool)
    .await
    .map_err(err)?;
    if updated.rows_affected() == 0 {
        let eid = body.employee_id.ok_or_else(|| {
            crate::api::app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "employee_id is required when creating a new account row",
            ))
        })?;
        let owns = sqlx::query("SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2")
            .bind(eid)
            .bind(agency)
            .fetch_optional(pool)
            .await
            .map_err(err)?;
        if owns.is_none() {
            return Err(not_found("employee not found in this agency"));
        }
        sqlx::query(
            "INSERT INTO employee_accounts (agency_id, employee_id, platform, account_id, voice_profile) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (platform, account_id) DO UPDATE SET voice_profile = EXCLUDED.voice_profile",
        )
        .bind(agency)
        .bind(eid)
        .bind(&body.platform)
        .bind(&body.account_id)
        .bind(&body.voice_profile)
        .execute(pool)
        .await
        .map_err(err)?;
    }
    Ok(Json(
        json!({ "platform": body.platform, "account_id": body.account_id, "saved": true }),
    ))
}

#[derive(Deserialize)]
struct EffectiveQuery {
    employee_id: uuid::Uuid,
    platform: String,
    account_id: Option<String>,
}

async fn get_effective(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<EffectiveQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    // 基线: 员工 identity.voice_profile (校验属本机构)
    let owns = sqlx::query("SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2")
        .bind(q.employee_id)
        .bind(agency)
        .fetch_optional(pool)
        .await
        .map_err(err)?;
    if owns.is_none() {
        return Err(not_found("employee not found in this agency"));
    }
    let base: Value = sqlx::query_scalar(
        "SELECT COALESCE(identity->'voice_profile', '{}'::jsonb) FROM employee_identity_versions WHERE employee_id = $1 AND status = 'active' ORDER BY version DESC LIMIT 1",
    )
    .bind(q.employee_id)
    .fetch_optional(pool)
    .await
    .map_err(err)?
    .unwrap_or_else(|| json!({}));
    // 覆盖: 优先 account_id, 否则 employee+platform 最新一行
    let over: Value = match &q.account_id {
        Some(aid) => sqlx::query_scalar(
            "SELECT voice_profile FROM employee_accounts WHERE agency_id = $1 AND platform = $2 AND account_id = $3",
        )
        .bind(agency)
        .bind(&q.platform)
        .bind(aid)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .flatten()
        .unwrap_or(Value::Null),
        None => sqlx::query_scalar(
            "SELECT voice_profile FROM employee_accounts WHERE agency_id = $1 AND employee_id = $2 AND platform = $3 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(agency)
        .bind(q.employee_id)
        .bind(&q.platform)
        .fetch_optional(pool)
        .await
        .map_err(err)?
        .flatten()
        .unwrap_or(Value::Null),
    };
    let effective = sincy_content::voice::effective_voice(&base, &over);
    let block = sincy_content::voice::compile_voice_prompt(&effective, 3);
    Ok(Json(json!({
        "base": base,
        "account_override": over,
        "effective": effective,
        "prompt_block": block,
    })))
}

#[derive(Deserialize)]
struct DistillReq {
    samples: Vec<String>,
    #[serde(default)]
    handles: Vec<String>,
}

async fn distill(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Json(body): Json<DistillReq>,
) -> Result<Json<Value>, axum::response::Response> {
    let _ = auth;
    let _ = state;
    if body.samples.iter().filter(|s| !s.trim().is_empty()).count() < 3 {
        return Err(crate::api::app_error_response(
            sincy_common::AppError::non_retryable("bad_request", "need at least 3 sample posts"),
        ));
    }
    let prompt = sincy_content::voice::distill_prompt(&body.samples, &body.handles);
    let allow_mock = std::env::var("SINCY_ENV").as_deref() != Ok("production");
    let llm = crate::employees::PgLlmPort::new(allow_mock);
    let raw = sincy_employee::cycle::LlmPort::generate(&llm, &prompt, 1024)
        .await
        .map_err(|e| crate::api::app_error_response(sincy_common::AppError::retryable("llm", e)))?;
    let profile = sincy_content::voice::parse_distill(&raw, "pg-llm").map_err(|e| {
        crate::api::app_error_response(sincy_common::AppError::retryable("parse", e))
    })?;
    Ok(Json(
        json!({ "profile": profile, "prompt_block": sincy_content::voice::compile_voice_prompt(&profile, 3) }),
    ))
}
