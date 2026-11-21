//! One-time pairing-code flow, end to end against a real server + real DB:
//!
//!   owner (JWT) -> POST /devices/pairing-codes -> 8-char code
//!   phone (NO auth header) -> POST /devices/pair {code} -> device_id + token
//!   device token works on device routes (GET /agent/tasks/{id} no longer 401)
//!   wrong code -> 400, reused code -> 400, expired code -> 400
//!
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sincy_common::AgencyId;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "pairing-e2e-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE device_pairing_codes, device_sessions, devices, agency_members, agencies RESTART IDENTITY CASCADE")
        .execute(&pool)
        .await
        .expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'pairing-e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("PairingE2e2026").expect("hash");
    sqlx::query(
        "INSERT INTO agency_members (id, agency_id, email, password_hash, role)
         VALUES ($1, $2, 'pair@sincy.dev', $3, 'owner')",
    )
    .bind(member_id)
    .bind(agency.as_uuid())
    .bind(hash)
    .execute(&pool)
    .await
    .expect("member");

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
        .json(&json!({"email": "pair@sincy.dev", "password": "PairingE2e2026"}))
        .send()
        .await
        .expect("login")
        .json::<serde_json::Value>()
        .await
        .expect("login json");
    let user_token = login["token"].as_str().expect("token").to_string();

    Some((pool, agency, port, user_token))
}

#[tokio::test]
async fn pairing_code_pairs_device_without_password() {
    let Some((pool, _agency, port, user_token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let http = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}/api/v1");

    // 1) 未带 JWT 不能生成配对码
    let resp = http
        .post(format!("{base}/devices/pairing-codes"))
        .json(&json!({}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 401, "pairing-codes must require auth");

    // 2) owner 生成配对码
    let resp = http
        .post(format!("{base}/devices/pairing-codes"))
        .bearer_auth(&user_token)
        .json(&json!({"ttl_minutes": 10}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 200, "create code: {:?}", resp.text().await);
    let body: serde_json::Value = resp.json().await.expect("json");
    let code = body["code"].as_str().expect("code").to_string();
    assert_eq!(code.len(), 4, "code is 4 digits: {code}");
    assert!(
        code.bytes().all(|b| b.is_ascii_digit()),
        "digits only: {code}"
    );
    assert!(body["expires_at"].is_string());

    // 3) 手机端不带任何认证头, 只凭码配对
    let resp = http
        .post(format!("{base}/devices/pair"))
        .json(&json!({"code": code, "platform": "android", "name": "e2e-pixel"}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 200, "pair: {:?}", resp.text().await);
    let paired: serde_json::Value = resp.json().await.expect("json");
    let device_id = paired["device_id"].as_str().expect("device_id").to_string();
    let device_token = paired["token"].as_str().expect("token").to_string();
    assert!(!device_token.is_empty());

    // 4) 设备 token 能通过设备路由认证 (查不存在的任务应是 404 而不是 401)
    let resp = http
        .get(format!("{base}/agent/tasks/{}", Uuid::new_v4()))
        .bearer_auth(&device_token)
        .send()
        .await
        .expect("send");
    assert_ne!(resp.status(), 401, "device token must authenticate");

    // 5) 设备出现在列表里且 active
    let resp = http
        .get(format!("{base}/devices"))
        .bearer_auth(&user_token)
        .send()
        .await
        .expect("send");
    let list: serde_json::Value = resp.json().await.expect("json");
    let found = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .any(|d| d["id"].as_str() == Some(&device_id) && d["state"] == "active");
    assert!(found, "paired device listed as active: {list}");

    // 6) 码只能用一次: 再用 -> 400
    let resp = http
        .post(format!("{base}/devices/pair"))
        .json(&json!({"code": code}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 400, "reused code must fail");

    // 7) 错码 -> 400 (把生成码的最后一位改掉, 保证与已用码不同)
    let wrong = format!(
        "{}{}",
        &code[..3],
        if code.ends_with('9') { '8' } else { '9' }
    );
    let resp = http
        .post(format!("{base}/devices/pair"))
        .json(&json!({"code": wrong}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 400, "wrong code must fail");

    // 7b) 非 4 位数字的码 -> 400
    for bad in ["12", "12345", "ABCD", "12 4"] {
        let resp = http
            .post(format!("{base}/devices/pair"))
            .json(&json!({"code": bad}))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 400, "malformed code {bad:?} must fail");
    }

    // 8) 过期码 -> 400 (直接塞一条已过期的码)
    let expired_code = format!(
        "{}{}",
        if code.starts_with('9') { '8' } else { '9' },
        &code[1..]
    );
    sqlx::query(
        "INSERT INTO device_pairing_codes (code, agency_id, created_by, expires_at)
         VALUES ($2, (SELECT id FROM agencies LIMIT 1), $1, now() - interval '1 minute')",
    )
    .bind(Uuid::new_v4())
    .bind(&expired_code)
    .execute(&pool)
    .await
    .expect("insert expired");
    let resp = http
        .post(format!("{base}/devices/pair"))
        .json(&json!({"code": expired_code}))
        .send()
        .await
        .expect("send");
    assert_eq!(resp.status(), 400, "expired code must fail");
}
