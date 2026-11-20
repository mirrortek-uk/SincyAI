//! WP-05 + WP-06 joint acceptance: route selection (capability matrix) ->
//! quota reserve -> mock API execution -> receipt -> confirm, with the route
//! decision audited into execution_routes. Quota exhaustion must degrade
//! (NoSideEffect) instead of double-publishing.
//! Skips when SINCY_TEST_DATABASE_URL is unset.

use sincy_common::AgencyId;
use sincy_platform::matrix::PgCapabilityMatrix;
use sincy_platform::quota::QuotaLedger;
use sincy_platform::youtube_mock::MockYouTubeAdapter;
use sincy_routing::action::{ChannelAction, ExecCtx, ExecOutcome, PublishingPackageRef};
use sincy_routing::capability::Route;
use sincy_routing::router::{Preconditions, Router, RoutingOverrides, fallback_decision};
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
    sqlx::query("TRUNCATE execution_routes, routing_overrides, quota_ledger, quota_allocations, quota_pools, platform_apps, channel_capabilities, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'e2e')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .expect("agency");

    // capability matrix: api verified (priority 10), device unverified, assisted verified
    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints)
         VALUES
           ('youtube', 'publish_video', 'api', 'verified', 10, '{\"quota_cost\": 1600}'),
           ('youtube', 'publish_video', 'device', 'unverified', 20, '{}'),
           ('youtube', 'publish_video', 'assisted', 'verified', 30, '{}')",
    )
    .execute(&pool)
    .await
    .expect("capabilities");

    let app_id: Uuid = sqlx::query(
        "INSERT INTO platform_apps (platform, client_id, client_secret_ref)
         VALUES ('youtube', 'mock', 'vault://kv/mock') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("app")
    .get("id");
    let quota_pool_id: Uuid = sqlx::query(
        "INSERT INTO quota_pools (platform_app_id, quota_kind, limit_value, used_value)
         VALUES ($1, 'videos.insert', 100, 0) RETURNING id",
    )
    .bind(app_id)
    .fetch_one(&pool)
    .await
    .expect("pool")
    .get("id");

    Some((pool, agency, quota_pool_id))
}

fn ctx(task_id: Uuid) -> ExecCtx {
    ExecCtx {
        agency_id: "agency-e2e".into(),
        brand_id: "brand-1".into(),
        account_id: "acc-1".into(),
        task_id: task_id.to_string(),
        route_id: Uuid::new_v4().to_string(),
        idempotency_key: format!("idem-{task_id}"),
    }
}

fn pkg() -> PublishingPackageRef {
    PublishingPackageRef {
        package_id: "pkg-1".into(),
        version: 1,
        platform: "youtube".into(),
        action: "publish_video".into(),
    }
}

#[tokio::test]
async fn route_quota_execute_receipt_audit() {
    let Some((pool, agency, quota_pool_id)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let task_id = Uuid::new_v4();

    // 1. route selection from the PG capability matrix
    let matrix = PgCapabilityMatrix::new(&pool);
    let snap = matrix
        .snapshot("youtube", "publish_video")
        .await
        .expect("snapshot");
    let router = Router::new(&snap);
    let pre = Preconditions {
        api_ready: true,
        device_ready: false,
        device_block_reason: Some("device offline".into()),
        assisted_ready: true,
        ..Default::default()
    };
    let decision = router
        .select(
            "youtube",
            "publish_video",
            &RoutingOverrides::default(),
            &pre,
        )
        .expect("route");
    assert!(matches!(decision.reason.selected, Route::Api));

    // audit row
    sqlx::query(
        "INSERT INTO execution_routes (agency_id, task_id, platform, action, selected_route, capability_status, priority, skipped)
         VALUES ($1, $2, 'youtube', 'publish_video', 'api', 'verified', 10, $3)",
    )
    .bind(agency.as_uuid())
    .bind(task_id)
    .bind(serde_json::to_value(&decision.reason.skipped).unwrap())
    .execute(&pool).await.expect("audit route");

    // 2. quota reserve (1600 units > pool 100 => exhausted => QUOTA_WAIT semantics)
    let ledger = QuotaLedger::new(&pool);
    let reserve = ledger.reserve(agency, quota_pool_id, 1600, task_id).await;
    assert!(reserve.is_err(), "1600 > 100 must be exhausted");

    // degrade decision on NoSideEffectFailure goes to next route (assisted)
    let remaining = [Route::Assisted];
    let outcome = ExecOutcome::NoSideEffectFailure(sincy_common::AppError::no_side_effect(
        "quota_exhausted",
        "pool has 100 left, need 1600",
    ));
    assert!(matches!(
        fallback_decision(&outcome, &remaining),
        sincy_routing::router::Fallback::Degrade(Route::Assisted)
    ));

    // 3. with enough quota the mock adapter executes and we confirm
    sqlx::query("UPDATE quota_pools SET limit_value = 10000 WHERE id = $1")
        .bind(quota_pool_id)
        .execute(&pool)
        .await
        .unwrap();
    let r = ledger
        .reserve(agency, quota_pool_id, 1600, task_id)
        .await
        .expect("reserve ok");
    let adapter = MockYouTubeAdapter {
        quota_pool_id,
        quota_cost: 1600,
        simulate_quota_exhausted: false,
        platform: "youtube".into(),
    };
    assert!(adapter.precheck(&ctx(task_id), &pkg()).await.ok);
    let exec = adapter.execute(&ctx(task_id), &pkg()).await;
    match exec {
        ExecOutcome::Success {
            external_content_id,
            ..
        } => {
            assert!(external_content_id.unwrap().starts_with("mock-youtube-"));
        }
        _ => panic!("expected success"),
    }
    ledger.confirm(r.reservation_id).await.expect("confirm");
    assert_eq!(ledger.available(quota_pool_id).await.unwrap(), 8400);

    // 4. uncertain outcome must NOT degrade
    let uncertain = ExecOutcome::Uncertain(sincy_common::AppError::uncertain(
        "verification_pending",
        "clicked publish, no receipt",
    ));
    assert!(matches!(
        fallback_decision(&uncertain, &remaining),
        sincy_routing::router::Fallback::VerificationPending
    ));

    // route audit is queryable per tenant
    let row = sqlx::query("SELECT selected_route FROM execution_routes WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("selected_route"), "api");
}
