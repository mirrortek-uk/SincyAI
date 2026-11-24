//! 全平台 KPI / 配额规格 (claude/kpi-schema.md v1) 的运行时实现。
//!
//! 两级生效: kpi_categories(档) ⊕ platform_kpi_templates(平台) ⊕ account_kpi.overrides(账号) = effective
//!   - GET  /kpi/categories            6 档分类模板
//!   - GET  /kpi/platforms             平台画像列表 (?category= 过滤)
//!   - GET  /kpi/platforms/{p}         单平台完整模板
//!   - GET  /kpi/effective?platform=&account_id=&employee_id=   本账号实际 KPI (未建档 = 模板值)
//!   - GET  /kpi/accounts               本机构全部账号 KPI
//!   - PUT  /kpi/accounts               写账号覆盖并重算 effective
//!   - GET  /kpi/ledger/today?platform=&account_id=   今日各动作已用/配额/剩余
//!   - POST /kpi/ledger/record          记一笔动作并做压顶检查 (供派发循环调用, 幂等累加)
//!   - GET  /kpi/plan?platform=&account_id=           今日计划 (配额 - 已用 = 剩余, 含时段)
//!
//! 优先级 D > B > A > C: record 时若健康红线命中 (ledger 侧由健康信号表/调用方标记) 或
//! 日总上限/小时限速到顶, 返回 blocked=true, 调用方必须停派。

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::Row;

use crate::auth::AuthCtx;
use crate::state::AppState;

fn err(e: impl ToString) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/kpi/categories", get(list_categories))
        .route("/kpi/platforms", get(list_platforms))
        .route("/kpi/platforms/{p}", get(get_platform))
        .route("/kpi/effective", get(get_effective))
        .route("/kpi/accounts", get(list_accounts).put(put_account))
        .route("/kpi/ledger/today", get(ledger_today))
        .route("/kpi/ledger/record", post(ledger_record))
        .route("/kpi/plan", get(daily_plan))
}

/// 深合并: 委托给 sincy_content::voice::deep_merge (统一实现, voice/KPI 共用)
pub fn deep_merge(base: &Value, over: &Value) -> Value {
    sincy_content::voice::deep_merge(base, over)
}

/// deep_merge 但 over 为 Null 时保留 base (空 overrides 不擦掉模板)
fn merge_opt(base: &Value, over: &Value) -> Value {
    sincy_content::voice::merge_opt(base, over)
}

/// 算 effective: category 默认 ⊕ platform 模板 ⊕ account overrides
pub async fn compute_effective(
    pool: &sqlx::PgPool,
    platform: &str,
    overrides: &Value,
) -> Result<Value, String> {
    let tpl = sqlx::query(
        "SELECT t.category, t.growth_metric, t.engagement_formula, t.available_actions,
                t.default_quotas, t.secondary_metrics, t.health_signals, t.rate_caps,
                c.default_quotas AS cat_quotas, c.quality_gates, c.health_guardrails
         FROM platform_kpi_templates t
         JOIN kpi_categories c ON c.category = t.category
         WHERE t.platform = $1",
    )
    .bind(platform)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("unknown platform {platform}"))?;

    let cat_quotas: Value = tpl.get("cat_quotas");
    let tpl_quotas: Value = tpl.get("default_quotas");
    let gates: Value = tpl.get("quality_gates");
    let cat_guard: Value = tpl.get("health_guardrails");
    let rate_caps: Value = tpl.get("rate_caps");

    let quotas = merge_opt(
        &deep_merge(&cat_quotas, &tpl_quotas),
        &overrides["activity_quotas"],
    );
    let guard = merge_opt(
        &deep_merge(&cat_guard, &rate_caps),
        &overrides["health_guardrails"],
    );
    Ok(json!({
        "platform": platform,
        "category": tpl.get::<String, _>("category"),
        "growth_metric": tpl.get::<String, _>("growth_metric"),
        "engagement_formula": tpl.get::<String, _>("engagement_formula"),
        "available_actions": tpl.get::<Vec<String>, _>("available_actions"),
        "secondary_metrics": tpl.get::<Vec<String>, _>("secondary_metrics"),
        "health_signals": tpl.get::<Vec<String>, _>("health_signals"),
        "activity_quotas": quotas,
        "quality_gates": merge_opt(&gates, &overrides["quality_gates"]),
        "growth_targets": overrides.get("growth_targets").cloned().unwrap_or(json!({"metric": tpl.get::<String,_>("growth_metric")})),
        "health_guardrails": guard,
    }))
}

async fn list_categories(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let _ = auth;
    let pool = state.pool().map_err(err)?;
    let rows = sqlx::query(
        "SELECT category, label, growth_metric, engagement_formula, available_actions, default_quotas, quality_gates, health_guardrails FROM kpi_categories ORDER BY category",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    Ok(Json(json!({ "items": rows.iter().map(|r| json!({
        "category": r.get::<String,_>("category"),
        "label": r.get::<String,_>("label"),
        "growth_metric": r.get::<String,_>("growth_metric"),
        "engagement_formula": r.get::<String,_>("engagement_formula"),
        "available_actions": r.get::<Vec<String>,_>("available_actions"),
        "default_quotas": r.get::<Value,_>("default_quotas"),
        "quality_gates": r.get::<Value,_>("quality_gates"),
        "health_guardrails": r.get::<Value,_>("health_guardrails"),
    })).collect::<Vec<_>>() })))
}

#[derive(serde::Deserialize)]
struct PlatformQuery {
    category: Option<String>,
}

async fn list_platforms(
    State(state): State<AppState>,
    Query(q): Query<PlatformQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let rows = sqlx::query(
        "SELECT platform, label, category, growth_metric, engagement_formula, available_actions,
                default_quotas, secondary_metrics, health_signals, rate_caps, adapter_requirements
         FROM platform_kpi_templates
         WHERE ($1::text IS NULL OR category = $1)
         ORDER BY category, platform",
    )
    .bind(&q.category)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    Ok(Json(json!({ "items": rows.iter().map(|r| json!({
        "platform": r.get::<String,_>("platform"),
        "label": r.get::<String,_>("label"),
        "category": r.get::<String,_>("category"),
        "growth_metric": r.get::<String,_>("growth_metric"),
        "engagement_formula": r.get::<String,_>("engagement_formula"),
        "available_actions": r.get::<Vec<String>,_>("available_actions"),
        "default_quotas": r.get::<Value,_>("default_quotas"),
        "secondary_metrics": r.get::<Vec<String>,_>("secondary_metrics"),
        "health_signals": r.get::<Vec<String>,_>("health_signals"),
        "rate_caps": r.get::<Value,_>("rate_caps"),
        "adapter_requirements": r.get::<Vec<String>,_>("adapter_requirements"),
    })).collect::<Vec<_>>() })))
}

async fn get_platform(
    State(state): State<AppState>,
    Path(p): Path<String>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let eff = compute_effective(pool, &p, &json!({})).await.map_err(|e| {
        crate::api::app_error_response(sincy_common::AppError::non_retryable("not_found", e))
    })?;
    Ok(Json(eff))
}

#[derive(serde::Deserialize)]
struct EffectiveQuery {
    platform: String,
    account_id: String,
    employee_id: Option<uuid::Uuid>,
}

async fn get_effective(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<EffectiveQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let row = sqlx::query(
        "SELECT overrides, effective FROM account_kpi
         WHERE agency_id = $1 AND platform = $2 AND account_id = $3",
    )
    .bind(auth.agency_id.as_uuid())
    .bind(&q.platform)
    .bind(&q.account_id)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    let (overrides, effective) = match row {
        Some(r) => (
            r.get::<Value, _>("overrides"),
            r.get::<Value, _>("effective"),
        ),
        None => (json!({}), Value::Null),
    };
    let eff = if effective.is_null() {
        compute_effective(pool, &q.platform, &overrides)
            .await
            .map_err(|e| {
                crate::api::app_error_response(sincy_common::AppError::non_retryable(
                    "not_found",
                    e,
                ))
            })?
    } else {
        effective
    };
    Ok(Json(
        json!({ "platform": q.platform, "account_id": q.account_id, "employee_id": q.employee_id, "overrides": overrides, "effective": eff }),
    ))
}

async fn list_accounts(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let rows = sqlx::query(
        "SELECT employee_id, platform, account_id, overrides, effective, updated_at
         FROM account_kpi WHERE agency_id = $1 ORDER BY platform, account_id",
    )
    .bind(auth.agency_id.as_uuid())
    .fetch_all(pool)
    .await
    .map_err(err)?;
    Ok(Json(json!({ "items": rows.iter().map(|r| json!({
        "employee_id": r.get::<Option<uuid::Uuid>,_>("employee_id"),
        "platform": r.get::<String,_>("platform"),
        "account_id": r.get::<String,_>("account_id"),
        "overrides": r.get::<Value,_>("overrides"),
        "effective": r.get::<Value,_>("effective"),
        "updated_at": r.get::<chrono::DateTime<chrono::Utc>,_>("updated_at"),
    })).collect::<Vec<_>>() })))
}

#[derive(serde::Deserialize)]
struct PutAccount {
    employee_id: Option<uuid::Uuid>,
    platform: String,
    account_id: String,
    overrides: Value,
}

async fn put_account(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Json(body): Json<PutAccount>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    // 平台必须存在
    let eff = compute_effective(pool, &body.platform, &body.overrides)
        .await
        .map_err(|e| {
            crate::api::app_error_response(sincy_common::AppError::non_retryable("bad_request", e))
        })?;
    sqlx::query(
        "INSERT INTO account_kpi (agency_id, employee_id, platform, account_id, overrides, effective)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (agency_id, platform, account_id) DO UPDATE SET
           employee_id = EXCLUDED.employee_id, overrides = EXCLUDED.overrides,
           effective = EXCLUDED.effective, updated_at = now()",
    )
    .bind(agency)
    .bind(body.employee_id)
    .bind(&body.platform)
    .bind(&body.account_id)
    .bind(&body.overrides)
    .bind(&eff)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(Json(
        json!({ "platform": body.platform, "account_id": body.account_id, "effective": eff }),
    ))
}

#[derive(serde::Deserialize)]
struct LedgerQuery {
    platform: String,
    account_id: String,
}

/// 今日 ledger vs effective 配额
async fn ledger_today(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<LedgerQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let eff = get_effective(
        State(state.clone()),
        Extension(auth.clone()),
        Query(EffectiveQuery {
            platform: q.platform.clone(),
            account_id: q.account_id.clone(),
            employee_id: None,
        }),
    )
    .await?;
    let eff = eff.0;
    let quotas = &eff["effective"]["activity_quotas"];
    let rows = sqlx::query(
        "SELECT action, count FROM account_activity_ledger
         WHERE platform = $1 AND account_id = $2 AND date = CURRENT_DATE",
    )
    .bind(&q.platform)
    .bind(&q.account_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mut usage: serde_json::Map<String, Value> = serde_json::Map::new();
    for r in &rows {
        let key = quota_key_for(&r.get::<String, _>("action")).to_string();
        let n = r.get::<i32, _>("count") as i64;
        let acc = usage.get(&key).and_then(|v| v.as_i64()).unwrap_or(0) + n;
        usage.insert(key, json!(acc));
    }
    let mut actions = serde_json::Map::new();
    if let Some(obj) = quotas.as_object() {
        for (k, qv) in obj {
            let quota = qv.as_i64();
            let used = usage.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
            actions.insert(
                k.clone(),
                json!({
                    "quota": quota,
                    "used": used,
                    "remaining": quota.map(|q| q - used),
                    "disabled": quota == Some(0),
                    "not_applicable": qv.is_null(),
                }),
            );
        }
    }
    let daily_total: i64 = rows.iter().map(|r| r.get::<i32, _>("count") as i64).sum();
    let ceiling = eff["effective"]["health_guardrails"]["daily_action_ceiling"].as_i64();
    Ok(Json(json!({
        "platform": q.platform, "account_id": q.account_id, "date": chrono::Utc::now().date_naive(),
        "actions": actions,
        "daily_total": daily_total,
        "daily_ceiling": ceiling,
        "ceiling_remaining": ceiling.map(|c| c - daily_total),
        "ceiling_blocked": ceiling.map(|c| daily_total >= c).unwrap_or(false),
    })))
}

#[derive(serde::Deserialize)]
struct RecordBody {
    platform: String,
    account_id: String,
    action: String,
    employee_id: Option<uuid::Uuid>,
    /// 健康信号命中 (由调用方根据健康源判定): 命中且 stop_on_* 开启 => 拒绝本次动作
    health_signal: Option<String>,
}

/// 动作名 → 配额 key (record 与 today 视图共用, 保持一致)
pub fn quota_key_for(action: &str) -> &str {
    match action {
        "post" | "publish" => "posts_per_day",
        "reply" | "comment" => "replies_per_day",
        "follow" => "follows_per_day",
        "like" | "upvote" | "react" => "likes_per_day",
        "repost" | "quote" | "crosspost" | "boost" => "reposts_per_day",
        "dm" => "dms_per_day",
        other => other,
    }
}

/// 记一笔动作 + 压顶检查 (HTTP handler 与后台循环共用)。
/// 返回 JSON: blocked=true 时调用方必须停派该账号今日动作。
pub async fn record_action(
    pool: &sqlx::PgPool,
    agency: uuid::Uuid,
    employee_id: Option<uuid::Uuid>,
    platform: &str,
    account_id: &str,
    action: &str,
    health_signal: Option<&str>,
) -> Result<Value, String> {
    let body = RecordBody {
        platform: platform.to_string(),
        account_id: account_id.to_string(),
        action: action.to_string(),
        employee_id,
        health_signal: health_signal.map(|s| s.to_string()),
    };
    let v = record_action_inner(pool, agency, &body).await?;
    Ok(v)
}

async fn ledger_record(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Json(body): Json<RecordBody>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let agency = *auth.agency_id.as_uuid();
    let v = record_action_inner(pool, agency, &body)
        .await
        .map_err(|e| crate::api::app_error_response(sincy_common::AppError::retryable("db", e)))?;
    Ok(Json(v))
}

async fn record_action_inner(
    pool: &sqlx::PgPool,
    agency: uuid::Uuid,
    body: &RecordBody,
) -> Result<Value, String> {
    let eff = compute_effective(pool, &body.platform, &json!({})).await?;
    let guard = &eff["health_guardrails"];
    let quotas = &eff["activity_quotas"];

    // D 优先: 健康信号命中且对应 stop_on_* => 拒绝
    if let Some(sig) = &body.health_signal {
        let stop = match sig.as_str() {
            "shadowban" => guard["stop_on_shadowban"].as_bool().unwrap_or(true),
            "rate_limit" | "rate_limit_429" => {
                guard["stop_on_rate_limit"].as_bool().unwrap_or(true)
            }
            "policy_strike" | "policy_violation" | "community_strike" => {
                guard["stop_on_policy_strike"].as_bool().unwrap_or(true)
            }
            _ => false,
        };
        if stop {
            return Ok(json!({
                "blocked": true, "reason": format!("health_signal:{sig}"),
                "cooldown_minutes": guard["cooldown_minutes_after_block"],
            }));
        }
    }

    // A 配额: 该动作 0=禁用 / null=平台无此动作
    let qv = &quotas[quota_key_for(&body.action)];
    if qv.is_null() {
        return Ok(
            json!({ "blocked": true, "reason": "action_not_applicable_on_platform", "action": body.action }),
        );
    }
    let quota = qv.as_i64().unwrap_or(-1);
    if quota == 0 {
        return Ok(json!({ "blocked": true, "reason": "action_disabled", "action": body.action }));
    }

    // D 日总上限压顶
    let daily: Option<(i64,)> = sqlx::query_as(
        "SELECT COALESCE(SUM(count),0)::bigint FROM account_activity_ledger
         WHERE platform = $1 AND account_id = $2 AND date = CURRENT_DATE",
    )
    .bind(&body.platform)
    .bind(&body.account_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let daily_total = daily.map(|d| d.0).unwrap_or(0);
    let ceiling = guard["daily_action_ceiling"].as_i64().unwrap_or(i64::MAX);
    if daily_total >= ceiling {
        return Ok(
            json!({ "blocked": true, "reason": "daily_ceiling_reached", "daily_total": daily_total, "ceiling": ceiling }),
        );
    }

    // 单动作配额
    let cur: Option<(i32,)> = sqlx::query_as(
        "SELECT count FROM account_activity_ledger
         WHERE platform = $1 AND account_id = $2 AND date = CURRENT_DATE AND action = $3",
    )
    .bind(&body.platform)
    .bind(&body.account_id)
    .bind(&body.action)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let used = cur.map(|c| c.0 as i64).unwrap_or(0);
    if quota >= 0 && used >= quota {
        return Ok(
            json!({ "blocked": true, "reason": "action_quota_reached", "action": body.action, "used": used, "quota": quota }),
        );
    }

    sqlx::query(
        "INSERT INTO account_activity_ledger (agency_id, employee_id, platform, account_id, action, count)
         VALUES ($1, $2, $3, $4, $5, 1)
         ON CONFLICT (platform, account_id, date, action) DO UPDATE SET count = account_activity_ledger.count + 1",
    )
    .bind(agency)
    .bind(body.employee_id)
    .bind(&body.platform)
    .bind(&body.account_id)
    .bind(&body.action)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "blocked": false,
        "action": body.action,
        "used": used + 1,
        "quota": if quota < 0 { Value::Null } else { json!(quota) },
        "daily_total": daily_total + 1,
        "ceiling": ceiling,
    }))
}

#[derive(serde::Deserialize)]
struct PlanQuery {
    platform: String,
    account_id: String,
}

/// 今日计划: 配额 - 已用 = 剩余 + 发布时段 (供调度器摊任务)
async fn daily_plan(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<PlanQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let today = ledger_today(
        State(state.clone()),
        Extension(auth.clone()),
        Query(LedgerQuery {
            platform: q.platform.clone(),
            account_id: q.account_id.clone(),
        }),
    )
    .await?;
    let t = today.0;
    let eff = get_effective(
        State(state),
        Extension(auth),
        Query(EffectiveQuery {
            platform: q.platform.clone(),
            account_id: q.account_id.clone(),
            employee_id: None,
        }),
    )
    .await?;
    let quotas = &eff.0["effective"]["activity_quotas"];
    Ok(Json(json!({
        "platform": q.platform,
        "account_id": q.account_id,
        "date": t["date"],
        "posting_windows": quotas["posting_windows"],
        "timezone": quotas["timezone"],
        "actions": t["actions"],
        "daily_total": t["daily_total"],
        "ceiling_blocked": t["ceiling_blocked"],
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overrides_win() {
        let base = json!({"a": 1, "b": {"x": 1, "y": 2}, "c": [1, 2]});
        let over = json!({"b": {"y": 9}, "c": [3]});
        let m = deep_merge(&base, &over);
        assert_eq!(m["a"], 1);
        assert_eq!(m["b"]["x"], 1);
        assert_eq!(m["b"]["y"], 9);
        assert_eq!(m["c"], json!([3]));
    }

    #[test]
    fn merge_opt_keeps_base_on_null() {
        let base = json!({"posts_per_day": 3});
        // 空 overrides (缺字段 => Null) 不擦掉模板
        assert_eq!(merge_opt(&base, &json!({})["activity_quotas"]), base);
        // 显式 null 字段确实覆盖为 null (平台无此动作语义)
        assert_eq!(
            deep_merge(&base, &json!({"posts_per_day": null}))["posts_per_day"],
            Value::Null
        );
    }
}
