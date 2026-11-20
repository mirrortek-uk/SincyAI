//! Agent WebSocket gateway (WP-08): wss://<host>/api/v1/agent/connect.
//!
//! - device authentication via session token (WP-07); revoked devices are rejected
//! - Envelope schema_version is checked explicitly; mismatch => protocol error + close
//! - handles device.heartbeat / capabilities / app_versions / task.ack|progress|result
//!   / step.evidence / ui_drift.detected / challenge.detected
//! - dispatches QUEUED device tasks as device.task.assign
//! - challenge (captcha/security) always lands the task in WAITING_USER — never solved
//!
//! Dev slice transports Envelope as JSON over WS; protobuf binary framing is
//! layered on the same Envelope model (sincy-protocol::proto) in WP-02 follow-up.

use axum::{
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use sincy_common::{AgencyId, AppError};
use sincy_device::DeviceManager;
use sincy_execution::state::{DeviceTaskState, StateMachine};
use sincy_protocol::{Envelope, MessageType, check_schema};
use sqlx::Row;
use uuid::Uuid;

use crate::api::app_error_response;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ConnectQuery {
    token: String,
}

pub async fn agent_connect(
    State(state): State<AppState>,
    Query(q): Query<ConnectQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let jwt_secret = state.jwt_secret();
    let mgr = DeviceManager::new(pool, jwt_secret);
    // authenticate BEFORE upgrading: forged/revoked devices never connect
    let (device_id, agency_id) = mgr.authenticate(&q.token).await.map_err(|e| {
        app_error_response(AppError::non_retryable("device_auth_failed", e.to_string()))
    })?;
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, device_id, agency_id)))
}

async fn handle_socket(socket: WebSocket, state: AppState, device_id: Uuid, agency_id: AgencyId) {
    let pool = match state.pool() {
        Ok(p) => p.clone(),
        Err(e) => {
            tracing::error!(error = %e, "gateway without db");
            return;
        }
    };
    let jwt_secret = state.jwt_secret().to_vec();
    let mgr = DeviceManager::new(&pool, &jwt_secret);
    let (mut sender, mut receiver) = socket.split();

    tracing::info!(%device_id, %agency_id, "agent connected");

    // dispatch loop: poll QUEUED device tasks for this tenant and assign them
    let dispatch_pool = pool.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(64);
    let dispatch_device = device_id;
    let dispatch_agency = agency_id;
    let tx_dispatch = tx.clone();
    let dispatch = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            ticker.tick().await;
            let rows = sqlx::query(
                "SELECT id, idempotency_key, package_id, package_version, route_id
                 FROM device_tasks WHERE agency_id = $1 AND state = 'QUEUED' ORDER BY created_at LIMIT 5",
            )
            .bind(dispatch_agency.as_uuid())
            .fetch_all(&dispatch_pool)
            .await;
            let Ok(rows) = rows else { continue };
            for r in rows {
                let task_id: Uuid = r.get("id");
                let env = Envelope::new(
                    MessageType::DeviceTaskAssign,
                    dispatch_agency.to_string(),
                    dispatch_device.to_string(),
                    json!({
                        "task_id": task_id,
                        "idempotency_key": r.get::<String, _>("idempotency_key"),
                        "package_id": r.get::<Option<Uuid>, _>("package_id"),
                        "package_version": r.get::<Option<i32>, _>("package_version"),
                        "route_id": r.get::<Option<Uuid>, _>("route_id"),
                    }),
                );
                let _ = sqlx::query("UPDATE device_tasks SET state = 'EXECUTING', attempt = attempt + 1 WHERE id = $1")
                    .bind(task_id)
                    .execute(&dispatch_pool)
                    .await;
                if let Ok(text) = env.to_json()
                    && tx_dispatch.send(Message::Text(text.into())).await.is_err()
                {
                    return;
                }
            }
        }
    });

    // forwarder: mpsc -> websocket
    let fwd = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                let env = match Envelope::from_json(&text) {
                    Ok(e) => e,
                    Err(e) => {
                        let err = Envelope::new(
                            MessageType::DeviceHeartbeat,
                            agency_id.to_string(),
                            device_id.to_string(),
                            json!({"error": format!("invalid envelope: {e}")}),
                        );
                        let _ = tx2_send(&tx, err).await;
                        continue;
                    }
                };
                if let Err(e) = check_schema(&env) {
                    let err = Envelope::new(
                        MessageType::DeviceHeartbeat,
                        agency_id.to_string(),
                        device_id.to_string(),
                        json!({"protocol_error": e.to_string()}),
                    );
                    let _ = tx2_send(&tx, err).await;
                    break; // protocol mismatch: close
                }
                if let Err(e) = handle_envelope(&pool, &mgr, &env, device_id, agency_id).await {
                    tracing::warn!(error = %e, message_type = env.message_type.as_str(), "envelope handling failed");
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    dispatch.abort();
    fwd.abort();
    let _ = mgr.heartbeat(device_id).await;
    tracing::info!(%device_id, "agent disconnected");
}

async fn tx2_send(tx: &tokio::sync::mpsc::Sender<Message>, env: Envelope) {
    if let Ok(text) = env.to_json() {
        let _ = tx.send(Message::Text(text.into())).await;
    }
}

async fn handle_envelope(
    pool: &sqlx::PgPool,
    mgr: &DeviceManager<'_>,
    env: &Envelope,
    device_id: Uuid,
    agency_id: AgencyId,
) -> Result<(), AppError> {
    let db = |e: sqlx::Error| AppError::retryable("db", e.to_string());
    match env.message_type {
        MessageType::DeviceHeartbeat => {
            mgr.heartbeat(device_id)
                .await
                .map_err(|e| AppError::retryable("db", e.to_string()))?;
        }
        MessageType::DeviceCapabilitiesUpdate => {
            let caps = env
                .payload
                .get("capabilities")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let rows: Vec<(String, bool, String)> = caps
                .iter()
                .filter_map(|c| {
                    Some((
                        c["name"].as_str()?.to_string(),
                        c["available"].as_bool().unwrap_or(false),
                        c["detail"].as_str().unwrap_or("").to_string(),
                    ))
                })
                .collect();
            mgr.update_capabilities(agency_id, device_id, &rows)
                .await
                .map_err(|e| AppError::retryable("db", e.to_string()))?;
        }
        MessageType::DeviceAppVersionsUpdate => {
            let apps = env
                .payload
                .get("apps")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let rows: Vec<(String, String, i32)> = apps
                .iter()
                .filter_map(|a| {
                    Some((
                        a["package"].as_str()?.to_string(),
                        a["version_name"].as_str()?.to_string(),
                        a["version_code"].as_i64()? as i32,
                    ))
                })
                .collect();
            mgr.update_app_versions(agency_id, device_id, &rows)
                .await
                .map_err(|e| AppError::retryable("db", e.to_string()))?;
        }
        MessageType::DeviceTaskAck | MessageType::DeviceTaskProgress => {
            let task_id = env.payload["task_id"].as_str().unwrap_or_default();
            let state = env
                .payload
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            append_event(
                pool,
                agency_id,
                task_id,
                &env.message_type,
                state,
                &env.payload,
            )
            .await
            .map_err(db)?;
        }
        MessageType::DeviceTaskResult => {
            let task_id = Uuid::parse_str(env.payload["task_id"].as_str().unwrap_or_default())
                .map_err(|e| AppError::non_retryable("bad_task_id", e.to_string()))?;
            let outcome = env.payload["outcome"].as_str().unwrap_or("");
            let new_state = match outcome {
                "success" => DeviceTaskState::Completed,
                "no_side_effect_failure" => DeviceTaskState::Failed,
                "uncertain" => DeviceTaskState::VerificationPending,
                _ => DeviceTaskState::Failed,
            };
            let mut sm = StateMachine {
                state: DeviceTaskState::ExecutingSteps,
                side_effect_done: true,
            };
            let ev = sm
                .transition("task.result", new_state)
                .map_err(|e| AppError::non_retryable("illegal_transition", e.to_string()))?;
            let mut tx = pool.begin().await.map_err(db)?;
            sqlx::query(&format!(
                "SET LOCAL app.current_agency_id = '{}'",
                agency_id.as_uuid()
            ))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            sqlx::query("UPDATE device_tasks SET state = $1, updated_at = now() WHERE id = $2 AND agency_id = $3")
                .bind(format!("{:?}", new_state).to_uppercase())
                .bind(task_id)
                .bind(agency_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            sqlx::query(
                "INSERT INTO task_events (agency_id, task_id, event, from_state, to_state, detail)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(agency_id.as_uuid())
            .bind(task_id)
            .bind(&ev.event)
            .bind(&ev.from)
            .bind(&ev.to)
            .bind(&env.payload)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)?;
        }
        MessageType::DeviceStepEvidence => {
            let task_id = env.payload["task_id"].as_str().unwrap_or_default();
            append_event(
                pool,
                agency_id,
                task_id,
                &env.message_type,
                "EVIDENCE",
                &env.payload,
            )
            .await
            .map_err(db)?;
        }
        MessageType::DeviceChallengeDetected => {
            // captcha / security challenge => WAITING_USER, never auto-solved
            let task_id = Uuid::parse_str(env.payload["task_id"].as_str().unwrap_or_default())
                .map_err(|e| AppError::non_retryable("bad_task_id", e.to_string()))?;
            let mut tx = pool.begin().await.map_err(db)?;
            sqlx::query(&format!(
                "SET LOCAL app.current_agency_id = '{}'",
                agency_id.as_uuid()
            ))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            sqlx::query("UPDATE device_tasks SET state = 'WAITING_USER', updated_at = now() WHERE id = $1 AND agency_id = $2")
                .bind(task_id).bind(agency_id.as_uuid())
                .execute(&mut *tx).await.map_err(db)?;
            sqlx::query(
                "INSERT INTO task_events (agency_id, task_id, event, from_state, to_state, detail)
                 VALUES ($1, $2, 'challenge.detected', 'EXECUTING_STEPS', 'WAITING_USER', $3)",
            )
            .bind(agency_id.as_uuid())
            .bind(task_id)
            .bind(&env.payload)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)?;
        }
        MessageType::DeviceUiDriftDetected => {
            // mark the adapter degraded for that platform/action (WP-13)
            let platform = env.payload["platform"].as_str().unwrap_or("");
            let action = env.payload["action"].as_str().unwrap_or("publish_video");
            let evidence = env.payload["reason"].as_str().unwrap_or("ui_drift");
            sqlx::query(
                "UPDATE channel_capabilities SET status = 'degraded', updated_at = now()
                 WHERE platform = $1 AND action = $2 AND route = 'device'",
            )
            .bind(platform)
            .bind(action)
            .execute(pool)
            .await
            .map_err(db)?;
            tracing::warn!(%device_id, platform, action, evidence, "UI_DRIFT_DETECTED: adapter marked degraded");
        }
        _ => {
            return Err(AppError::non_retryable(
                "unexpected_message",
                format!("server does not handle {}", env.message_type.as_str()),
            ));
        }
    }
    Ok(())
}

async fn append_event(
    pool: &sqlx::PgPool,
    agency_id: AgencyId,
    task_id: &str,
    message_type: &MessageType,
    state: &str,
    payload: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let Ok(task_uuid) = Uuid::parse_str(task_id) else {
        return Ok(()); // non-task messages are not journaled
    };
    sqlx::query(
        "INSERT INTO task_events (agency_id, task_id, event, from_state, to_state, detail)
         VALUES ($1, $2, $3, NULL, $4, $5)",
    )
    .bind(agency_id.as_uuid())
    .bind(task_uuid)
    .bind(message_type.as_str())
    .bind(state)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}
