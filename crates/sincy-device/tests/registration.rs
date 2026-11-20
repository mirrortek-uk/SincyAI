//! WP-07 acceptance: 伪造 device_id 无法认证; 撤销设备后连接失败;
//! 能力与 App 版本更新有审计(可查). Skips without SINCY_TEST_DATABASE_URL.

use sha2::{Digest, Sha256};
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, AgencyId)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE device_app_versions, device_capabilities, device_sessions, account_device_bindings, social_accounts, devices, automation_consents, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'device-test')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");
    Some((pool, agency))
}

fn proof_for(secret: &str, nonce: &str) -> String {
    let secret_hash = {
        let mut h = Sha256::new();
        h.update(secret.as_bytes());
        format!("{:x}", h.finalize())
    };
    let mut h = Sha256::new();
    h.update(secret_hash.as_bytes());
    h.update(nonce.as_bytes());
    format!("{:x}", h.finalize())
}

#[tokio::test]
async fn register_activate_authenticate_revoke() {
    let Some((pool, agency)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let mgr = DeviceManager::new(&pool, b"test-jwt-secret");

    // 1. start registration
    let ch = mgr
        .start_registration(agency, "android", Some("pixel-8"))
        .await
        .expect("start");
    // 2. wrong proof rejected (forged)
    let bad = mgr
        .complete_registration(ch.device_id, &ch.nonce, "forged")
        .await;
    assert!(bad.is_err(), "forged proof must be rejected");
    // 3. correct proof activates
    let session = mgr
        .complete_registration(
            ch.device_id,
            &ch.nonce,
            &proof_for(&ch.device_secret, &ch.nonce),
        )
        .await
        .expect("complete");
    // 4. authenticate with the session token
    let (device_id, agency_id) = mgr.authenticate(&session.token).await.expect("auth");
    assert_eq!(device_id, ch.device_id);
    assert_eq!(agency_id, agency);

    // 5. forged device_id token rejected
    let forged = mgr.jwt_issue_forged(Uuid::new_v4(), agency);
    assert!(mgr.authenticate(&forged).await.is_err());

    // 6. capabilities + app versions reported and queryable
    mgr.update_capabilities(
        agency,
        ch.device_id,
        &[
            ("accessibility".into(), true, "enabled".into()),
            ("media_projection".into(), false, "denied".into()),
        ],
    )
    .await
    .expect("caps");
    mgr.update_app_versions(
        agency,
        ch.device_id,
        &[("com.xhs".into(), "8.20.1".into(), 82001)],
    )
    .await
    .expect("apps");
    let caps: i64 = sqlx::query("SELECT count(*) FROM device_capabilities WHERE device_id = $1")
        .bind(ch.device_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(caps, 2);
    let app: String = sqlx::query(
        "SELECT version_name FROM device_app_versions WHERE device_id = $1 AND package = 'com.xhs'",
    )
    .bind(ch.device_id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(app, "8.20.1");

    // 7. heartbeat updates last_seen
    mgr.heartbeat(ch.device_id).await.expect("hb");
    let seen: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query("SELECT last_seen FROM devices WHERE id = $1")
            .bind(ch.device_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert!(seen.is_some());

    // 8. revoke => authenticate fails afterwards
    mgr.revoke(agency, ch.device_id).await.expect("revoke");
    let after = mgr.authenticate(&session.token).await;
    assert!(after.is_err(), "revoked device must be rejected");
}
