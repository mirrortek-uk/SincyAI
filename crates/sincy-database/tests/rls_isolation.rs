//! WP-03 acceptance: cross-agency reads/writes are rejected at the SQL/RLS
//! layer; platform-level behavior and missing tenant context fail closed.
//!
//! Runs against a real PostgreSQL when SINCY_TEST_DATABASE_URL is set
//! (local: postgres://postgres:password@127.0.0.1:5432/sincy_test).
//! Without the env var the test skips (CI has no database yet).

use sincy_common::AgencyId;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn test_pool() -> Option<sqlx::PgPool> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect test db");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    Some(pool)
}

/// Clean slate + two isolated agencies with one client/brand each.
async fn seed(pool: &sqlx::PgPool) -> (AgencyId, AgencyId) {
    sqlx::query(
        "TRUNCATE brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE",
    )
    .execute(pool)
    .await
    .expect("truncate");
    let a = AgencyId::new();
    let b = AgencyId::new();
    for (agency, name) in [(a, "Agency-A"), (b, "Agency-B")] {
        sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, $2)")
            .bind(agency.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("insert agency");
        let client_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, $3)")
            .bind(client_id)
            .bind(agency.as_uuid())
            .bind(format!("{name}-client"))
            .execute(pool)
            .await
            .expect("insert client");
        sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, $4)")
            .bind(uuid::Uuid::new_v4())
            .bind(agency.as_uuid())
            .bind(client_id)
            .bind(format!("{name}-brand"))
            .execute(pool)
            .await
            .expect("insert brand");
    }
    (a, b)
}

/// Connect as the unprivileged app role used by workers/API.
async fn app_pool(url: &str) -> sqlx::PgPool {
    // rewrite credentials to the sincy_app role created by migration 0002
    let app_url = url.replace(
        "postgres://postgres:password@",
        "postgres://sincy_app:sincy_app_dev_only@",
    );
    PgPoolOptions::new()
        .max_connections(3)
        .connect(&app_url)
        .await
        .expect("connect as sincy_app")
}

#[tokio::test]
async fn rls_blocks_cross_tenant_read_write() {
    let Some(pool) = test_pool().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let url = std::env::var(TEST_URL_ENV).expect("url");
    let (a, b) = seed(&pool).await;
    let app = app_pool(&url).await;

    // --- tenant A context sees only its own rows ---------------------------
    let mut tx = app.begin().await.expect("begin");
    sqlx::query(&format!(
        "SET LOCAL app.current_agency_id = '{}'",
        a.as_uuid()
    ))
    .execute(&mut *tx)
    .await
    .expect("set local");
    let rows = sqlx::query("SELECT name FROM brands")
        .fetch_all(&mut *tx)
        .await
        .expect("select brands");
    let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>("name")).collect();
    assert_eq!(
        names,
        vec!["Agency-A-brand".to_string()],
        "RLS must hide agency B"
    );

    // --- cross-tenant write is rejected by WITH CHECK ---------------------
    let client_b: uuid::Uuid = sqlx::query("SELECT id FROM clients WHERE agency_id = $1")
        .bind(b.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("client b")
        .get::<uuid::Uuid, _>("id");
    let insert_b =
        sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, $4)")
            .bind(uuid::Uuid::new_v4())
            .bind(b.as_uuid()) // forged tenant: client input must never win
            .bind(client_b)
            .bind("evil")
            .execute(&mut *tx)
            .await;
    assert!(
        insert_b.is_err(),
        "cross-tenant INSERT must be rejected by RLS WITH CHECK"
    );
    tx.rollback().await.expect("rollback");

    // --- no tenant context => fail closed (zero rows) ----------------------
    let rows = sqlx::query("SELECT count(*)::int8 as c FROM brands")
        .fetch_one(&app)
        .await
        .expect("count without context");
    assert_eq!(
        rows.get::<i64, _>("c"),
        0,
        "missing tenant context must yield zero rows"
    );

    // --- tenant B context sees only B --------------------------------------
    let mut tx = app.begin().await.expect("begin");
    sqlx::query(&format!(
        "SET LOCAL app.current_agency_id = '{}'",
        b.as_uuid()
    ))
    .execute(&mut *tx)
    .await
    .expect("set local");
    let rows = sqlx::query("SELECT name FROM brands")
        .fetch_all(&mut *tx)
        .await
        .expect("select");
    let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>("name")).collect();
    assert_eq!(names, vec!["Agency-B-brand".to_string()]);
    tx.rollback().await.expect("rollback");
}
