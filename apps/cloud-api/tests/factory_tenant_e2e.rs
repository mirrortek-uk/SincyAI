//! factory 租户隔离 e2e: 两个 agency 互相看不到对方的 Brief, 跨租户 brand 被拒。

use std::net::SocketAddr;

use sqlx::PgPool;
use uuid::Uuid;

async fn setup() -> Option<(PgPool, Uuid, Uuid, Uuid, Uuid)> {
    let url = std::env::var("SINCY_TEST_DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.ok()?;
    for t in [
        "factory_briefs",
        "factory_learnings",
        "brands",
        "clients",
        "members",
        "agencies",
    ] {
        sqlx::query(&format!("TRUNCATE {t} CASCADE"))
            .execute(&pool)
            .await
            .ok()?;
    }
    let a1 = Uuid::new_v4();
    let a2 = Uuid::new_v4();
    for (a, name) in [(a1, "agency-A"), (a2, "agency-B")] {
        sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, $2)")
            .bind(a)
            .bind(name)
            .execute(&pool)
            .await
            .ok()?;
    }
    let mut brands = Vec::new();
    for a in [a1, a2] {
        let c = Uuid::new_v4();
        sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, 'c')")
            .bind(c)
            .bind(a)
            .execute(&pool)
            .await
            .ok()?;
        let b = Uuid::new_v4();
        sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, 'b')")
            .bind(b)
            .bind(a)
            .bind(c)
            .execute(&pool)
            .await
            .ok()?;
        brands.push(b);
    }
    Some((pool, a1, a2, brands[0], brands[1]))
}

#[tokio::test]
async fn factory_briefs_are_tenant_isolated() {
    let Some((pool, a1, a2, brand_a, _brand_b)) = setup().await else {
        eprintln!("skip: no SINCY_TEST_DATABASE_URL");
        return;
    };
    cloud_api::auth::init_auth(b"factory-tenant-secret", 60);
    let issuer = sincy_security::JwtIssuer::new(b"factory-tenant-secret", 60);
    let tok_a = issuer
        .issue(
            &Uuid::new_v4().to_string(),
            &a1.to_string(),
            vec!["owner".into()],
        )
        .unwrap();
    let tok_b = issuer
        .issue(
            &Uuid::new_v4().to_string(),
            &a2.to_string(),
            vec!["owner".into()],
        )
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
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}/api/v1");

    // agency A 用自己的 brand 建 Brief → 成功
    let r = client
        .post(format!("{base}/factory/briefs"))
        .bearer_auth(&tok_a)
        .json(&serde_json::json!({
            "trigger": "manual", "brand_id": brand_a.to_string(),
            "topic": "t", "goal": "g", "platforms": ["x"], "angle": "a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "agency A create with own brand should succeed"
    );

    // agency B 用 A 的 brand 建 Brief → 403
    let r = client
        .post(format!("{base}/factory/briefs"))
        .bearer_auth(&tok_b)
        .json(&serde_json::json!({
            "trigger": "manual", "brand_id": brand_a.to_string(),
            "topic": "t", "goal": "g", "platforms": ["x"], "angle": "a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "cross-tenant brand must be rejected");

    // agency B 列表看不到 A 的 Brief
    let r = client
        .get(format!("{base}/factory/briefs"))
        .bearer_auth(&tok_b)
        .send()
        .await
        .unwrap();
    let doc: serde_json::Value = r.json().await.unwrap();
    assert_eq!(doc["total"], 0, "agency B must not see agency A briefs");

    // agency A 能看到自己的
    let r = client
        .get(format!("{base}/factory/briefs"))
        .bearer_auth(&tok_a)
        .send()
        .await
        .unwrap();
    let doc: serde_json::Value = r.json().await.unwrap();
    assert_eq!(doc["total"], 1, "agency A sees its own brief");
    assert_eq!(doc["briefs"][0]["brand_id"], brand_a.to_string());

    // agency B 不能 run A 的 brief (用 A 的 brief_id)
    let brief_id = doc["briefs"][0]["brief_id"].as_str().unwrap().to_string();
    let r = client
        .post(format!("{base}/factory/briefs/{brief_id}/run"))
        .bearer_auth(&tok_b)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "cross-tenant run must 404");
}
