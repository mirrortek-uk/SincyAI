//! Skill 注册表 + 员工绑定 API (DE guide: skills 可版本化、动态注册、绑定到员工)。
//!
//! - GET  /api/v1/skills?domains=&q=     skill 列表 (可按域/关键字过滤)
//! - GET  /api/v1/skills/domains         9 个域 + 每域 skill 数量
//! - POST /api/v1/skills                 注册新 skill 或给已有 name 出新版本 (进 skill_versions)
//! - GET  /api/v1/employees/{id}/skills  该员工绑定的 skills
//! - POST /api/v1/employees/{id}/skills  绑定 {skill_id|name, enabled?}
//! - DELETE /api/v1/employees/{id}/skills/{skill_id}  解绑

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use sincy_employee::skills::SkillDomain;

use crate::api::app_error_response;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/skills", get(list_skills).post(register_skill))
        .route("/skills/domains", get(list_domains))
        .route(
            "/employees/{id}/skills",
            get(list_employee_skills).post(bind_skill),
        )
        .route("/employees/{id}/skills/{skill_id}", delete(unbind_skill))
}

fn err(e: sqlx::Error) -> axum::response::Response {
    app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

fn skill_json(r: &sqlx::postgres::PgRow) -> Value {
    let tools: Value = r.get("tools");
    json!({
        "id": r.get::<Uuid, _>("id"),
        "domain": r.get::<String, _>("domain"),
        "name": r.get::<String, _>("name"),
        "version": r.get::<i32, _>("version"),
        "title": r.get::<String, _>("title"),
        "description": r.get::<String, _>("description"),
        "input_schema": r.get::<Value, _>("input_schema"),
        "output_schema": r.get::<Value, _>("output_schema"),
        "tools": tools.as_array().cloned().unwrap_or_default(),
        "status": r.get::<String, _>("status"),
        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
    })
}

#[derive(Debug, Deserialize)]
struct SkillsQuery {
    domain: Option<String>,
    q: Option<String>,
    status: Option<String>,
}

/// GET /api/v1/skills — skill 注册表 (平台级, 对所有租户可见)。
async fn list_skills(
    State(state): State<AppState>,
    Query(q): Query<SkillsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let sql = "SELECT * FROM skills WHERE ($1::text IS NULL OR domain = $1) \
         AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%' OR title ILIKE '%' || $2 || '%') \
         AND ($3::text IS NULL OR status = $3) ORDER BY domain, name";
    let status = match q.status.as_deref() {
        Some("all") | None => None,
        other => other,
    };
    let rows = sqlx::query(sql)
        .bind(q.domain.as_deref())
        .bind(q.q.as_deref())
        .bind(status)
        .fetch_all(pool)
        .await
        .map_err(err)?;
    Ok(Json(
        json!({ "items": rows.iter().map(skill_json).collect::<Vec<_>>() }),
    ))
}

/// GET /api/v1/skills/domains — 9 个 skill 域 + 每域数量 (架构图/动画用)。
async fn list_domains(
    State(state): State<AppState>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let rows = sqlx::query(
        "SELECT domain, count(*)::int8 AS count, array_agg(name ORDER BY name) AS names \
         FROM skills WHERE status = 'active' GROUP BY domain",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut items: Vec<Value> = SkillDomain::ALL
        .iter()
        .map(|d| {
            let row = rows
                .iter()
                .find(|r| r.get::<String, _>("domain") == d.as_str());
            json!({
                "id": d.as_str(),
                "title_zh": d.title_zh(),
                "count": row.map(|r| r.get::<i64, _>("count")).unwrap_or(0),
                "skills": row.map(|r| r.get::<Vec<String>, _>("names")).unwrap_or_default(),
            })
        })
        .collect();
    let total: i64 = items.iter().map(|d| d["count"].as_i64().unwrap_or(0)).sum();
    // 兜底: DB 里可能有新注册的域外数据, 也不丢
    for r in &rows {
        let d = r.get::<String, _>("domain");
        if !items.iter().any(|i| i["id"] == d) {
            items.push(json!({
                "id": d,
                "title_zh": d,
                "count": r.get::<i64, _>("count"),
                "skills": r.get::<Vec<String>, _>("names"),
            }));
        }
    }
    Ok(Json(json!({ "domains": items, "total_skills": total })))
}

#[derive(Debug, Deserialize)]
struct RegisterSkillBody {
    domain: String,
    name: String,
    title: Option<String>,
    description: Option<String>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
    tools: Option<Vec<String>>,
}

/// POST /api/v1/skills — 注册新 skill; 同名 name 自动升版本并留档到 skill_versions。
async fn register_skill(
    State(state): State<AppState>,
    Json(body): Json<RegisterSkillBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let domain = SkillDomain::parse(&body.domain).ok_or_else(|| {
        app_error_response(sincy_common::AppError::non_retryable(
            "invalid_domain",
            format!(
                "domain must be one of {:?}",
                SkillDomain::ALL
                    .iter()
                    .map(|d| d.as_str())
                    .collect::<Vec<_>>()
            ),
        ))
    })?;
    let name = body.name.trim().to_lowercase().replace(' ', "_");
    if name.is_empty() {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "invalid_name",
            "name required",
        )));
    }
    let title = body.title.unwrap_or_else(|| name.clone());
    let description = body.description.unwrap_or_default();
    let input_schema = body.input_schema.unwrap_or_else(|| json!({}));
    let output_schema = body.output_schema.unwrap_or_else(|| json!({}));
    let tools = json!(body.tools.unwrap_or_default());

    let existing: Option<i32> = sqlx::query_scalar("SELECT version FROM skills WHERE name = $1")
        .bind(&name)
        .fetch_optional(pool)
        .await
        .map_err(err)?;

    let (id, version) = match existing {
        Some(v) => {
            // 新版本: 留档旧版本, 升级当前
            let cur: Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM skills s WHERE name = $1")
                .bind(&name)
                .fetch_one(pool)
                .await
                .map_err(err)?;
            sqlx::query(
                "INSERT INTO skill_versions (skill_name, version, spec) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(&name)
            .bind(v)
            .bind(&cur)
            .execute(pool)
            .await
            .map_err(err)?;
            let nv = v + 1;
            let id: Uuid = sqlx::query_scalar(
                "UPDATE skills SET version = $2, domain = $3, title = $4, description = $5, \
                 input_schema = $6, output_schema = $7, tools = $8, status = 'active', updated_at = now() \
                 WHERE name = $1 RETURNING id",
            )
            .bind(&name)
            .bind(nv)
            .bind(domain.as_str())
            .bind(&title)
            .bind(&description)
            .bind(&input_schema)
            .bind(&output_schema)
            .bind(&tools)
            .fetch_one(pool)
            .await
            .map_err(err)?;
            sqlx::query(
                "INSERT INTO skill_versions (skill_name, version, spec) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(&name)
            .bind(nv)
            .bind(json!({"title": title, "description": description, "input_schema": input_schema, "output_schema": output_schema, "tools": tools}))
            .execute(pool)
            .await
            .map_err(err)?;
            (id, nv)
        }
        None => {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO skills (domain, name, version, title, description, input_schema, output_schema, tools) \
                 VALUES ($1, $2, 1, $3, $4, $5, $6, $7) RETURNING id",
            )
            .bind(domain.as_str())
            .bind(&name)
            .bind(&title)
            .bind(&description)
            .bind(&input_schema)
            .bind(&output_schema)
            .bind(&tools)
            .fetch_one(pool)
            .await
            .map_err(err)?;
            sqlx::query(
                "INSERT INTO skill_versions (skill_name, version, spec) VALUES ($1, 1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&name)
            .bind(json!({"title": title, "description": description, "input_schema": input_schema, "output_schema": output_schema, "tools": tools}))
            .execute(pool)
            .await
            .map_err(err)?;
            (id, 1)
        }
    };
    Ok(Json(
        json!({ "id": id, "name": name, "domain": domain.as_str(), "version": version }),
    ))
}

/// 读取员工绑定且启用的 skills (create 时自动绑定全部 active skill)。
pub(crate) async fn load_bound_skills(
    pool: &sqlx::PgPool,
    employee_id: Uuid,
) -> Result<Vec<sincy_employee::skills::Skill>, axum::response::Response> {
    let rows = sqlx::query(
        "SELECT s.* FROM employee_skill_bindings b JOIN skills s ON s.id = b.skill_id \
         WHERE b.employee_id = $1 AND b.enabled = true AND s.status = 'active' ORDER BY s.domain, s.name",
    )
    .bind(employee_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let skills = rows
        .iter()
        .map(|r| sincy_employee::skills::Skill {
            id: r.get("id"),
            domain: r.get("domain"),
            name: r.get("name"),
            version: r.get("version"),
            title: r.get("title"),
            description: r.get("description"),
            tools: r
                .get::<Value, _>("tools")
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            status: r.get("status"),
        })
        .collect();
    Ok(skills)
}

/// 创建员工时把全部 active skills 绑定上去 (可在绑定接口里逐个 disable)。
pub(crate) async fn bind_all_active_skills(
    pool: &sqlx::PgPool,
    employee_id: Uuid,
) -> Result<(), axum::response::Response> {
    sqlx::query(
        "INSERT INTO employee_skill_bindings (employee_id, skill_id) \
         SELECT $1, id FROM skills WHERE status = 'active' ON CONFLICT DO NOTHING",
    )
    .bind(employee_id)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(())
}

async fn owns_employee(
    pool: &sqlx::PgPool,
    id: Uuid,
    agency: &Uuid,
) -> Result<bool, axum::response::Response> {
    let owns: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM digital_employees WHERE id = $1 AND agency_id = $2)",
    )
    .bind(id)
    .bind(agency)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    Ok(owns.unwrap_or(false))
}

/// GET /api/v1/employees/{id}/skills — 员工绑定的 skills (含启停状态)。
async fn list_employee_skills(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    if !owns_employee(pool, id, auth.agency_id.as_uuid()).await? {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let rows = sqlx::query(
        "SELECT s.*, b.enabled, b.bound_at FROM employee_skill_bindings b JOIN skills s ON s.id = b.skill_id \
         WHERE b.employee_id = $1 ORDER BY s.domain, s.name",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = skill_json(r);
            v["enabled"] = json!(r.get::<bool, _>("enabled"));
            v["bound_at"] = json!(r.get::<chrono::DateTime<chrono::Utc>, _>("bound_at"));
            v
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

#[derive(Debug, Deserialize)]
struct BindBody {
    skill_id: Option<Uuid>,
    name: Option<String>,
    enabled: Option<bool>,
}

/// POST /api/v1/employees/{id}/skills — 绑定 skill (按 id 或 name), 可 enable/disable。
async fn bind_skill(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path(id): Path<Uuid>,
    Json(body): Json<BindBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    if !owns_employee(pool, id, auth.agency_id.as_uuid()).await? {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    let skill_id = match (body.skill_id, body.name) {
        (Some(sid), _) => sid,
        (None, Some(name)) => sqlx::query_scalar("SELECT id FROM skills WHERE name = $1")
            .bind(name.trim().to_lowercase().replace(' ', "_"))
            .fetch_optional(pool)
            .await
            .map_err(err)?
            .ok_or_else(|| {
                app_error_response(sincy_common::AppError::non_retryable(
                    "skill_not_found",
                    "no skill with that name",
                ))
            })?,
        _ => {
            return Err(app_error_response(sincy_common::AppError::non_retryable(
                "bad_request",
                "provide skill_id or name",
            )));
        }
    };
    let enabled = body.enabled.unwrap_or(true);
    sqlx::query(
        "INSERT INTO employee_skill_bindings (employee_id, skill_id, enabled) VALUES ($1, $2, $3) \
         ON CONFLICT (employee_id, skill_id) DO UPDATE SET enabled = EXCLUDED.enabled",
    )
    .bind(id)
    .bind(skill_id)
    .bind(enabled)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(Json(
        json!({ "employee_id": id, "skill_id": skill_id, "enabled": enabled }),
    ))
}

/// DELETE /api/v1/employees/{id}/skills/{skill_id} — 解绑。
async fn unbind_skill(
    State(state): State<AppState>,
    axum::extract::Extension(auth): axum::extract::Extension<crate::auth::AuthCtx>,
    Path((id, skill_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(app_error_response)?;
    if !owns_employee(pool, id, auth.agency_id.as_uuid()).await? {
        return Err(app_error_response(sincy_common::AppError::non_retryable(
            "not_found",
            "employee not found",
        )));
    }
    sqlx::query("DELETE FROM employee_skill_bindings WHERE employee_id = $1 AND skill_id = $2")
        .bind(id)
        .bind(skill_id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(Json(json!({ "unbound": true })))
}
