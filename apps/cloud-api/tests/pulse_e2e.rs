//! Sincy Pulse e2e (真实 PostgreSQL, 无 SINCY_TEST_DATABASE_URL 时跳过):
//! manifest 注册 → legal 红线(未知法务状态不允许启用) → 改合规后启用 →
//! 健康接口 → 实体/观测查询为空态。采集执行不走网络, 单独验证。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";
static E2E_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn setup() -> Option<(sqlx::PgPool, cloud_api::state_for_test::AppState, String)> {
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
    sqlx::query("TRUNCATE pulse_observations, pulse_entities, pulse_provider_runs, pulse_providers, agency_members, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    let agency_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'pulse-e2e')")
        .bind(agency_id)
        .execute(&pool)
        .await
        .expect("agency");
    let member_id = Uuid::new_v4();
    let hash = sincy_security::password::hash_password("test-password-123").expect("hash");
    sqlx::query(
        "INSERT INTO agency_members (id, agency_id, email, role, password_hash) VALUES ($1, $2, 'pulse@test.local', 'owner', $3)",
    )
    .bind(member_id).bind(agency_id).bind(&hash)
    .execute(&pool).await.expect("member");
    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    let token = state
        .jwt()
        .issue(
            &member_id.to_string(),
            &agency_id.to_string(),
            vec!["owner".into()],
        )
        .expect("token");
    Some((pool, state, token))
}

async fn call(
    state: &cloud_api::state_for_test::AppState,
    token: &str,
    req: Request<Body>,
) -> (StatusCode, Value) {
    let mut req = req;
    req.headers_mut().insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    let app = cloud_api::routes_for_test().with_state(state.clone());
    let res = app.oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn pulse_provider_legal_redline_and_health() {
    let _g = E2E_LOCK.lock().await;
    let Some((_pool, state, token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };

    // 1. 注册一个 legal_status=unknown 的 manifest → 自动不启用
    let manifest = json!({
        "id": "xbangdan", "name": "Xbangdan", "platform": "x",
        "capabilities": ["account_ranking", "viral_content"],
        "acquisition": {"type": "html", "strategy": "html_parser"},
        "datasets": [{"id": "account_ranking", "update_frequency_secs": 21600, "entity_type": "social_account"}],
        "reliability": 0.75
        // legal_status 缺省 = unknown
    });
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .method("POST")
            .uri("/pulse/providers")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&manifest).unwrap()))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], json!(false), "unknown legal 不能启用: {v}");
    assert_eq!(v["legal_status"], "unknown");

    // 2. 红线: 直接 toggle enabled=true 必须被拒
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .method("POST")
            .uri("/pulse/providers/xbangdan/toggle")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled": true}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"]["code"], "legal_blocked");

    // 3. 改法务状态为 commercial_ok 后允许启用
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .method("PUT")
            .uri("/pulse/providers/xbangdan")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"legal_status": "commercial_ok", "enabled": true}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["enabled"], json!(true));

    // 4. 列表可见 + schedulable 标记
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .uri("/pulse/providers")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let p = &v["items"][0];
    assert_eq!(p["id"], "xbangdan");
    assert_eq!(p["schedulable"], json!(true));
    assert_eq!(p["manifest"]["capabilities"][0], "account_ranking");

    // 5. health 接口: 从未跑过 → never
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .uri("/pulse/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let p = v["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["provider_id"] == "xbangdan")
        .unwrap();
    assert_eq!(p["status"], "never");

    // 6. 空态 entities/observations 查询正常
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .uri("/pulse/entities?type=social_account")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn pulse_bad_manifest_rejected() {
    let _g = E2E_LOCK.lock().await;
    let Some((_pool, state, token)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (st, v) = call(
        &state,
        &token,
        Request::builder()
            .method("POST")
            .uri("/pulse/providers")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"id": 123}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "bad_manifest");
}
