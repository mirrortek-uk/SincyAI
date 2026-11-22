//! system.agentic ("真人员工" LLM 闭环) 端到端测试:
//!
//!   控制台 POST /setup/run {action:"agentic", goal:"..."} -> device 任务 QUEUED
//!     -> WSS 派发 -> agent 拉签名简报 (含 task_context.goal)
//!     -> Rust core 每步: snapshot -> llm_decide -> 白名单动作 -> perform
//!     -> done => COMPLETED; 越权动作 => FAILED 零副作用; 验证页 => WAITING_USER
//!
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sincy_adapter::interpreter::MockSnapshot;
use sincy_adapter::sign::sign_definition;
use sincy_agent::{AgentClient, ScriptedIo};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";
const DEV_SIGNING_KEY: &[u8] = b"sincy-dev-adapter-signing-key";

const AGENTIC_YAML: &str = include_str!("../../../adapters/system.agentic.yaml");

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "agentic-e2e-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, device_sessions, devices, channel_capabilities, app_adapter_versions, app_adapters, setup_app_catalog, agency_members, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'agentic-e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("AgenticE2e2026").expect("hash");
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'agentic@sincy.dev', $3, 'owner')")
        .bind(member_id).bind(agency.as_uuid()).bind(hash)
        .execute(&pool).await.expect("member");

    // system capability (run_setup 先查 system/setup_install 的 device 路由) + 签名 agentic adapter
    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES
         ('system', 'setup_install', 'device', 'verified', 5, '{\"adapter\": \"system.agentic\"}')",
    )
    .execute(&pool).await.expect("caps");
    let def = sincy_adapter::schema::AdapterDefinition::from_yaml(AGENTIC_YAML)
        .expect("valid agentic adapter");
    let canonical = serde_json::to_string(&def).expect("canonical");
    let sig = sign_definition(DEV_SIGNING_KEY, &canonical);
    sqlx::query("INSERT INTO app_adapters (id, app_package, description) VALUES ('system.agentic', 'com.android.settings', 'agentic')")
        .execute(&pool).await.expect("adapter row");
    sqlx::query(
        "INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
         VALUES ('system.agentic', 1, '>=0', $1::jsonb, $2, 'x', 'active')",
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
        .json(&json!({"email": "agentic@sincy.dev", "password": "AgenticE2e2026"}))
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
    let mgr = DeviceManager::new(pool, b"agentic-e2e-secret");
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

async fn run_agent(
    pool: &sqlx::PgPool,
    agency: AgencyId,
    port: u16,
    device_token: &str,
    device_id: Uuid,
    io: ScriptedIo,
) -> sincy_agent::AdapterExecution {
    let agent = AgentClient {
        ws_url: format!("ws://127.0.0.1:{port}/api/v1/agent/connect?token={device_token}"),
        http_base: format!("http://127.0.0.1:{port}/api/v1"),
        token: device_token.to_string(),
        device_id: device_id.to_string(),
        agency_id: agency.to_string(),
        signing_key: DEV_SIGNING_KEY.to_vec(),
    };
    let _ = pool;
    agent
        .run_next_task(|| Box::new(io), "0", std::time::Duration::from_secs(20))
        .await
        .expect("agent run")
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

async fn dispatch(port: u16, user_token: &str, goal: &str) -> serde_json::Value {
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/setup/run"))
        .bearer_auth(user_token)
        .json(&json!({"action": "agentic", "goal": goal}))
        .send()
        .await
        .expect("setup/run")
        .json::<serde_json::Value>()
        .await
        .expect("json")
}

#[tokio::test]
async fn agentic_e2e() {
    agentic_loop_completes_via_llm_decisions().await;
    agentic_disallowed_action_fails_without_side_effect().await;
    agentic_challenge_screen_parks_waiting_user().await;
    agentic_missing_goal_rejected().await;
    agentic_identical_decision_loop_fails_fast().await;
}

/// LLM 连续 3 次返回完全相同的决策 => 立即 FAILED (loop detected), 不烧完 max_steps
async fn agentic_identical_decision_loop_fails_fast() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    let resp = dispatch(port, &user_token, "点开 Play 商店").await;
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    let mut io = ScriptedIo::new(vec![
        MockSnapshot::new(vec![], vec!["桌面".into()]),
        MockSnapshot::new(vec![], vec!["桌面".into()]),
        MockSnapshot::new(vec![], vec!["桌面".into()]),
        MockSnapshot::new(vec![], vec!["桌面".into()]),
    ]);
    let same_tap = json!({"action": "tap", "target": {"text": "Play 商店"}});
    for _ in 0..4 {
        io.llm_script.push_back(Ok(same_tap.clone()));
    }
    let handle = tokio::spawn({
        let pool = pool.clone();
        let device_token = device_token.clone();
        async move { run_agent(&pool, agency, port, &device_token, device_id, io).await }
    });

    wait_task_state(&pool, task_id, "FAILED", 20).await;
    let exec = handle.await.expect("join");
    match &exec.outcome {
        sincy_agent::TaskOutcome::NoSideEffectFailure(reason) => {
            assert!(reason.contains("loop detected"), "reason: {reason}");
        }
        other => panic!("expected NoSideEffectFailure(loop detected), got {other:?}"),
    }
    // 只跑了 3 步 (第 4 次决策前就 fail-fast), 不是 30 步
    let steps: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM task_events WHERE task_id = $1 AND event = 'device.step.evidence'",
    )
    .bind(task_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert!(steps <= 3, "should fail fast, got {steps} steps");
}

/// LLM 决定 tap -> done: 任务 COMPLETED, performed 留下 tap 证据, 简报带 goal
async fn agentic_loop_completes_via_llm_decisions() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    let resp = dispatch(port, &user_token, "打开设置里的电池页面").await;
    assert_eq!(resp["state"], "QUEUED", "{resp}");
    assert_eq!(resp["adapter"], "system.agentic", "{resp}");
    assert_eq!(resp["action"], "agentic", "{resp}");
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    // 简报里必须带 task_context.goal (数据, 非签名)
    let brief = reqwest::Client::new()
        .get(format!(
            "http://127.0.0.1:{port}/api/v1/agent/tasks/{task_id}?token={device_token}"
        ))
        .bearer_auth(&device_token)
        .send()
        .await
        .expect("brief")
        .json::<serde_json::Value>()
        .await
        .expect("brief json");
    assert_eq!(brief["adapter_id"], "system.agentic", "{brief}");
    assert_eq!(
        brief["task_context"]["goal"].as_str().unwrap(),
        "打开设置里的电池页面",
        "{brief}"
    );

    let mut io = ScriptedIo::new(vec![
        MockSnapshot::new(
            vec!["com.android.settings:id/home".into()],
            vec!["设置".into()],
        ),
        MockSnapshot::new(
            vec!["com.android.settings:id/battery".into()],
            vec!["电池".into()],
        ),
    ]);
    io.llm_script.push_back(Ok(
        json!({"action": "tap", "target": {"id": "com.android.settings:id/battery"}}),
    ));
    io.llm_script.push_back(Ok(json!({"action": "done"})));
    let handle = tokio::spawn({
        let pool = pool.clone();
        let device_token = device_token.clone();
        async move { run_agent(&pool, agency, port, &device_token, device_id, io).await }
    });

    wait_task_state(&pool, task_id, "COMPLETED", 20).await;
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(detail::text, '') FROM task_events WHERE task_id = $1 ORDER BY at",
    )
    .bind(task_id)
    .fetch_all(&pool)
    .await
    .expect("events");
    let joined = events.join("\n");
    assert!(
        joined.contains("step 1: tap"),
        "events missing tap step:\n{joined}"
    );
    let exec = handle.await.expect("join");
    assert!(
        matches!(exec.outcome, sincy_agent::TaskOutcome::Success),
        "outcome: {:?}",
        exec.outcome
    );
    assert!(!exec.side_effect_done, "agentic 白名单不允许副作用动作");
}

/// LLM 越权返回 install_apk (不在签名白名单): FAILED 且没有任何 perform
async fn agentic_disallowed_action_fails_without_side_effect() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    let resp = dispatch(port, &user_token, "帮我安装一个应用").await;
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    let mut io = ScriptedIo::new(vec![MockSnapshot::new(vec![], vec!["桌面".into()])]);
    io.llm_script
        .push_back(Ok(json!({"action": "install_apk"})));
    let handle = tokio::spawn({
        let pool = pool.clone();
        let device_token = device_token.clone();
        async move { run_agent(&pool, agency, port, &device_token, device_id, io).await }
    });

    wait_task_state(&pool, task_id, "FAILED", 20).await;
    let exec = handle.await.expect("join");
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

/// 屏幕出现验证文本 => 直接 WAITING_USER, LLM 根本不会被问到
async fn agentic_challenge_screen_parks_waiting_user() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

    let resp = dispatch(port, &user_token, "登录 X 账号").await;
    let task_id: Uuid = resp["task_id"].as_str().unwrap().parse().unwrap();

    let io = ScriptedIo::new(vec![MockSnapshot::new(
        vec![],
        vec!["安全检测".into(), "请完成验证".into()],
    )]);
    let handle = tokio::spawn({
        let pool = pool.clone();
        let device_token = device_token.clone();
        async move { run_agent(&pool, agency, port, &device_token, device_id, io).await }
    });

    wait_task_state(&pool, task_id, "WAITING_USER", 20).await;
    let exec = handle.await.expect("join");
    assert!(
        matches!(exec.outcome, sincy_agent::TaskOutcome::WaitingUser(_)),
        "outcome: {:?}",
        exec.outcome
    );
}

/// agentic 不带 goal 直接 400
async fn agentic_missing_goal_rejected() {
    let Some((_pool, _agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/setup/run"))
        .bearer_auth(&user_token)
        .json(&json!({"action": "agentic"}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 400, "缺 goal 必须拒绝");
}
