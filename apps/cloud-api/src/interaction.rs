//! 互动子循环 (粉丝运营): read_mentions 抓提及 → LLM 生成回复 → 派发 reply_comment 设备任务。
//!
//! 数据流: interaction_loop 每 N 小时 (SINCY_INTERACTION_INTERVAL_SECS, 默认 2h)
//!   1) harvest 设备回执里 read_mentions 任务的 captured.mentions → social_mentions
//!      (captured 格式: 每行 "handle|post_id|text", 由 x.read_mentions 适配器 read 动作抓整块文本)
//!   2) 对每个 RUNNING 员工的绑定账号派发 read_mentions 设备任务 (小时桶幂等, 仅当适配器 active)
//!   3) reply pass: 未回复的 mention (每 tick 每账号上限 5 条) 用 LLM 生成回复,
//!      走 x.reply_comment 已签名适配器派发 reply_comment 设备任务 (package_text 注入, 不参与签名),
//!      成功入队后标记 social_mentions.replied + task_id。
//!
//! challenge 一律 WAITING_USER (适配器内置), 绝不自动过。表不走 FORCE RLS, 显式 agency_id scope。

use serde_json::{Value, json};
use sqlx::Row;

use crate::employees::PgLlmPort;
use sincy_employee::cycle::LlmPort;

/// 后台循环入口 (main.rs spawn)
pub async fn interaction_loop(pool: sqlx::PgPool, allow_mock_llm: bool) {
    let interval_secs: u64 = std::env::var("SINCY_INTERACTION_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2 * 3600);
    tracing::info!(interval_secs, "interaction_loop started");
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(60)));
    loop {
        tick.tick().await;
        harvest_mentions(&pool).await;
        dispatch_read_mentions(&pool).await;
        reply_pass(&pool, allow_mock_llm).await;
    }
}

/// harvest: read_mentions 任务回执里的 captured.mentions 写入 social_mentions
/// (按 platform+account_id+post_id 去重; 无 post_id 时按 handle+前40字 去重)
async fn harvest_mentions(pool: &sqlx::PgPool) {
    let rows = match sqlx::query(
        "SELECT e.task_id, e.detail, dt.agency_id, dt.context
         FROM task_events e
         JOIN device_tasks dt ON dt.id = e.task_id
         WHERE e.event = 'task.result'
           AND dt.context->>'action' = 'read_mentions'
           AND e.detail->'captured'->>'mentions' IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "harvest mentions failed");
            return;
        }
    };
    for r in rows {
        let task_id: uuid::Uuid = r.get("task_id");
        let detail: Value = r.get("detail");
        let agency_id: uuid::Uuid = r.get("agency_id");
        let context: Value = r.get("context");
        let platform = context["platform"].as_str().unwrap_or("").to_string();
        let account_id = context["account_id"].as_str().unwrap_or("").to_string();
        let employee_id: Option<uuid::Uuid> = context["employee_id"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        if platform.is_empty() || account_id.is_empty() {
            continue;
        }
        let raw = detail["captured"]["mentions"].as_str().unwrap_or("");
        for line in raw.lines() {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if parts.len() < 2 || parts[0].trim().is_empty() {
                continue;
            }
            let handle = parts[0].trim().trim_start_matches('@').to_string();
            let post_id = parts
                .get(1)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let text = parts
                .get(2)
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let r = sqlx::query(
                "INSERT INTO social_mentions (agency_id, employee_id, platform, account_id, author_handle, post_id, text, kind)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'mention')
                 ON CONFLICT DO NOTHING",
            )
            .bind(agency_id)
            .bind(employee_id)
            .bind(&platform)
            .bind(&account_id)
            .bind(&handle)
            .bind(post_id.as_deref())
            .bind(&text)
            .execute(pool)
            .await;
            match r {
                Ok(res) if res.rows_affected() == 1 => {
                    tracing::info!(%task_id, platform, handle, "mention harvested");
                }
                Ok(_) => { /* dedup 命中: 同一 post_id/text 已存在 */ }
                Err(e) => tracing::warn!(error = %e, "insert mention failed"),
            }
        }
    }
}

/// 派发 read_mentions 设备任务 (小时桶幂等, 仅当 <platform>.read_mentions 适配器 active)
async fn dispatch_read_mentions(pool: &sqlx::PgPool) {
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
        Err(_) => return,
    };
    let hour_bucket = chrono::Utc::now().format("%Y%m%d%H").to_string();
    for a in accounts {
        let platform: String = a.get("platform");
        let account_id: String = a.get("account_id");
        let employee_id: uuid::Uuid = a.get("employee_id");
        let agency_id: uuid::Uuid = a.get("agency_id");
        let adapter_id = format!("{platform}.read_mentions");
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
            continue;
        };
        let task_id = uuid::Uuid::new_v4();
        let route_id = uuid::Uuid::new_v4();
        let idem = format!("mentions-{employee_id}-{platform}-{hour_bucket}");
        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let r = sqlx::query(
            "INSERT INTO execution_routes (id, agency_id, task_id, platform, action, selected_route, capability_status, priority)
             VALUES ($1, $2, $3, $4, 'read_mentions', 'device', 'verified', 50)
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
             ON CONFLICT (agency_id, idempotency_key) DO NOTHING",
        )
        .bind(task_id)
        .bind(agency_id)
        .bind(&idem)
        .bind(route_id)
        .bind(version)
        .bind(json!({"platform": platform, "account_id": account_id, "action": "read_mentions", "adapter": adapter_id, "employee_id": employee_id.to_string()}))
        .execute(&mut *tx)
        .await;
        match r {
            Ok(res) if res.rows_affected() == 1 => {
                let _ = tx.commit().await;
                tracing::info!(%task_id, platform, account_id, "read_mentions task queued");
            }
            _ => {
                let _ = tx.rollback().await;
            }
        }
    }
}

/// reply pass: 未回复 mention → LLM 生成回复 → reply_comment 设备任务 (幂等键 reply-<mention_id>)
async fn reply_pass(pool: &sqlx::PgPool, allow_mock_llm: bool) {
    let pending = match sqlx::query(
        "SELECT m.id, m.agency_id, m.employee_id, m.platform, m.account_id, m.author_handle, m.text
         FROM social_mentions m
         WHERE m.replied = false AND m.kind = 'mention'
         ORDER BY m.observed_at ASC LIMIT 25",
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    let llm = PgLlmPort::new(allow_mock_llm);
    for m in pending {
        let mention_id: uuid::Uuid = m.get("id");
        let agency_id: uuid::Uuid = m.get("agency_id");
        let platform: String = m.get("platform");
        let account_id: String = m.get("account_id");
        let handle: String = m.get("author_handle");
        let text: String = m.get("text");
        // 每账号每 tick 上限 5 条
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM social_mentions
             WHERE platform = $1 AND account_id = $2 AND replied = true
               AND task_id IN (SELECT id FROM device_tasks WHERE context->>'idem_tick' = to_char(now(), 'YYYYMMDDHH24'))",
        )
        .bind(&platform)
        .bind(&account_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if n >= 5 {
            continue;
        }
        // reply_comment 适配器版本
        let adapter_id = format!("{platform}.reply_comment");
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
            continue;
        };
        // LLM 生成回复 (品牌语气在 prompt 里兜底; 失败则跳过该条)
        let prompt = format!(
            "You are the social media manager of a tech brand. Reply to this mention from @{handle} \
             in ONE short, friendly sentence (same language as the mention, max 120 chars). \
             Mention: {text}\nReply only, no quotes, no explanation."
        );
        let reply = match llm.generate(&prompt, 128).await {
            Ok(t) => t.trim().trim_matches('"').to_string(),
            Err(e) => {
                tracing::warn!(mention = %mention_id, error = %e, "llm reply failed, skip");
                continue;
            }
        };
        if reply.is_empty() {
            continue;
        }
        let task_id = uuid::Uuid::new_v4();
        let route_id = uuid::Uuid::new_v4();
        let idem = format!("reply-{mention_id}");
        let tick = chrono::Utc::now().format("%Y%m%d%H").to_string();
        let mut tx = match pool.begin().await {
            Ok(t) => t,
            Err(_) => continue,
        };
        let r = sqlx::query(
            "INSERT INTO execution_routes (id, agency_id, task_id, platform, action, selected_route, capability_status, priority)
             VALUES ($1, $2, $3, $4, 'reply_comment', 'device', 'verified', 50)
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
             ON CONFLICT (agency_id, idempotency_key) DO NOTHING",
        )
        .bind(task_id)
        .bind(agency_id)
        .bind(&idem)
        .bind(route_id)
        .bind(version)
        .bind(json!({
            "platform": platform,
            "account_id": account_id,
            "action": "reply_comment",
            "adapter": adapter_id,
            "package_text": reply,
            "mention_id": mention_id.to_string(),
            "idem_tick": tick,
        }))
        .execute(&mut *tx)
        .await;
        match r {
            Ok(res) if res.rows_affected() == 1 => {
                let _ = sqlx::query(
                    "UPDATE social_mentions SET replied = true, task_id = $2 WHERE id = $1",
                )
                .bind(mention_id)
                .bind(task_id)
                .execute(&mut *tx)
                .await;
                let _ = tx.commit().await;
                tracing::info!(%task_id, %mention_id, platform, "reply_comment task queued");
            }
            _ => {
                let _ = tx.rollback().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mention_line_parse() {
        // 格式: handle|post_id|text
        let line = "@alice|8899123|nice post!";
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        assert_eq!(parts[0], "@alice");
        assert_eq!(parts[1], "8899123");
        assert_eq!(parts[2], "nice post!");
    }
}
