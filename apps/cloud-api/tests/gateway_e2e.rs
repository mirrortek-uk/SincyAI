//! WP-08 acceptance with a simulated Agent over a real WebSocket:
//! register -> connect -> heartbeat -> receive device.task.assign ->
//! report result -> task COMPLETED in DB; challenge.detected -> WAITING_USER;
//! revoked device cannot connect. Skips without SINCY_TEST_DATABASE_URL.

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use sha2::{Digest, Sha256};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sincy_protocol::{Envelope, MessageType};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

fn proof_for(secret: &str, nonce: &str) -> String {
    let secret_hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
    format!(
        "{:x}",
        Sha256::digest(format!("{secret_hash}{nonce}").as_bytes())
    )
}

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "gw-test-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, device_app_versions, device_capabilities, device_sessions, devices, automation_consents, social_accounts, account_device_bindings, quota_ledger, quota_pools, platform_apps, channel_capabilities, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'gw')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");

    // start the real server on an ephemeral port
    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    let app = axum::Router::new()
        .nest("/api/v1", cloud_api::routes())
        .with_state(state)
        .fallback(cloud_api::api::not_found);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    Some((pool, agency, port))
}

async fn register_device(pool: &sqlx::PgPool, agency: AgencyId) -> (Uuid, String) {
    let mgr = DeviceManager::new(pool, b"gw-test-secret");
    let ch = mgr
        .start_registration(agency, "android", Some("sim"))
        .await
        .expect("start");
    let session = mgr
        .complete_registration(
            ch.device_id,
            &ch.nonce,
            &proof_for(&ch.device_secret, &ch.nonce),
        )
        .await
        .expect("complete");
    sqlx::query("UPDATE devices SET last_seen = now() WHERE id = $1")
        .bind(ch.device_id)
        .execute(pool)
        .await
        .unwrap();
    (ch.device_id, session.token)
}

#[tokio::test]
async fn simulated_agent_full_cycle() {
    let Some((pool, agency, port)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, token) = register_device(&pool, agency).await;

    // insert a QUEUED device task to be dispatched
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO device_tasks (id, agency_id, idempotency_key, state) VALUES ($1, $2, $3, 'QUEUED')",
    )
    .bind(task_id).bind(agency.as_uuid()).bind(format!("gw-{task_id}"))
    .execute(&pool).await.expect("task");

    // connect over WebSocket
    let url = format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={token}");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("ws connect");

    // heartbeat
    let hb = Envelope::new(
        MessageType::DeviceHeartbeat,
        agency.to_string(),
        device_id.to_string(),
        json!({"status": "online"}),
    );
    ws.send(WsMessage::Text(hb.to_json().unwrap().into()))
        .await
        .expect("send hb");

    // wait for device.task.assign (dispatch loop ticks every 2s)
    let mut assigned: Option<Uuid> = None;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next()).await;
        let Some(Some(Ok(WsMessage::Text(text)))) = msg.ok() else {
            break;
        };
        let env = Envelope::from_json(&text).expect("envelope");
        if env.message_type == MessageType::DeviceTaskAssign {
            assigned = Some(Uuid::parse_str(env.payload["task_id"].as_str().unwrap()).unwrap());
            break;
        }
    }
    let assigned = assigned.expect("should receive task assign");
    assert_eq!(assigned, task_id);

    // report success result
    let result = Envelope::new(
        MessageType::DeviceTaskResult,
        agency.to_string(),
        device_id.to_string(),
        json!({"task_id": task_id, "outcome": "success", "external_content_id": "dev-post-1"}),
    );
    ws.send(WsMessage::Text(result.to_json().unwrap().into()))
        .await
        .expect("send result");

    // wait a moment for processing, then verify DB
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .expect("task row");
    assert_eq!(row.get::<String, _>("state"), "COMPLETED");
    let events: i64 = sqlx::query(
        "SELECT count(*) FROM task_events WHERE task_id = $1 AND event = 'task.result'",
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(events, 1);

    // challenge detected -> WAITING_USER
    let task2 = Uuid::new_v4();
    sqlx::query("INSERT INTO device_tasks (id, agency_id, idempotency_key, state) VALUES ($1, $2, $3, 'EXECUTING_STEPS')")
        .bind(task2).bind(agency.as_uuid()).bind(format!("gw-{task2}"))
        .execute(&pool).await.unwrap();
    let challenge = Envelope::new(
        MessageType::DeviceChallengeDetected,
        agency.to_string(),
        device_id.to_string(),
        json!({"task_id": task2, "screen_id": "challenge", "challenge_kind": "captcha"}),
    );
    ws.send(WsMessage::Text(challenge.to_json().unwrap().into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(task2)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row.get::<String, _>("state"),
        "WAITING_USER",
        "challenge must park the task for a human"
    );

    drop(ws);

    // revoked device cannot connect
    let mgr = DeviceManager::new(&pool, b"gw-test-secret");
    mgr.revoke(agency, device_id).await.expect("revoke");
    let url2 = format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={token}");
    let res = tokio_tungstenite::connect_async(&url2).await;
    assert!(res.is_err(), "revoked device must be rejected at connect");
}
