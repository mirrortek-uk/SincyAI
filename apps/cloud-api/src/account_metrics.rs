//! 粉丝数测量回流 patch-A: account_metrics_loop 后台调度 + 样本写入/查询 API。
//!
//! 数据流: account_metrics_loop 每 N 小时对每个 RUNNING 员工的绑定账号派发
//! read_followers 设备任务 (走现有 execution_routes/device_tasks/verify 机器),
//! 设备回执里的 captured 值 (task_events.detail) 被 harvest 成 account_metric_samples 快照;
//! 有 API 的平台也可走 POST /intel/account-metrics 直接灌样本 (source='api')。
//! 涨粉曲线/增量一律按窗口从样本现算 (StatsPort::account_growth)。
//!
//! 表不走 FORCE RLS, 显式 agency_id/account_id scope (后台循环无租户上下文)。

use axum::{
    Json,
    extract::{Extension, Query, State},
};
use serde_json::{Value, json};
use sqlx::Row;

use crate::auth::AuthCtx;
use crate::state::AppState;

fn err(e: impl ToString) -> axum::response::Response {
    crate::api::app_error_response(sincy_common::AppError::retryable("db", e.to_string()))
}

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/intel/account-metrics",
        axum::routing::post(ingest_sample).get(list_samples),
    )
}

/// POST /intel/account-metrics — 样本灌入 (API route / 测试 / 设备回流补录), 租户隔离
pub async fn ingest_sample(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let platform = body["platform"].as_str().unwrap_or("").to_string();
    let account_id = body["account_id"].as_str().unwrap_or("").to_string();
    if platform.is_empty() || account_id.is_empty() {
        return Err(crate::api::app_error_response(
            sincy_common::AppError::non_retryable("bad_request", "platform/account_id required"),
        ));
    }
    let employee_id: Option<uuid::Uuid> = body["employee_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let brand_id: Option<uuid::Uuid> = body["brand_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let source = body["source"].as_str().unwrap_or("api");
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO account_metric_samples
           (agency_id, brand_id, employee_id, platform, account_id, followers, following, posts_count, extras, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id",
    )
    .bind(auth.agency_id.as_uuid())
    .bind(brand_id)
    .bind(employee_id)
    .bind(&platform)
    .bind(&account_id)
    .bind(body["followers"].as_i64())
    .bind(body["following"].as_i64())
    .bind(body["posts_count"].as_i64())
    .bind(body.get("extras").cloned().unwrap_or(json!({})))
    .bind(source)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    Ok(Json(json!({ "id": id })))
}

/// GET /intel/account-metrics?platform=x&account_id=..&window_days=7 — 曲线 + 窗口增量
pub async fn list_samples(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthCtx>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, axum::response::Response> {
    let pool = state.pool().map_err(err)?;
    let platform = q.get("platform").cloned().unwrap_or_default();
    let account_id = q.get("account_id").cloned().unwrap_or_default();
    let window_days: i64 = q
        .get("window_days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7)
        .clamp(1, 365);
    if platform.is_empty() || account_id.is_empty() {
        return Err(crate::api::app_error_response(
            sincy_common::AppError::non_retryable("bad_request", "platform/account_id required"),
        ));
    }
    let rows = sqlx::query(
        "SELECT id, employee_id, brand_id, followers, following, posts_count, extras, source, observed_at
         FROM account_metric_samples
         WHERE agency_id = $1 AND platform = $2 AND account_id = $3
         ORDER BY observed_at DESC LIMIT 200",
    )
    .bind(auth.agency_id.as_uuid())
    .bind(&platform)
    .bind(&account_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let samples: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "employee_id": r.get::<Option<uuid::Uuid>, _>("employee_id"),
                "followers": r.get::<Option<i64>, _>("followers"),
                "following": r.get::<Option<i64>, _>("following"),
                "posts_count": r.get::<Option<i64>, _>("posts_count"),
                "extras": r.get::<Value, _>("extras"),
                "source": r.get::<String, _>("source"),
                "observed_at": r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at"),
            })
        })
        .collect();
    // 窗口增量现算: 最新 vs window_days 前最近一个样本
    let latest = rows.first();
    let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days);
    let baseline = rows
        .iter()
        .filter(|r| r.get::<chrono::DateTime<chrono::Utc>, _>("observed_at") <= cutoff)
        .last();
    let delta = match (latest, baseline) {
        (Some(l), Some(b)) => {
            let now = l.get::<Option<i64>, _>("followers");
            let base = b.get::<Option<i64>, _>("followers");
            match (now, base) {
                (Some(n), Some(b)) => json!({
                    "followers_now": n,
                    "followers_base": b,
                    "delta": n - b,
                    "delta_pct": if b != 0 { json!(((n - b) as f64 / b as f64 * 100.0).round() / 100.0) } else { Value::Null },
                    "window_days": window_days,
                }),
                _ => json!(null),
            }
        }
        _ => json!(null),
    };
    Ok(Json(
        json!({ "platform": platform, "account_id": account_id, "samples": samples, "delta": delta }),
    ))
}

/// 数字归一化: 1.2万 / 12.3K / 1,234 / 1.2M → 整数 (原始串由调用方留存 extras.raw)
pub fn normalize_count(s: &str) -> Option<i64> {
    let t = s.trim().replace(',', "");
    let (num, mult) = if let Some(x) = t.strip_suffix('万') {
        (x.parse::<f64>().ok()?, 10_000)
    } else if let Some(x) = t.strip_suffix('K').or_else(|| t.strip_suffix('k')) {
        (x.parse::<f64>().ok()?, 1_000)
    } else if let Some(x) = t.strip_suffix('M').or_else(|| t.strip_suffix('m')) {
        (x.parse::<f64>().ok()?, 1_000_000)
    } else {
        (t.parse::<f64>().ok()?, 1)
    };
    Some((num * mult as f64).round() as i64)
}

/// 后台循环: 每 N 小时 (SINCY_ACCOUNT_METRIC_INTERVAL_SECS, 默认 6h)
///   1) harvest 设备回执里的 read_followers captured 值 → account_metric_samples
///   2) 对每个 RUNNING 员工的绑定账号派发 read_followers 设备任务 (幂等键按小时分桶)
pub async fn account_metrics_loop(pool: sqlx::PgPool) {
    let interval_secs: u64 = std::env::var("SINCY_ACCOUNT_METRIC_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6 * 3600);
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(60)));
    loop {
        tick.tick().await;
        harvest_device_samples(&pool).await;
        dispatch_read_followers(&pool).await;
    }
}

/// harvest: task_events 里 read_followers 任务的 captured 值写入样本 (按 extras.task_id 去重)
async fn harvest_device_samples(pool: &sqlx::PgPool) {
    let rows = match sqlx::query(
        "SELECT e.task_id, e.detail, dt.agency_id, dt.context
         FROM task_events e
         JOIN device_tasks dt ON dt.id = e.task_id
         WHERE e.event = 'task.result'
           AND dt.context->>'action' = 'read_followers'
           AND e.detail->'captured' IS NOT NULL
           AND NOT EXISTS (
             SELECT 1 FROM account_metric_samples s
             WHERE s.extras->>'task_id' = e.task_id::text)",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "harvest read_followers samples failed");
            return;
        }
    };
    for r in rows {
        let task_id: uuid::Uuid = r.get("task_id");
        let detail: Value = r.get("detail");
        let agency_id: uuid::Uuid = r.get("agency_id");
        let context: Value = r.get("context");
        let captured = &detail["captured"];
        let platform = context["platform"].as_str().unwrap_or("").to_string();
        let account_id = context["account_id"].as_str().unwrap_or("").to_string();
        if platform.is_empty() || account_id.is_empty() {
            continue;
        }
        let followers = captured["followers"]
            .as_str()
            .and_then(normalize_count)
            .or_else(|| captured["followers"].as_i64());
        let following = captured["following"]
            .as_str()
            .and_then(normalize_count)
            .or_else(|| captured["following"].as_i64());
        let raw = json!({"followers": captured["followers"], "following": captured["following"]});
        let employee_id: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT employee_id FROM employee_accounts WHERE platform = $1 AND account_id = $2",
        )
        .bind(&platform)
        .bind(&account_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let bid: Option<uuid::Uuid> = match employee_id {
            Some(eid) => sqlx::query_scalar("SELECT brand_id FROM digital_employees WHERE id = $1")
                .bind(eid)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .flatten(),
            None => None,
        };
        let _ = sqlx::query(
            "INSERT INTO account_metric_samples
               (agency_id, brand_id, employee_id, platform, account_id, followers, following, extras, source)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'device')",
        )
        .bind(agency_id)
        .bind(bid)
        .bind(employee_id)
        .bind(&platform)
        .bind(&account_id)
        .bind(followers)
        .bind(following)
        .bind(json!({"task_id": task_id.to_string(), "raw": raw}))
        .execute(pool)
        .await;
        tracing::info!(%task_id, platform, account_id, "read_followers sample harvested");
    }
}

/// 派发: 每个 RUNNING 员工的绑定账号 → read_followers 设备任务 (小时桶幂等)
async fn dispatch_read_followers(pool: &sqlx::PgPool) {
    let accounts = match sqlx::query(
        "SELECT ea.platform, ea.account_id, ea.employee_id, ea.agency_id
         FROM employee_accounts ea
         JOIN digital_employees e ON e.id = ea.employee_id
         WHERE e.status = 'RUNNING'",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "list employee accounts failed");
            return;
        }
    };
    let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
    for a in accounts {
        let platform: String = a.get("platform");
        let account_id: String = a.get("account_id");
        let employee_id: uuid::Uuid = a.get("employee_id");
        let agency_id: uuid::Uuid = a.get("agency_id");
        let adapter_id = format!("{platform}.read_followers");
        // 只有该平台适配器已注册 active 版本才派发 (Kotlin 侧 read/capture 原语就绪后生效)
        let av: Option<i32> = sqlx::query_scalar(
            "SELECT v.version FROM app_adapter_versions v
             WHERE v.adapter_id = $1 AND v.state = 'active' ORDER BY v.version DESC LIMIT 1",
        )
        .bind(&adapter_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let Some(version) = av else {
            continue; // 适配器未上线: 跳过 (曲线靠 API 灌入或后续设备回流)
        };
        let task_id = uuid::Uuid::new_v4();
        let route_id = uuid::Uuid::new_v4();
        let idem = format!("metrics-{employee_id}-{platform}-{hour_bucket}");
        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let r = sqlx::query(
            "INSERT INTO execution_routes (id, agency_id, task_id, platform, action, selected_route, capability_status, priority)
             VALUES ($1, $2, $3, $4, 'read_followers', 'device', 'verified', 50)
             ON CONFLICT DO NOTHING",
        )
        .bind(route_id)
        .bind(agency_id)
        .bind(task_id)
        .bind(&platform)
        .execute(&mut *tx)
        .await;
        if r.is_err() {
            continue;
        }
        let r = sqlx::query(
            "INSERT INTO device_tasks (id, agency_id, idempotency_key, state, route_id, adapter_version, context)
             VALUES ($1, $2, $3, 'QUEUED', $4, $5, $6)
             ON CONFLICT (idempotency_key) DO NOTHING",
        )
        .bind(task_id)
        .bind(agency_id)
        .bind(&idem)
        .bind(route_id)
        .bind(version)
        .bind(json!({"platform": platform, "account_id": account_id, "action": "read_followers", "adapter": adapter_id, "employee_id": employee_id.to_string()}))
        .execute(&mut *tx)
        .await;
        match r {
            Ok(res) if res.rows_affected() == 1 => {
                let _ = tx.commit().await;
                tracing::info!(%task_id, platform, account_id, "read_followers task queued");
            }
            _ => {
                let _ = tx.rollback().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_counts() {
        assert_eq!(normalize_count("1,234"), Some(1234));
        assert_eq!(normalize_count("12.3K"), Some(12_300));
        assert_eq!(normalize_count("1.2万"), Some(12_000));
        assert_eq!(normalize_count("3.4M"), Some(3_400_000));
        assert_eq!(normalize_count("567"), Some(567));
        assert_eq!(normalize_count("abc"), None);
    }
}
