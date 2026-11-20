//! Integration tests for WP-14: FR-026 budget circuit breaker + FR-016 analytics.
//! Skipped unless SINCY_TEST_DATABASE_URL is set (see sincy-database tests).

use rust_decimal::Decimal;
use sincy_ai::{Analytics, BudgetChecker, CostRecord, MetricImport};
use sincy_common::AgencyId;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("SINCY_TEST_DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.ok()?;
    sqlx::migrate!("../../migrations").run(&pool).await.ok()?;
    Some(pool)
}

/// Create a tenant agency row (RLS needs app.current_agency_id set per tx).
async fn make_agency(pool: &PgPool) -> AgencyId {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("ai-test-{}", id.simple()))
        .execute(pool)
        .await
        .expect("insert agency");
    AgencyId::from_uuid(id)
}

#[tokio::test]
async fn budget_blocks_over_limit_and_records_costs() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: SINCY_TEST_DATABASE_URL not set");
        return;
    };
    let agency = make_agency(&pool).await;
    let checker = BudgetChecker::new(&pool, Some(Decimal::new(10, 0))); // $10/mo

    // $6 fits
    checker
        .check(agency, None, Decimal::new(6, 0))
        .await
        .expect("6 fits");
    // record $6
    checker
        .record_cost(
            agency,
            &CostRecord {
                brand_id: None,
                content_item_id: None,
                provider: "mock",
                model: "mock-1",
                kind: "llm",
                tokens_in: 10,
                tokens_out: 20,
                cost_usd: Decimal::new(6, 0),
            },
        )
        .await
        .expect("record");
    // another $6 would exceed (6+6 > 10)
    let err = checker
        .check(agency, None, Decimal::new(6, 0))
        .await
        .expect_err("must be over budget");
    assert!(err.to_string().contains("budget exceeded"), "got: {err}");
    // $4 exactly fits (6+4 == 10, not >)
    checker
        .check(agency, None, Decimal::new(4, 0))
        .await
        .expect("exactly at limit fits");
}

#[tokio::test]
async fn analytics_import_and_snapshot() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: SINCY_TEST_DATABASE_URL not set");
        return;
    };
    let agency = make_agency(&pool).await;
    let analytics = Analytics::new(&pool);
    let content_id = Uuid::new_v4();

    analytics
        .import_metrics(
            agency,
            &MetricImport {
                platform: "youtube".into(),
                external_content_id: "yt-abc".into(),
                account_id: None,
                metric_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                metrics: serde_json::json!({"views": 100, "likes": 5}),
                source: "api".into(),
            },
        )
        .await
        .expect("import");

    let snap = analytics
        .snapshot_performance(agency, content_id, 1, None, "youtube", "yt-abc")
        .await
        .expect("snapshot");
    let series = snap["series"].as_array().expect("series array");
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["metrics"]["views"], serde_json::json!(100));
    assert_eq!(series[0]["source"], serde_json::json!("api"));
}
