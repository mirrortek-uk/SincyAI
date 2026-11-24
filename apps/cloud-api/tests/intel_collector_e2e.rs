//! Connector 采集 e2e (WI §9 / Sincy-05): 老 intel_collector 定时循环已退役,
//! 现役唯一抓取方是 manifest 驱动的 HttpConnector。本测试用本地 HTTP 服务
//! 扮演一个已注册的公开 RSS 信源, 验证 run_connector_instance 能把记录
//! 归一化为 RawRecord 入管线、镜像 legacy intel_items、登记 collection job,
//! 且重复轮询幂等 (dedup_key)。无 SINCY_TEST_DATABASE_URL 时跳过。

use axum::{Router, routing::get};
use serde_json::json;
use sincy_intel::pipeline::connector::AuthDecl;
use sincy_intel::pipeline::{
    ConnectorDescriptor, ConnectorManifest, DatasetDecl, RecordType, SourceRef,
};
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
    sqlx::query("TRUNCATE intel_opportunities, intel_items, intel_collection_jobs, intel_providers, intelligence_sources, intelligence_raw_records, connector_cursors, trend_series, ranking_snapshots RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    // register the source the connector will use
    sqlx::query("INSERT INTO intelligence_sources (id, name, category, access_method, capabilities) VALUES ('rsshub','RSSHub','rss','ExternalService','[\"rss_research\"]') ON CONFLICT (id) DO NOTHING")
        .execute(&pool).await.unwrap();
    Some(pool)
}

fn fixture_manifest(url: String) -> ConnectorManifest {
    ConnectorManifest {
        descriptor: ConnectorDescriptor {
            id: "fixture-rss".into(),
            name: "Fixture RSS".into(),
            connector_type: "rss".into(),
            capabilities: vec!["article_discovery".into()],
            datasets: vec![DatasetDecl {
                id: "fixture-rss".into(),
                record_type: RecordType::Article,
                url: Some(url.clone()),
            }],
            default_interval_secs: 60,
            rate_limit_per_min: 30,
            auth: AuthDecl::default(),
        },
        sources: vec![SourceRef {
            source_id: "rsshub".into(),
            label: "fixture".into(),
            url,
            kind: "news".into(),
            record_type: RecordType::Article,
            enabled: true,
        }],
        fetch_impl: "http:rss".into(),
    }
}

#[tokio::test]
async fn connector_fetches_ingests_mirrors_and_marks_jobs() {
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

    let manifest = fixture_manifest(format!("http://127.0.0.1:{port}/feed"));
    let conn = cloud_api::pipeline::connectors::HttpConnector::new(manifest);

    // first run: 3 fetched, 3 processed into the pipeline
    let (fetched, processed) =
        cloud_api::pipeline::connectors::run_connector_instance(&pool, &conn)
            .await
            .expect("connector run");
    assert_eq!(fetched, 3);
    assert_eq!(processed, 3);

    // RawRecord 入管线 (统一信封 + record_type)
    let raw = sqlx::query(
        "SELECT record_type, source_id, dedup_key, payload FROM intelligence_raw_records ORDER BY fetched_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(raw.len(), 3);
    for r in &raw {
        assert_eq!(r.get::<String, _>("record_type"), "article");
        assert_eq!(r.get::<String, _>("source_id"), "rsshub");
        let p: serde_json::Value = r.get("payload");
        assert!(p["title"].is_string() && p["url"].is_string() && p["summary"].is_string());
        assert!(r.get::<String, _>("dedup_key").starts_with("rsshub:news:"));
    }

    // legacy 镜像: intel_items 仍有数据 (机会引擎 / 趋势接口 / 健康审计不受影响)
    let items =
        sqlx::query("SELECT kind, dedup_key, payload FROM intel_items ORDER BY collected_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(items.len(), 3);
    let titles: Vec<String> = items
        .iter()
        .map(|r| {
            r.get::<serde_json::Value, _>("payload")["title"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(titles.contains(&"AI agents hit production".to_string()));

    // second run: dedup_key 幂等, 不产生新记录
    let (fetched2, processed2) =
        cloud_api::pipeline::connectors::run_connector_instance(&pool, &conn)
            .await
            .expect("connector rerun");
    assert_eq!(fetched2, 3);
    assert_eq!(processed2, 0);
    let raw_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM intelligence_raw_records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(raw_count, 3);
    let item_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM intel_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(item_count, 3);

    // 健康审计: job 以 schedule='connector' 登记 (老 feed:* 调度已退役)
    let job = sqlx::query(
        "SELECT status, last_run_at, last_fetched, last_new FROM intel_collection_jobs WHERE source_id = 'rsshub' AND schedule = 'connector'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(job.get::<String, _>("status"), "idle");
    assert!(
        job.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_run_at")
            .is_some()
    );
    assert_eq!(job.get::<Option<i32>, _>("last_fetched"), Some(3));

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
