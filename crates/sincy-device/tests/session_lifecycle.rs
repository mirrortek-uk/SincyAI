//! 设备会话生命周期: 注册后 token_hash 精确匹配 + 60 天过期; 撤销/轮换后旧 token 立即失效。
use sha2::Digest;
use sincy_common::AgencyId;
use sincy_device::DeviceManager;
use sqlx::postgres::PgPoolOptions;

async fn fresh_mgr() -> (sqlx::PgPool, DeviceManager<'static>, AgencyId) {
    let url = std::env::var("SINCY_TEST_DATABASE_URL").expect("SINCY_TEST_DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'sess') ON CONFLICT DO NOTHING")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mgr = DeviceManager::new(Box::leak(Box::new(pool.clone())), b"sess-secret");
    (pool, mgr, agency)
}

async fn register(mgr: &DeviceManager<'_>, agency: AgencyId) -> sincy_device::SessionToken {
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
        .unwrap()
}

#[tokio::test]
async fn session_token_hash_matches_and_expires_in_60d() {
    let (_pool, mgr, agency) = fresh_mgr().await;
    let st = register(&mgr, agency).await;
    let (device_id, _) = mgr.authenticate(&st.token).await.expect("auth ok");
    assert_eq!(device_id, st.device_id);
    let row = sqlx::query(
        "SELECT expires_at FROM device_sessions WHERE device_id = $1 AND state = 'active'",
    )
    .bind(st.device_id)
    .fetch_one(&_pool)
    .await
    .unwrap();
    use sqlx::Row;
    let exp: chrono::DateTime<chrono::Utc> = row.get("expires_at");
    let days = (exp - chrono::Utc::now()).num_days();
    assert!((59..=60).contains(&days), "expires in {days} days");
}

#[tokio::test]
async fn revoke_and_rotate_kill_old_tokens() {
    let (_pool, mgr, agency) = fresh_mgr().await;
    let st = register(&mgr, agency).await;
    // revoke: 旧 token 立即失效
    mgr.revoke(agency, st.device_id).await.unwrap();
    assert!(mgr.authenticate(&st.token).await.is_err());
    // 重新激活后 rotate: 旧 token 失效, 新 token 可用
    sqlx::query("UPDATE devices SET state = 'active' WHERE id = $1")
        .bind(st.device_id)
        .execute(&_pool)
        .await
        .unwrap();
    let st2 = mgr.rotate(agency, st.device_id).await.unwrap();
    assert!(
        mgr.authenticate(&st.token).await.is_err(),
        "old token must die after rotate"
    );
    assert!(
        mgr.authenticate(&st2.token).await.is_ok(),
        "new token must work"
    );
}
