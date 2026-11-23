//! Device-route end-to-end with a simulated phone agent running the REAL agent
//! core (sincy-agent) over a real WebSocket to a real server:
//!
//!   publish X post -> route=device (DEVICE_QUEUED)
//!     -> gateway dispatches device.task.assign over WSS
//!     -> agent fetches signed task brief via GET /agent/tasks/{id}
//!     -> agent verifies adapter signature + version range, runs the interpreter
//!        on a scripted screen sequence
//!     -> device.task.result success -> device_tasks COMPLETED + task_events audit
//!
//! Also covers: forged device token cannot fetch task detail (401-class error).
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sha2::{Digest, Sha256};
use sincy_adapter::interpreter::MockSnapshot;
use sincy_adapter::schema::AdapterDefinition;
use sincy_adapter::sign::sign_definition;
use sincy_agent::{AgentClient, ScriptedIo, TaskOutcome};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";
const DEV_SIGNING_KEY: &[u8] = b"sincy-dev-adapter-signing-key";
const X_ADAPTER_YAML: &str = include_str!("../../../adapters/x.publish_post.yaml");

fn proof_for(secret: &str, nonce: &str) -> String {
    let secret_hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
    format!(
        "{:x}",
        Sha256::digest(format!("{secret_hash}{nonce}").as_bytes())
    )
}

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "agent-e2e-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, device_app_versions, device_capabilities, device_sessions, devices, automation_consents, social_accounts, account_device_bindings, quota_ledger, quota_pools, platform_apps, channel_capabilities, app_adapter_versions, app_adapters, publishing_packages, package_assets, ai_labels, content_rights, assets, content_versions, content_items, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'agent-e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("AgentE2e2026").expect("hash");
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'e2e@sincy.dev', $3, 'owner')")
        .bind(member_id).bind(agency.as_uuid()).bind(hash)
        .execute(&pool).await.expect("member");

    // capability matrix: x publish_post device-first (migration 0014 semantics)
    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES
         ('x', 'publish_post', 'device', 'verified', 10, '{\"app_package\": \"com.twitter.android\", \"adapter\": \"x.publish_post\"}'),
         ('x', 'publish_post', 'api', 'unverified', 50, '{\"paid_tier_required\": true}'),
         ('x', 'publish_post', 'assisted', 'verified', 90, '{}')",
    )
    .execute(&pool).await.expect("caps");

    // signed adapter registry entry (same as migration 0015)
    let def = AdapterDefinition::from_yaml(X_ADAPTER_YAML).expect("valid");
    let canonical = serde_json::to_string(&def).expect("canonical");
    let sig = sign_definition(DEV_SIGNING_KEY, &canonical);
    sqlx::query("INSERT INTO app_adapters (id, app_package, description) VALUES ('x.publish_post', 'com.twitter.android', 'x device adapter')")
        .execute(&pool).await.expect("adapter row");
    sqlx::query(
        "INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
         VALUES ('x.publish_post', 1, '>=10.0 <11.0', $1::jsonb, $2, 'x', 'active')",
    )
    .bind(&canonical).bind(&sig)
    .execute(&pool).await.expect("adapter version");

    // client + brand + package
    let client_id = Uuid::new_v4();
    sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, 'c')")
        .bind(client_id)
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("client");
    let brand_id = Uuid::new_v4();
    sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, 'b')")
        .bind(brand_id)
        .bind(agency.as_uuid())
        .bind(client_id)
        .execute(&pool)
        .await
        .expect("brand");
    let package_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO publishing_packages (id, version, agency_id, brand_id, platform, account_id, action, text_content, approval_status)
         VALUES ($1, 1, $2, $3, 'x', $4, 'publish_post', 'hello from device agent e2e', 'approved')",
    )
    .bind(package_id).bind(agency.as_uuid()).bind(brand_id).bind(client_id)
    .execute(&pool).await.expect("package");

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

    // owner JWT for the publish call
    let login = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/auth/login"))
        .json(&json!({"email": "e2e@sincy.dev", "password": "AgentE2e2026"}))
        .send()
        .await
        .expect("login")
        .json::<serde_json::Value>()
        .await
        .expect("login json");
    let user_token = login["token"].as_str().expect("token").to_string();

    Some((pool, agency, port, user_token))
}

async fn register_device(pool: &sqlx::PgPool, agency: AgencyId) -> (Uuid, String) {
    let mgr = DeviceManager::new(pool, b"agent-e2e-secret");
    let ch = mgr
        .start_registration(agency, "android", Some("sim-pixel"))
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
    // 设备路由按心跳判在线: 注册后立刻心跳, 模拟真实在线设备
    sqlx::query("UPDATE devices SET last_seen = now(), state = 'active' WHERE id = $1")
        .bind(ch.device_id)
        .execute(pool)
        .await
        .expect("heartbeat");
    (ch.device_id, session.token)
}

#[tokio::test]
async fn x_post_flows_through_device_agent_to_completed() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    // 1) publish an X post as the console user -> must route to device
    let client = reqwest::Client::new();
    let brand_id: Uuid = sqlx::query_scalar("SELECT id FROM brands WHERE agency_id = $1")
        .bind(agency.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let package_id: Uuid =
        sqlx::query_scalar("SELECT id FROM publishing_packages WHERE agency_id = $1")
            .bind(agency.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let pub_resp = client
        .post(format!("http://127.0.0.1:{port}/api/v1/tasks/publish"))
        .bearer_auth(&user_token)
        .json(&json!({
            "brand_id": brand_id,
            "platform": "x",
            "action": "publish_post",
            "package_id": package_id,
            "package_version": 1,
            "idempotency_key": format!("e2e-{}", Uuid::new_v4()),
        }))
        .send()
        .await
        .expect("publish")
        .json::<serde_json::Value>()
        .await
        .expect("publish json");
    assert_eq!(
        pub_resp["route"], "device",
        "X must route to device: {pub_resp}"
    );
    assert_eq!(pub_resp["status"], "DEVICE_QUEUED");
    let task_id: Uuid = pub_resp["task_id"].as_str().unwrap().parse().unwrap();

    // 2) run the simulated phone agent: real WSS + real sincy-agent core
    let agent = AgentClient {
        ws_url: format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={device_token}"),
        http_base: format!("http://127.0.0.1:{port}/api/v1"),
        token: device_token.clone(),
        device_id: device_id.to_string(),
        agency_id: agency.to_string(),
        signing_key: DEV_SIGNING_KEY.to_vec(),
    };
    let screens = vec![
        MockSnapshot::new(
            vec!["com.twitter.android:id/bottom_bar".into()],
            vec!["Home".into()],
        ),
        MockSnapshot::new(
            vec!["com.twitter.android:id/tweet_text".into()],
            vec!["What's happening?".into()],
        ),
        MockSnapshot::new(
            vec!["com.twitter.android:id/tweet_text".into()],
            vec!["What's happening?".into()],
        ),
        MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
        MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
    ];
    let agent_handle = tokio::spawn(async move {
        agent
            .run_next_task(
                move || Box::new(ScriptedIo::new(screens)),
                "10.5.0",
                std::time::Duration::from_secs(20),
            )
            .await
    });

    // 3) wait for the task to reach COMPLETED in the DB
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    let agent_handle = agent_handle;
    let final_state = loop {
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
                .bind(task_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
        if let Some(s) = state
            && (s == "COMPLETED"
                || s == "FAILED"
                || s == "WAITING_USER"
                || s == "VERIFICATION_PENDING")
        {
            break s;
        }
        if tokio::time::Instant::now() > deadline {
            let evs: Vec<String> = sqlx::query(
                "SELECT event || ':' || to_state FROM task_events WHERE task_id = $1 ORDER BY at",
            )
            .bind(task_id)
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| r.get::<String, _>(0))
            .collect();
            let st: Option<String> =
                sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
                    .bind(task_id)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            let agent_result = agent_handle.await;
            eprintln!(
                "TIMEOUT. task state: {st:?}, events: {evs:?}, agent result: {agent_result:?}"
            );
            panic!("timeout waiting for task completion");
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    };
    let ex = agent_handle
        .await
        .expect("agent join")
        .unwrap_or_else(|e| panic!("agent run failed: {e:?}"));
    eprintln!("agent outcome: {:?}", ex.outcome);
    assert_eq!(ex.outcome, TaskOutcome::Success);
    assert!(ex.side_effect_done);
    assert_eq!(
        final_state, "COMPLETED",
        "task must complete via device agent"
    );

    // audit trail: route decision + result event
    let route: String =
        sqlx::query_scalar("SELECT selected_route FROM execution_routes WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(route, "device");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_events WHERE task_id = $1 AND event = 'task.result'",
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(events >= 1, "task.result event must be recorded");

    // 4) a forged device token must NOT be able to fetch the task brief
    let forged = client
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/agent/tasks/{task_id}?token=forged"
        ))
        .send()
        .await
        .expect("forged req");
    assert!(
        !forged.status().is_success(),
        "forged device token must be rejected"
    );
}
