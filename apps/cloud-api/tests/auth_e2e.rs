//! 正式产品化认证验收: 注册(建机构+owner) -> 登录 -> 受保护路由校验。
//! 覆盖: 无令牌 401、错误密码 bad_credentials、租户不匹配拒绝、预算熔断。
//! 跑过真实 PostgreSQL; 无 SINCY_TEST_DATABASE_URL 时跳过。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

/// 每个测试会 TRUNCATE 共享测试库, 必须串行执行
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn setup() -> Option<(sqlx::PgPool, cloud_api::state_for_test::AppState)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "auth-e2e-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, device_app_versions, device_capabilities, device_sessions, devices, automation_consents, social_accounts, account_device_bindings, quota_ledger, model_cost_records, platform_metrics, content_performance_snapshots, publishing_packages, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");
    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    Some((pool, state))
}

async fn call(
    state: &cloud_api::state_for_test::AppState,
    req: Request<Body>,
) -> (StatusCode, Value) {
    let app = cloud_api::routes_for_test().with_state(state.clone());
    let res = app.oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

fn post_json(uri: &str, token: Option<&str>, body: Value) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn register_login_and_protected_routes() {
    let _guard = TEST_LOCK.lock().await;
    let Some((_pool, state)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };

    // 1. 注册: POST /agencies (公开) 创建机构 + owner, 直接返回 token
    let (st, v) = call(
        &state,
        post_json(
            "/agencies",
            None,
            json!({"name": "正式机构", "owner_email": "owner@sincy.dev", "owner_password": "correct-horse-123"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let _token = v["token"].as_str().expect("token on register").to_string();
    let agency_id = v["id"].as_str().unwrap().to_string();

    // 2. 弱密码被拒
    let (st, v) = call(
        &state,
        post_json(
            "/agencies",
            None,
            json!({"name": "x", "owner_email": "x@x.dev", "owner_password": "short"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "weak_password");

    // 3. 错误密码登录被拒
    let (st, v) = call(
        &state,
        post_json(
            "/auth/login",
            None,
            json!({"email": "owner@sincy.dev", "password": "wrong-password"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "bad_credentials");

    // 4. 正确密码登录换新 token
    let (st, v) = call(
        &state,
        post_json(
            "/auth/login",
            None,
            json!({"email": "owner@sincy.dev", "password": "correct-horse-123"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let token2 = v["token"].as_str().unwrap().to_string();
    assert_eq!(v["role"], "owner");

    // 5. 无令牌访问受保护路由 -> 401
    let (st, v) = call(
        &state,
        Request::builder()
            .uri("/agencies/me")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"]["code"], "unauthorized");

    // 6. 带令牌访问 /agencies/me 正常, 租户来自 JWT
    let (st, v) = call(
        &state,
        Request::builder()
            .uri("/agencies/me")
            .header("authorization", format!("Bearer {token2}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["id"].as_str().unwrap(), agency_id);
    assert_eq!(v["name"], "正式机构");

    // 7. 任务列表为空但可达
    let (st, v) = call(
        &state,
        Request::builder()
            .uri("/tasks")
            .header("authorization", format!("Bearer {token2}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["items"].as_array().unwrap().len(), 0);

    // 8. 伪造/篡改令牌被拒
    let (st, _v) = call(
        &state,
        Request::builder()
            .uri("/agencies/me")
            .header("authorization", "Bearer not-a-jwt")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn publish_rejects_foreign_tenant_in_body() {
    let _guard = TEST_LOCK.lock().await;
    let Some((_pool, state)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    let (st, v) = call(
        &state,
        post_json(
            "/agencies",
            None,
            json!({"name": "a1", "owner_email": "a1@x.dev", "owner_password": "password-123"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let token = v["token"].as_str().unwrap().to_string();
    // body 里偷换别人 agency_id -> 拒绝
    let (st, v) = call(
        &state,
        post_json(
            "/tasks/publish",
            Some(&token),
            json!({
                "agency_id": Uuid::new_v4(),
                "brand_id": Uuid::new_v4(),
                "platform": "youtube", "action": "publish_video",
                "package_id": Uuid::new_v4(), "package_version": 1,
                "idempotency_key": "x-1",
            }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"]["code"], "tenant_mismatch");
}

#[tokio::test]
async fn ai_generate_respects_budget_and_mock_flag() {
    let _guard = TEST_LOCK.lock().await;
    let Some((_pool, state)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    unsafe { std::env::set_var("SINCY_MONTHLY_BUDGET_USD", "0.001") }; // 极低预算
    let (st, v) = call(
        &state,
        post_json(
            "/agencies",
            None,
            json!({"name": "a2", "owner_email": "a2@x.dev", "owner_password": "password-123"}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let token = v["token"].as_str().unwrap().to_string();
    let mut st2 = state.clone();
    st2.monthly_budget_usd = Some(0.001);
    let (st, v) = call(
        &st2,
        post_json(
            "/ai/generate",
            Some(&token),
            json!({"prompt": "写一条视频文案", "max_tokens": 2048}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "budget_exceeded");
    unsafe { std::env::remove_var("SINCY_MONTHLY_BUDGET_USD") };

    // mock provider 在测试态(allow_mock=true)可用并标记 mock
    let (st, v) = call(
        &state,
        post_json(
            "/ai/generate",
            Some(&token),
            json!({"prompt": "你好", "max_tokens": 64}),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["mock"], true);
    assert!(v["text"].as_str().unwrap().starts_with("[mock:"));
}
