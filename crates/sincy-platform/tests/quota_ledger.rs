//! WP-06 acceptance (quota part): reserve/confirm/release never over-issue;
//! repeated reservation for the same task is idempotent; exhaustion is reported.
//! Skips when SINCY_TEST_DATABASE_URL is unset.

use sincy_common::AgencyId;
use sincy_platform::quota::{QuotaError, QuotaLedger};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, AgencyId, Uuid)> {
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

    sqlx::query("TRUNCATE quota_ledger, quota_allocations, quota_pools, platform_apps, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'quota-test')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");

    let app_id: Uuid = sqlx::query(
        "INSERT INTO platform_apps (platform, client_id, client_secret_ref)
         VALUES ('youtube', 'mock-client-id', 'vault://kv/mock/youtube/secret')
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("platform_app")
    .get("id");

    let pool_id: Uuid = sqlx::query(
        "INSERT INTO quota_pools (platform_app_id, quota_kind, limit_value, used_value, reset_cycle)
         VALUES ($1, 'videos.insert', 100, 0, 'daily')
         RETURNING id",
    )
    .bind(app_id)
    .fetch_one(&pool).await.expect("quota_pool")
    .get("id");

    Some((pool, agency, pool_id))
}

#[tokio::test]
async fn reserve_confirm_release_and_exhaustion() {
    let Some((pool, agency, quota_pool_id)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let ledger = QuotaLedger::new(&pool);
    let task1 = Uuid::new_v4();
    let task2 = Uuid::new_v4();

    // reserve 60 of 100
    let r1 = ledger
        .reserve(agency, quota_pool_id, 60, task1)
        .await
        .expect("reserve 60");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 40);

    // idempotent: same task again returns the same reservation, no double charge
    let r1b = ledger
        .reserve(agency, quota_pool_id, 60, task1)
        .await
        .expect("re-reserve");
    assert_eq!(r1.reservation_id, r1b.reservation_id);
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 40);

    // another task asking 60 > 40 available => exhausted
    let err = ledger
        .reserve(agency, quota_pool_id, 60, task2)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        QuotaError::Exhausted {
            requested: 60,
            available: 40,
            ..
        }
    ));

    // confirm task1 (side effect happened): used stays 60
    ledger.confirm(r1.reservation_id).await.expect("confirm");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 40);

    // release a fresh reservation returns units
    let task3 = Uuid::new_v4();
    let r3 = ledger
        .reserve(agency, quota_pool_id, 30, task3)
        .await
        .expect("reserve 30");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 10);
    ledger.release(r3.reservation_id).await.expect("release");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 40);

    // releasing twice is a no-op (idempotent)
    ledger
        .release(r3.reservation_id)
        .await
        .expect("release again");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 40);
}
