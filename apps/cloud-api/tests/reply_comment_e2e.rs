//! x.reply_comment 端到端: 控制台 publish(action=reply_comment) -> device 路由
//!   -> 签名 x.reply_comment adapter -> 设备 flow: 通知/Mentions/点开/Reply/输入/发布/验签
//!   -> COMPLETED, 且 set_text 步骤证据里必须出现内容包正文 (package.text 替换生效)。
//!
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sha2::{Digest, Sha256};
use sincy_adapter::interpreter::MockSnapshot;
use sincy_adapter::schema::AdapterDefinition;
use sincy_adapter::sign::sign_definition;
use sincy_agent::{AgentClient, ScriptedIo};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";
const DEV_SIGNING_KEY: &[u8] = b"sincy-dev-adapter-signing-key";
const REPLY_ADAPTER_YAML: &str = include_str!("../../../adapters/x.reply_comment.yaml");

const REPLY_TEXT: &str = "Thanks for the mention — more on agent skills soon!";

fn proof_for(secret: &str, nonce: &str) -> String {
    let secret_hash = format!("{:x}", Sha256::digest(secret.as_bytes()));
    format!(
        "{:x}",
        Sha256::digest(format!("{secret_hash}{nonce}").as_bytes())
    )
}

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "reply-e2e-secret") };
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
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'reply-e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("ReplyE2e2026").expect("hash");
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'reply@sincy.dev', $3, 'owner')")
        .bind(member_id).bind(agency.as_uuid()).bind(hash)
        .execute(&pool).await.expect("member");

    // capability: x reply_comment走 device 路由
    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES
         ('x', 'reply_comment', 'device', 'verified', 95, '{\"app_package\": \"com.twitter.android\", \"adapter\": \"x.reply_comment\"}')",
    )
    .execute(&pool).await.expect("caps");

    let def = AdapterDefinition::from_yaml(REPLY_ADAPTER_YAML).expect("valid");
    let canonical = serde_json::to_string(&def).expect("canonical");
    let sig = sign_definition(DEV_SIGNING_KEY, &canonical);
    sqlx::query("INSERT INTO app_adapters (id, app_package, description) VALUES ('x.reply_comment', 'com.twitter.android', 'reply adapter')")
        .execute(&pool).await.expect("adapter row");
    sqlx::query(
        "INSERT INTO app_adapter_versions (adapter_id, version, version_range, definition, signature, sha256, state)
         VALUES ('x.reply_comment', 1, '>=10.0 <11.0', $1::jsonb, $2, 'x', 'active')",
    )
    .bind(&canonical).bind(&sig)
    .execute(&pool).await.expect("adapter version");

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
         VALUES ($1, 1, $2, $3, 'x', $4, 'reply_comment', $5, 'approved')",
    )
    .bind(package_id).bind(agency.as_uuid()).bind(brand_id).bind(client_id).bind(REPLY_TEXT)
    .execute(&pool).await.expect("package");

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
        .json(&json!({"email": "reply@sincy.dev", "password": "ReplyE2e2026"}))
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
    let mgr = DeviceManager::new(pool, b"reply-e2e-secret");
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
async fn x_reply_flows_through_device_agent_and_types_package_text() {
    let Some((pool, agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (device_id, device_token) = register_device(&pool, agency).await;

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
            "action": "reply_comment",
            "package_id": package_id,
            "package_version": 1,
            "idempotency_key": format!("reply-{}", Uuid::new_v4()),
        }))
        .send()
        .await
        .expect("publish")
        .json::<serde_json::Value>()
        .await
        .expect("publish json");
    assert_eq!(
        pub_resp["route"], "device",
        "reply must route to device: {pub_resp}"
    );
    assert_eq!(pub_resp["status"], "DEVICE_QUEUED");
    let task_id: Uuid = pub_resp["task_id"].as_str().unwrap().parse().unwrap();

    // 简报里 adapter 应为 x.reply_comment, task_context 带 package_text
    let brief = client
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
    assert_eq!(brief["adapter_id"], "x.reply_comment", "{brief}");
    assert_eq!(
        brief["task_context"]["package_text"].as_str().unwrap(),
        REPLY_TEXT,
        "brief must carry package_text: {brief}"
    );

    // 模拟手机屏幕序列: home -> notifications -> mention_list -> tweet_detail -> composer -> composer -> reply_sent -> reply_sent
    let screens = vec![
        MockSnapshot::new(
            vec!["com.twitter.android:id/bottom_bar".into()],
            vec!["Home".into()],
        ),
        MockSnapshot::new(vec![], vec!["Notifications".into(), "Mentions".into()]),
        MockSnapshot::new(vec![], vec!["Mentions".into(), "replied to you".into()]),
        MockSnapshot::new(
            vec!["com.twitter.android:id/tweet".into()],
            vec!["Reply".into()],
        ),
        MockSnapshot::new(
            vec!["com.twitter.android:id/tweet_text".into()],
            vec!["Post your reply".into()],
        ),
        MockSnapshot::new(
            vec!["com.twitter.android:id/tweet_text".into()],
            vec!["Post your reply".into()],
        ),
        MockSnapshot::new(vec![], vec!["Your reply was sent".into()]),
        MockSnapshot::new(vec![], vec!["Your reply was sent".into()]),
    ];
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
                move || Box::new(ScriptedIo::new(screens)),
                "10.5.0",
                std::time::Duration::from_secs(20),
            )
            .await
    });

    // 等任务 COMPLETED
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let state: String = sqlx::query_scalar("SELECT state FROM device_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(&pool)
            .await
            .expect("task");
        if state == "COMPLETED" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "task stuck at {state}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    let exec = handle.await.expect("join").expect("agent run");
    assert!(
        matches!(exec.outcome, sincy_agent::TaskOutcome::Success),
        "outcome: {:?}",
        exec.outcome
    );
    assert!(exec.side_effect_done, "reply Post step is the side effect");

    // 关键断言: set_text 步骤证据里出现的是内容包正文, 而不是占位符 package.text
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(detail::text, '') FROM task_events WHERE task_id = $1 ORDER BY at",
    )
    .bind(task_id)
    .fetch_all(&pool)
    .await
    .expect("events");
    let joined = events.join("\n");
    assert!(
        joined.contains(REPLY_TEXT),
        "package text must be substituted into set_text evidence:\n{joined}"
    );
    assert!(
        !joined.contains("package.text"),
        "placeholder must not leak into evidence:\n{joined}"
    );
}
