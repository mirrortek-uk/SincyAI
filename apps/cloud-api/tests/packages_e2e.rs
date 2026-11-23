//! Content-package flow (X 内容来源): create/list/approve + publish gate.
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sha2::Digest;
use sincy_common::AgencyId;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, AgencyId, u16, String, Uuid)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "pkg-test-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, quota_ledger, quota_pools, platform_apps, channel_capabilities, publishing_packages, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'pkg')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let hash = sincy_security::password::hash_password("PkgTest2026").unwrap();
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'pkg@sincy.dev', $3, 'owner')")
        .bind(Uuid::new_v4()).bind(agency.as_uuid()).bind(hash).execute(&pool).await.unwrap();
    let client_id = Uuid::new_v4();
    sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, 'c')")
        .bind(client_id)
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let brand_id = Uuid::new_v4();
    sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, 'b')")
        .bind(brand_id)
        .bind(agency.as_uuid())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    // x device route so publish can route
    sqlx::query("INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES ('x','publish_post','device','verified',10,'{\"adapter\":\"x.publish_post\"}'), ('x','publish_post','assisted','verified',90,'{}')")
        .execute(&pool).await.unwrap();

    // an active device so the device route is selectable (device_ready)
    let mgr = sincy_device::DeviceManager::new(&pool, b"pkg-test-secret");
    let ch = mgr
        .start_registration(agency, "android", Some("t"))
        .await
        .unwrap();
    let secret_hash = format!("{:x}", sha2::Sha256::digest(ch.device_secret.as_bytes()));
    let proof = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{secret_hash}{}", ch.nonce).as_bytes())
    );
    mgr.complete_registration(ch.device_id, &ch.nonce, &proof)
        .await
        .unwrap();
    sqlx::query("UPDATE devices SET last_seen = now() WHERE id = $1")
        .bind(ch.device_id)
        .execute(&pool)
        .await
        .unwrap();

    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    let app = axum::Router::new()
        .nest("/api/v1", cloud_api::routes())
        .with_state(state)
        .fallback(cloud_api::api::not_found);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let login = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/auth/login"))
        .json(&json!({"email": "pkg@sincy.dev", "password": "PkgTest2026"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    Some((
        pool,
        agency,
        port,
        login["token"].as_str().unwrap().to_string(),
        brand_id,
    ))
}

#[tokio::test]
async fn package_lifecycle_and_publish_gate() {
    let Some((_pool, _agency, port, token, brand_id)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let client = reqwest::Client::new();
    let auth = |r: reqwest::RequestBuilder| r.bearer_auth(&token);
    let base = format!("http://127.0.0.1:{port}/api/v1");

    // brands list
    let brands = auth(client.get(format!("{base}/brands")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(
        brands["items"][0]["id"].as_str().unwrap(),
        brand_id.to_string()
    );

    // manual package: approved immediately
    let created = auth(client.post(format!("{base}/packages")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post",
        "text": "manual x post"
    })))
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(created["approval_status"], "approved");
    let _pid = created["package_id"].as_str().unwrap().to_string();

    // x 280-char limit
    let too_long = auth(client.post(format!("{base}/packages")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post", "text": "a".repeat(281)
    })))
    .send()
    .await
    .unwrap();
    assert_eq!(too_long.status(), 409);

    // AI-style package starts pending and publish is gated until approved
    let pending = auth(client.post(format!("{base}/packages")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post", "text": "ai draft"
    })))
    .send()
    .await
    .unwrap();
    // simulate AI source via ai/generate path is covered by unit level; here use approve flow:
    // create via ai/generate with save_as_package (mock provider allowed in test state)
    let gen_resp = auth(client.post(format!("{base}/ai/generate")).json(&json!({
        "prompt": "tweet", "save_as_package": {"brand_id": brand_id, "platform": "x", "action": "publish_post"}
    }))).send().await.unwrap().json::<serde_json::Value>().await.unwrap();
    let ai_pid = gen_resp["package_id"]
        .as_str()
        .expect("ai generate saves package")
        .to_string();

    let gate = auth(client.post(format!("{base}/tasks/publish")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post",
        "package_id": ai_pid, "package_version": 1, "idempotency_key": format!("g-{}", Uuid::new_v4())
    }))).send().await.unwrap();
    assert_eq!(
        gate.status(),
        409,
        "pending package must not publish: {gate:?}"
    );

    let appr = auth(client.post(format!("{base}/packages/{ai_pid}/approve")))
        .send()
        .await
        .unwrap();
    assert_eq!(appr.status(), 200);

    let pub_resp = auth(client.post(format!("{base}/tasks/publish")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post",
        "package_id": ai_pid, "package_version": 1, "idempotency_key": format!("p-{}", Uuid::new_v4())
    }))).send().await.unwrap().json::<serde_json::Value>().await.unwrap();
    assert_eq!(pub_resp["route"], "device");
    assert_eq!(pub_resp["status"], "DEVICE_QUEUED");

    // list packages filtered by platform
    let list = auth(client.get(format!("{base}/packages?platform=x&action=publish_post")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(list["items"].as_array().unwrap().len() >= 2);

    let _ = pending;
}
