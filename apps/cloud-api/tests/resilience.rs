//! WP-15 fault-injection acceptance: device lost mid-task is requeued (never
//! after side effect), duplicate results are idempotent, and /metrics exposes
//! the observability surface. Skips without SINCY_TEST_DATABASE_URL.

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

/// Each test truncates the shared test DB, so they must not run concurrently.
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn proof_for(secret: &str, nonce: &str) -> String {
    let secret_hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
    format!(
        "{:x}",
        Sha256::digest(format!("{secret_hash}{nonce}").as_bytes())
    )
}

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "res-test-secret") };
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
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'res')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
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
    let mgr = DeviceManager::new(pool, b"res-test-secret");
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
    (ch.device_id, session.token)
}

async fn connect(
    port: u16,
    token: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={token}");
    let (ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("ws connect");
    ws
}

/// Wait until the next device.task.assign arrives; returns task_id.
async fn wait_assign(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    timeout_secs: u64,
) -> Uuid {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while tokio::time::Instant::now() < deadline {
        let msg =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), ws.next()).await;
        if let Ok(Some(Ok(WsMessage::Text(text)))) = msg {
            let env = Envelope::from_json(&text).expect("envelope");
            if env.message_type == MessageType::DeviceTaskAssign {
                return Uuid::parse_str(env.payload["task_id"].as_str().unwrap()).unwrap();
            }
        }
    }
    panic!("no device.task.assign within {timeout_secs}s");
}

async fn send_result(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    agency: AgencyId,
    device_id: Uuid,
    task_id: Uuid,
) {
    let result = Envelope::new(
        MessageType::DeviceTaskResult,
        agency.to_string(),
        device_id.to_string(),
        json!({"task_id": task_id, "outcome": "success", "external_content_id": "dev-post-x"}),
    );
    ws.send(WsMessage::Text(result.to_json().unwrap().into()))
        .await
        .expect("send result");
}

#[tokio::test]
async fn device_crash_mid_task_requeues_and_completes_once() {
    let _guard = TEST_LOCK.lock().await;
    let Some((pool, agency, port)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, token) = register_device(&pool, agency).await;

    let task_id = Uuid::new_v4();
    sqlx::query("INSERT INTO device_tasks (id, agency_id, idempotency_key, state) VALUES ($1, $2, $3, 'QUEUED')")
        .bind(task_id).bind(agency.as_uuid()).bind(format!("res-{task_id}"))
        .execute(&pool).await.expect("task");

    // agent connects, receives the task (EXECUTING), then "crashes" (socket drops)
    let mut ws = connect(port, &token).await;
    let assigned = wait_assign(&mut ws, 10).await;
    assert_eq!(assigned, task_id);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("state"), "EXECUTING");
    drop(ws); // crash: no result sent

    // wait for disconnect handling to requeue
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        if row.get::<String, _>("state") == "QUEUED" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "task was not requeued after device loss"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // agent reconnects and gets the SAME task again, then completes it
    let mut ws2 = connect(port, &token).await;
    let assigned2 = wait_assign(&mut ws2, 10).await;
    assert_eq!(assigned2, task_id, "requeued task must be re-dispatched");
    send_result(&mut ws2, agency, device_id, task_id).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("state"), "COMPLETED");

    // duplicate result delivery must be idempotent: still COMPLETED, one result event
    send_result(&mut ws2, agency, device_id, task_id).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let events: i64 = sqlx::query(
        "SELECT count(*) FROM task_events WHERE task_id = $1 AND event = 'task.result'",
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(events, 1, "duplicate task.result must not double-count");
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("state"), "COMPLETED");
}

#[tokio::test]
async fn device_crash_after_side_effect_never_auto_retries() {
    let _guard = TEST_LOCK.lock().await;
    let Some((pool, agency, port)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, token) = register_device(&pool, agency).await;

    // task already past the side-effect boundary, assigned to a device
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO device_tasks (id, agency_id, idempotency_key, state, side_effect_done, assigned_device_id)
         VALUES ($1, $2, $3, 'EXECUTING', true, $4)",
    )
    .bind(task_id).bind(agency.as_uuid()).bind(format!("res-{task_id}")).bind(device_id)
    .execute(&pool).await.expect("task");

    let mut ws = connect(port, &token).await;
    let hb = Envelope::new(
        MessageType::DeviceHeartbeat,
        agency.to_string(),
        device_id.to_string(),
        json!({"status": "online"}),
    );
    ws.send(WsMessage::Text(hb.to_json().unwrap().into()))
        .await
        .unwrap();
    drop(ws); // crash

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let st = row.get::<String, _>("state");
        if st != "EXECUTING" {
            assert_eq!(
                st, "VERIFICATION_PENDING",
                "post-side-effect loss must be Uncertain, never retried"
            );
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn metrics_endpoint_exposes_counters_and_gauges() {
    let _guard = TEST_LOCK.lock().await;
    let Some((pool, agency, _port)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    // one task so the state gauge has a series
    let task_id = Uuid::new_v4();
    sqlx::query("INSERT INTO device_tasks (id, agency_id, idempotency_key, state) VALUES ($1, $2, $3, 'QUEUED')")
        .bind(task_id).bind(agency.as_uuid()).bind(format!("res-{task_id}"))
        .execute(&pool).await.expect("task");
    use tower::ServiceExt;
    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    let app = axum::Router::new()
        .nest("/api/v1", cloud_api::routes())
        .with_state(state)
        .fallback(cloud_api::api::not_found);
    let req = axum::http::Request::builder()
        .uri("/api/v1/metrics")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("metrics response");
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        body.contains("sincy_http_requests_total"),
        "missing counters:\n{body}"
    );
    assert!(
        body.contains("sincy_device_tasks{state="),
        "missing DB gauges:\n{body}"
    );
    assert!(body.contains("sincy_devices_online"));
}
