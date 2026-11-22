//! system.setup_install 引导安装任务的端到端测试:
//!
//!   控制台 POST /setup/run {action:"setup_install"} -> device 任务 QUEUED
//!     -> WSS 派发 -> agent 拉签名简报 -> Rust core 验签
//!     -> 原生 op 序列: download_file(sha256 校验) -> install_apk -> wait_installed -> launch_app
//!     -> task COMPLETED, task_events 里留下 op 证据
//! 另测: sha256 不匹配 -> 只执行 download_file 一个 op -> FAILED (零副作用)。
//!
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sincy_adapter::sign::sign_definition;
use sincy_agent::{AgentClient, ScriptedIo};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";
const DEV_SIGNING_KEY: &[u8] = b"sincy-dev-adapter-signing-key";

const SETUP_YAML: &str = include_str!("../../../adapters/system.setup_install.yaml");

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "setup-e2e-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, device_sessions, devices, channel_capabilities, app_adapter_versions, app_adapters, agency_members, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'setup-e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("SetupE2e2026").expect("hash");
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'setup@sincy.dev', $3, 'owner')")
        .bind(member_id).bind(agency.as_uuid()).bind(hash)
        .execute(&pool).await.expect("member");

    // system.setup_install capability + 签名 adapter (dev key)
    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES
         ('system', 'setup_install', 'device', 'verified', 5, '{\"adapter\": \"system.setup_install\"}')",
    )
    .execute(&pool).await.expect("caps");
    let def = sincy_adapter::schema::AdapterDefinition::from_yaml(SETUP_YAML)
        .expect("valid system adapter");
    let canonical = serde_json::to_string(&def).expect("canonical");
    let sig = sign_definition(DEV_SIGNING_KEY, &canonical);
    sqlx::query("INSERT INTO app_adapters (id, app_package, description) VALUES ('system.setup_install', 'com.twitter.android', 'setup')")
        .execute(&pool).await.expect("adapter row");
    sqlx::query(
        "INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
         VALUES ('system.setup_install', 1, '>=0', $1::jsonb, $2, 'x', 'active')",
    )
    .bind(&canonical).bind(&sig)
    .execute(&pool).await.expect("adapter version");

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

    let login = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/auth/login"))
        .json(&json!({"email": "setup@sincy.dev", "password": "SetupE2e2026"}))
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
    let mgr = DeviceManager::new(pool, b"setup-e2e-secret");
    let code = mgr
        .create_pairing_code(agency, Uuid::new_v4(), chrono::Duration::minutes(10))
        .await
        .expect("code");
    let sess = mgr
        .pair_with_code(&code, "android", Some("sim"))
        .await
        .expect("pair");
    (sess.device_id, sess.token)
}

async fn wait_task_state(pool: &sqlx::PgPool, task_id: Uuid, want: &str, secs: u64) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let state: String = sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(pool)
            .await
            .expect("task");
        if state == want {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "task {task_id} stuck at {state}, want {want}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

#[tokio::test]
async fn setup_install_e2e() {
    setup_install_runs_signed_op_sequence_to_completed().await;
    setup_install_aborts_on_sha256_mismatch().await;
}

async fn setup_install_runs_signed_op_sequence_to_completed() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    // 1) 控制台派发引导安装任务
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/api/v1/setup/run"))
        .bearer_auth(&user_token)
        .json(&json!({"action": "setup_install"}))
        .send()
        .await
        .expect("setup/run")
        .json::<serde_json::Value>()
        .await
        .expect("json");
    assert_eq!(resp["state"], "QUEUED", "{resp}");
    assert_eq!(resp["route"], "device");
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    // 2) 模拟手机 agent 接任务 (app_version "0" — X 未安装, system adapter 跳过版本门)
    let agent = AgentClient {
        ws_url: format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={device_token}"),
        http_base: format!("http://127.0.0.1:{port}/api/v1"),
        token: device_token.clone(),
        device_id: device_id.to_string(),
        agency_id: agency.to_string(),
        signing_key: DEV_SIGNING_KEY.to_vec(),
    };
    let handle = tokio::spawn(async move {
        agent
            .run_next_task(
                || Box::new(ScriptedIo::new(vec![])),
                "0",
                std::time::Duration::from_secs(20),
            )
            .await
    });

    // 3) 任务应 COMPLETED, 且事件里有完整 op 序列证据
    wait_task_state(&pool, task_id, "COMPLETED", 20).await;
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(detail::text, '') FROM task_events WHERE task_id = $1 ORDER BY at",
    )
    .bind(task_id)
    .fetch_all(&pool)
    .await
    .expect("events");
    let joined = events.join("\n");
    for op in [
        "download_file",
        "install_apk",
        "wait_installed",
        "launch_app",
    ] {
        assert!(joined.contains(op), "events missing {op}:\n{joined}");
    }
    let exec = handle.await.expect("join").expect("agent run");
    assert!(
        matches!(exec.outcome, sincy_agent::TaskOutcome::Success),
        "outcome: {:?}",
        exec.outcome
    );
}

async fn setup_install_aborts_on_sha256_mismatch() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/api/v1/setup/run"))
        .bearer_auth(&user_token)
        .json(&json!({"action": "setup_install"}))
        .send()
        .await
        .expect("setup/run")
        .json::<serde_json::Value>()
        .await
        .expect("json");
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    let agent = AgentClient {
        ws_url: format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={device_token}"),
        http_base: format!("http://127.0.0.1:{port}/api/v1"),
        token: device_token.clone(),
        device_id: device_id.to_string(),
        agency_id: agency.to_string(),
        signing_key: DEV_SIGNING_KEY.to_vec(),
    };
    let handle = tokio::spawn(async move {
        // download_file 返回 sha256 不匹配
        let mut io = ScriptedIo::new(vec![]);
        io.system_results
            .push_back(Err("sha256 mismatch: got=deadbeef".to_string()));
        agent
            .run_next_task(|| Box::new(io), "0", std::time::Duration::from_secs(20))
            .await
    });

    wait_task_state(&pool, task_id, "FAILED", 20).await;
    // 只允许 download_file 一个 op 出现, 不许有 install_apk
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(detail::text, '') FROM task_events WHERE task_id = $1 ORDER BY at",
    )
    .bind(task_id)
    .fetch_all(&pool)
    .await
    .expect("events");
    let joined = events.join("\n");
    assert!(joined.contains("download_file"), "{joined}");
    assert!(
        !joined.contains("install_apk"),
        "sha mismatch must stop before install: {joined}"
    );
    let exec = handle.await.expect("join").expect("agent run");
    assert!(
        matches!(
            exec.outcome,
            sincy_agent::TaskOutcome::NoSideEffectFailure(_)
        ),
        "outcome: {:?}",
        exec.outcome
    );
    assert!(!exec.side_effect_done);
}
