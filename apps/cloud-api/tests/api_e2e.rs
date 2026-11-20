//! End-to-end API slice test against real PostgreSQL (skips without
//! SINCY_TEST_DATABASE_URL): agency -> capabilities -> publish (api route,
//! mock adapter) -> idempotent replay -> quota exhaustion -> kill switch.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::Row;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(
    sqlx::PgPool,
    cloud_api::state_for_test::AppState,
    Uuid,
    Uuid,
)> {
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
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, quota_ledger, quota_allocations, quota_pools, platform_apps, channel_capabilities, approvals, package_assets, publishing_packages, ai_labels, content_rights, assets, content_versions, content_items, brands, clients, agency_members, roles, agencies RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'e2e-api')")
        .bind(agency_id)
        .execute(&pool)
        .await
        .expect("agency");
    let client_id = Uuid::new_v4();
    sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, 'c1')")
        .bind(client_id)
        .bind(agency_id)
        .execute(&pool)
        .await
        .expect("client");
    let brand_id = Uuid::new_v4();
    sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, 'b1')")
        .bind(brand_id)
        .bind(agency_id)
        .bind(client_id)
        .execute(&pool)
        .await
        .expect("brand");

    sqlx::query(
        "INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints)
         VALUES ('youtube', 'publish_video', 'api', 'verified', 10, '{\"quota_cost\": 1600}'),
                ('youtube', 'publish_video', 'assisted', 'verified', 30, '{}')",
    )
    .execute(&pool)
    .await
    .expect("caps");
    let app_id: Uuid = sqlx::query(
        "INSERT INTO platform_apps (platform, client_id, client_secret_ref)
         VALUES ('youtube', 'mock', 'vault://kv/mock') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("app")
    .get("id");
    sqlx::query(
        "INSERT INTO quota_pools (platform_app_id, quota_kind, limit_value, used_value)
         VALUES ($1, 'videos.insert', 2000, 0)",
    )
    .bind(app_id)
    .execute(&pool)
    .await
    .expect("pool");

    let state = cloud_api::state_for_test::test_state(Some(pool.clone()));
    Some((pool, state, agency_id, brand_id))
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

#[tokio::test]
async fn publish_flow_e2e() {
    let Some((pool, state, agency_id, brand_id)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };

    // capabilities visible
    let (st, v) = call(
        &state,
        Request::builder()
            .uri("/capabilities?platform=youtube")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["route"] == "api" && c["status"] == "verified")
    );

    // publish via api route
    let idem = format!("idem-{}", Uuid::new_v4());
    let (st, v) = call(
        &state,
        Request::builder()
            .method("POST")
            .uri("/tasks/publish")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "agency_id": agency_id, "brand_id": brand_id,
                    "platform": "youtube", "action": "publish_video",
                    "package_id": Uuid::new_v4(), "package_version": 1,
                    "idempotency_key": idem,
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["route"], "api");
    assert_eq!(v["status"], "PUBLISHED");
    let ext = v["external_content_id"].as_str().unwrap().to_string();
    assert!(ext.starts_with("mock-yt-"));
    let task_id = v["task_id"].as_str().unwrap().to_string();

    // route audit
    let (st, v) = call(
        &state,
        Request::builder()
            .uri(format!("/tasks/{task_id}/route"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["selected_route"], "api");

    // idempotent replay: same key must NOT re-execute
    let (st, v) = call(
        &state,
        Request::builder()
            .method("POST")
            .uri("/tasks/publish")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "agency_id": agency_id, "brand_id": brand_id,
                    "platform": "youtube", "action": "publish_video",
                    "package_id": Uuid::new_v4(), "package_version": 1,
                    "idempotency_key": idem,
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["route"], "idempotent_replay");
    let count: i64 = sqlx::query("SELECT count(*) FROM execution_routes WHERE task_id = $1")
        .bind(Uuid::parse_str(&task_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap()
        .get::<i64, _>(0);
    assert_eq!(count, 1, "replay must not create a second route/execution");

    // quota exhaustion: pool has 2000, cost 1600 -> after one publish 400 left
    let (st, v) = call(
        &state,
        Request::builder()
            .method("POST")
            .uri("/tasks/publish")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "agency_id": agency_id, "brand_id": brand_id,
                    "platform": "youtube", "action": "publish_video",
                    "package_id": Uuid::new_v4(), "package_version": 1,
                    "idempotency_key": format!("idem-{}", Uuid::new_v4()),
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["status"], "QUOTA_WAIT", "{v}");
    assert_eq!(v["fallback"], "Degrade(Assisted)");

    // kill switch blocks new tasks (FR-025)
    let (st, _v) = call(&state, Request::builder()
        .method("POST").uri("/kill-switches")
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "agency_id": agency_id, "scope": "brand", "target_id": brand_id, "reason": "test",
        }).to_string())).unwrap()).await;
    assert_eq!(st, StatusCode::OK);
    let (st, v) = call(
        &state,
        Request::builder()
            .method("POST")
            .uri("/tasks/publish")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "agency_id": agency_id, "brand_id": brand_id,
                    "platform": "youtube", "action": "publish_video",
                    "package_id": Uuid::new_v4(), "package_version": 1,
                    "idempotency_key": format!("idem-{}", Uuid::new_v4()),
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"]["code"], "kill_switch_active");
}
