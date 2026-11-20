//! Intel collector e2e (WI §9): a local HTTP server plays the role of a
//! registered public RSS source; collect_once must normalize + dedup into
//! intel_items and mark the collection job. Skips without
//! SINCY_TEST_DATABASE_URL.

use axum::{Router, routing::get};
use cloud_api::intel_collector::{CollectorConfig, FeedConfig, Parser, collect_once};
use serde_json::json;
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>fixture</title>
<item><title>AI agents hit production</title><link>https://ex.com/1</link><description>agents &amp; evals</description><pubDate>Wed, 20 Nov 2026 10:00:00 GMT</pubDate></item>
<item><title>LLM ops playbook</title><link>https://ex.com/2</link><description>playbook</description></item>
<item><title>Multi-agent orchestration</title><link>https://ex.com/3</link><description>orch</description></item>
</channel></rss>"#;

async fn setup() -> Option<sqlx::PgPool> {
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
    sqlx::query("TRUNCATE intel_opportunities, intel_items, intel_collection_jobs, intel_providers, intelligence_sources RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    // register the source the collector will use
    sqlx::query("INSERT INTO intelligence_sources (id, name, category, access_method, capabilities) VALUES ('rsshub','RSSHub','rss','ExternalService','[\"rss_research\"]') ON CONFLICT (id) DO NOTHING")
        .execute(&pool).await.unwrap();
    Some(pool)
}

#[tokio::test]
async fn collector_fetches_normalizes_dedups_and_marks_jobs() {
    let Some(pool) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    // local fixture server standing in for the public RSS source
    let app = Router::new().route("/feed", get(|| async { RSS }));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let cfg = CollectorConfig {
        interval_secs: 60,
        feeds: vec![
            FeedConfig {
                source_id: "rsshub".into(),
                label: "fixture".into(),
                url: format!("http://127.0.0.1:{port}/feed"),
                kind: "news".into(),
                parser: Parser::Rss,
                max_items: 20,
            },
            // unregistered source must fail gracefully, not crash the run
            FeedConfig {
                source_id: "nope".into(),
                label: "ghost".into(),
                url: format!("http://127.0.0.1:{port}/feed"),
                kind: "news".into(),
                parser: Parser::Rss,
                max_items: 5,
            },
        ],
    };

    // first run: 3 inserted, 1 failure (unregistered source)
    let (ins, dup, fails) = collect_once(&pool, &cfg).await;
    assert_eq!(ins, 3);
    assert_eq!(dup, 0);
    assert_eq!(fails.len(), 1);
    assert!(fails[0].contains("not registered"));

    let rows =
        sqlx::query("SELECT kind, dedup_key, payload FROM intel_items ORDER BY collected_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    for r in &rows {
        assert_eq!(r.get::<String, _>("kind"), "news");
        let p: serde_json::Value = r.get("payload");
        assert!(p["title"].is_string() && p["url"].is_string() && p["summary"].is_string());
        assert!(r.get::<String, _>("dedup_key").starts_with("rsshub:news:"));
    }
    let titles: Vec<String> = rows
        .iter()
        .map(|r| {
            r.get::<serde_json::Value, _>("payload")["title"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(titles.contains(&"AI agents hit production".to_string()));

    // second run: everything dedups
    let (ins2, dup2, _) = collect_once(&pool, &cfg).await;
    assert_eq!(ins2, 0);
    assert_eq!(dup2, 3);

    // job rows marked (fixture idle, ghost failed)
    let job = sqlx::query("SELECT status, last_run_at, last_error FROM intel_collection_jobs WHERE source_id = 'rsshub' AND schedule = 'feed:fixture'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(job.get::<String, _>("status"), "idle");
    assert!(
        job.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_run_at")
            .is_some()
    );
    // unregistered source is refused by the registry check AND by the FK on
    // intel_collection_jobs, so no job row exists for it (failure is still
    // reported in the return value above)
    let ghost_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM intel_collection_jobs WHERE source_id = 'nope'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ghost_count, 0);

    // a trend item flows into the opportunity engine
    sqlx::query("INSERT INTO intel_items (source_id, kind, dedup_key, payload) VALUES ('rsshub','trend','t:x', $1)")
        .bind(json!({"topic":"ai agents","platforms":["github"],"volume":9000,"velocity":12.0,"sentiment":0.7,"window_hours":168}))
        .execute(&pool).await.unwrap();
    let (n, up) = cloud_api::intel::refresh_opportunities_db_for_test(&pool).await;
    assert!(n >= 1 && up >= 1);
    let opp = sqlx::query("SELECT topic, score FROM intel_opportunities WHERE topic = 'ai agents'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(opp.get::<rust_decimal::Decimal, _>("score") > rust_decimal::Decimal::new(30, 2));
}
