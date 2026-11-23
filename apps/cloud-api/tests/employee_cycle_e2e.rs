//! Digital Employee + World Intelligence e2e (DE guide §18/§5.2, WI §21):
//! agency → client → brand → employee → start → cycle (parked at approval)
//! → approve → publish → device task → memory/insights; plus intel
//! ingest/dedup → route → opportunity refresh.
//! Skips without SINCY_TEST_DATABASE_URL.

use serde_json::json;
use sha2::Digest;
use sincy_common::AgencyId;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use uuid::Uuid;

const TEST_URL_ENV: &str = "SINCY_TEST_DATABASE_URL";

async fn setup() -> Option<(sqlx::PgPool, u16, String, Uuid)> {
    let url = std::env::var(TEST_URL_ENV).ok()?;
    unsafe { std::env::set_var("SINCY_JWT_SECRET", "emp-test-secret") };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    sqlx::query("TRUNCATE task_events, execution_routes, device_tasks, kill_switches, quota_ledger, quota_pools, platform_apps, channel_capabilities, publishing_packages, employee_insights, memory_items, cycle_tasks, operating_cycles, employee_strategies, employee_goals, employee_missions, employee_identity_versions, digital_employees, brands, clients, agency_members, agencies, intel_opportunities, intel_items, intel_collection_jobs, intel_providers, intelligence_sources RESTART IDENTITY CASCADE")
        .execute(&pool).await.expect("clean");

    let agency = AgencyId::new();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, 'emp')")
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let hash = sincy_security::password::hash_password("EmpTest2026").unwrap();
    sqlx::query("INSERT INTO agency_members (id, agency_id, email, password_hash, role) VALUES ($1, $2, 'emp@sincy.dev', $3, 'owner')")
        .bind(Uuid::new_v4()).bind(agency.as_uuid()).bind(hash).execute(&pool).await.unwrap();
    let client_id = Uuid::new_v4();
    sqlx::query("INSERT INTO clients (id, agency_id, name) VALUES ($1, $2, 'c')")
        .bind(client_id)
        .bind(agency.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let brand_id = Uuid::new_v4();
    sqlx::query("INSERT INTO brands (id, agency_id, client_id, name) VALUES ($1, $2, $3, 'b')")
        .bind(brand_id)
        .bind(agency.as_uuid())
        .bind(client_id)
        .execute(&pool)
        .await
        .unwrap();
    // x routes: device preferred when a verified device exists
    sqlx::query("INSERT INTO channel_capabilities (platform, action, route, status, priority, constraints) VALUES ('x','publish_post','device','verified',10,'{\"adapter\":\"x.publish_post\"}'), ('x','publish_post','assisted','verified',90,'{}')")
        .execute(&pool).await.unwrap();
    // adapter definition for the device route is seeded by migration 0015
    // an active device
    let mgr = sincy_device::DeviceManager::new(&pool, b"emp-test-secret");
    let ch = mgr
        .start_registration(agency, "android", Some("t"))
        .await
        .unwrap();
    let secret_hash = format!("{:x}", sha2::Sha256::digest(ch.device_secret.as_bytes()));
    let proof = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{secret_hash}{}", ch.nonce).as_bytes())
    );
    mgr.complete_registration(ch.device_id, &ch.nonce, &proof)
        .await
        .unwrap();
    // 设备路由按心跳判在线: 注册后立刻心跳
    sqlx::query("UPDATE devices SET last_seen = now() WHERE id = $1")
        .bind(ch.device_id)
        .execute(&pool)
        .await
        .unwrap();
    // intel seed (normally from migration 0017, truncated above)
    sqlx::query("INSERT INTO intelligence_sources (id, name, category, access_method, capabilities) VALUES ('trendradar','TrendRadar','trend','ExternalService','[\"trend_search\"]'), ('reach-mcp','reach-mcp','social','MCPServer','[\"social_search\",\"x_research\"]') ON CONFLICT (id) DO NOTHING")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO intel_providers (source_id, name, health, reliability) VALUES ('reach-mcp','reach-mcp','healthy',0.9)")
        .execute(&pool).await.unwrap();

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

    let login = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/v1/auth/login"))
        .json(&json!({"email": "emp@sincy.dev", "password": "EmpTest2026"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    Some((
        pool,
        port,
        login["token"].as_str().unwrap().to_string(),
        brand_id,
    ))
}

#[tokio::test]
async fn employee_and_intel_e2e() {
    let Some((pool, port, token, brand_id)) = setup().await else {
        eprintln!("skip: {TEST_URL_ENV} not set");
        return;
    };
    intel_phase(&port, &token).await;
    employee_phase(&pool, &port, &token, &brand_id).await;
}

async fn intel_phase(port: &u16, token: &str) {
    let client = reqwest::Client::new();
    let auth = |r: reqwest::RequestBuilder| r.bearer_auth(token);
    let base = format!("http://127.0.0.1:{port}/api/v1");

    // ingest a trend + dedup on second post
    let body = json!({
        "source_id": "trendradar", "kind": "trend", "dedup_key": "trend:t1",
        "payload": {"topic":"ai agents","platforms":["x"],"volume":9000,"velocity":4.0,"sentiment":0.7,"window_hours":24}
    });
    let r1 = auth(client.post(format!("{base}/intel/ingest")).json(&body))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(r1["duplicate"], false);
    let r2 = auth(client.post(format!("{base}/intel/ingest")).json(&body))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(r2["duplicate"], true);

    // unknown source rejected
    let bad = auth(client.post(format!("{base}/intel/ingest")).json(&json!({
        "source_id": "nope", "kind": "trend", "dedup_key": "x", "payload": {}
    })))
    .send()
    .await
    .unwrap();
    assert_eq!(bad.status(), 400);

    // route social_search -> reach-mcp (healthy)
    let route = auth(client.post(format!("{base}/intel/route")).json(&json!({
        "capability": "social_search", "platform": "x"
    })))
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(route["providers"][0]["name"], "reach-mcp");

    // opportunity refresh scores the trend
    let refreshed = auth(client.post(format!("{base}/intel/opportunities")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(refreshed["opportunities"], 1);
    let opps = auth(client.get(format!("{base}/intel/opportunities")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(opps["items"][0]["topic"], "ai agents");
    assert!(
        opps["items"][0]["score"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            > 0.3
    );
}

async fn employee_phase(pool: &sqlx::PgPool, port: &u16, token: &str, brand_id: &Uuid) {
    let brand_id = *brand_id;
    let client = reqwest::Client::new();
    let auth = |r: reqwest::RequestBuilder| r.bearer_auth(token);
    let base = format!("http://127.0.0.1:{port}/api/v1");

    // seed an opportunity so the cycle has something to pick
    auth(client.post(format!("{base}/intel/ingest")).json(&json!({
        "source_id": "trendradar", "kind": "trend", "dedup_key": "trend:ai-agents",
        "payload": {"topic":"ai agents","platforms":["x"],"volume":9000,"velocity":4.0,"sentiment":0.7,"window_hours":24}
    }))).send().await.unwrap();
    auth(client.post(format!("{base}/intel/opportunities")))
        .send()
        .await
        .unwrap();

    // create employee (auto_publish=false → approval gate)
    let emp = auth(client.post(format!("{base}/employees")).json(&json!({
        "name": "Aria", "role": "content_operator", "brand_id": brand_id, "channel": "x",
        "auto_publish": false,
        "identity": {"name":"Aria","role":"content_operator","voice":"sharp","audience":["developers"],"expertise":["ai agents"],"values":[],"boundaries":[]},
        "mission": {"statement":"grow authority","domain":"content","audience":["developers"],"value_proposition":"practical","constraints":[],"success_definition":"engagement"},
        "strategy": {"pillars":["ai agents"],"cadence_per_week":7,"formats":["text"],"risk_tolerance":0.3,"exploration_budget":0.2}
    }))).send().await.unwrap().json::<serde_json::Value>().await.unwrap();
    let emp_id = emp["id"].as_str().unwrap().to_string();
    assert_eq!(emp["status"], "READY");

    // language: brand default zh-CN flows into the employee identity
    let det = auth(client.get(format!("{base}/employees/{emp_id}")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(det["language"], "zh-CN");
    // PATCH employee language (versioned identity) + invalid language rejected
    let patched = auth(
        client
            .patch(format!("{base}/employees/{emp_id}"))
            .json(&json!({"language":"en-US"})),
    )
    .send()
    .await
    .unwrap();
    assert!(patched.status().is_success());
    let det2 = auth(client.get(format!("{base}/employees/{emp_id}")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(det2["language"], "en-US");
    let bad_lang = auth(
        client
            .patch(format!("{base}/employees/{emp_id}"))
            .json(&json!({"language":"xx-XX"})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(bad_lang.status(), 400);
    // brand-level language via PATCH /brands/{id}
    let bp = auth(
        client
            .patch(format!("{base}/brands/{brand_id}"))
            .json(&json!({"language":"ja-JP"})),
    )
    .send()
    .await
    .unwrap();
    assert!(bp.status().is_success());
    let bl = auth(client.get(format!("{base}/brands")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(
        bl["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["language"] == "ja-JP")
    );
    // back to zh-CN for the cycle assertions below
    auth(
        client
            .patch(format!("{base}/employees/{emp_id}"))
            .json(&json!({"language":"zh-CN"})),
    )
    .send()
    .await
    .unwrap();

    // skills: catalog seeded by migration 0021 (9 domains / 77), auto-bound on create
    let doms = auth(client.get(format!("{base}/skills/domains")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(doms["domains"].as_array().unwrap().len(), 9);
    assert_eq!(doms["total_skills"].as_i64().unwrap(), 77);
    let bound = auth(client.get(format!("{base}/employees/{emp_id}/skills")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(bound["items"].as_array().unwrap().len(), 77);
    // register a new version of an existing skill -> version bumps (old archived in skill_versions)
    // skills 表是平台级种子, 不在 TRUNCATE 列表里, 所以按相对版本断言
    let before = auth(client.get(format!("{base}/skills?q=social_post_generation")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let v_before = before["items"][0]["version"].as_i64().unwrap();
    let v2 = auth(client.post(format!("{base}/skills")).json(&json!({
        "domain": "content", "name": "social_post_generation", "description": "v2 contract"
    })))
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(v2["version"].as_i64().unwrap(), v_before + 1);
    // disable one binding
    let sid = bound["items"][0]["id"].as_str().unwrap().to_string();
    auth(
        client
            .post(format!("{base}/employees/{emp_id}/skills"))
            .json(&json!({"skill_id": sid, "enabled": false})),
    )
    .send()
    .await
    .unwrap();

    // illegal transition: stop from READY is rejected (409-ish non_retryable)
    let bad = auth(client.post(format!("{base}/employees/{emp_id}/stop")))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // start
    let started = auth(client.post(format!("{base}/employees/{emp_id}/start")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(started["status"], "RUNNING");

    // cycle before start would 4xx — already running; run a cycle
    let cycle = auth(
        client
            .post(format!("{base}/employees/{emp_id}/cycles"))
            .json(&json!({"trigger":"test"})),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(cycle["state"], "SUCCEEDED", "cycle response: {cycle}");
    assert_eq!(cycle["selected_opportunity"], "ai agents");
    let pkg = cycle["package_id"].as_str().unwrap().to_string();
    // auto_publish=false → no task, parked for approval
    assert!(cycle["task_id"].is_null());

    // package is pending
    let pkgs = auth(client.get(format!("{base}/packages?platform=x")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let pkg_row = pkgs["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"].as_str().unwrap() == pkg)
        .unwrap();
    assert_eq!(pkg_row["approval_status"], "pending");
    assert_eq!(pkg_row["constraints"]["source"], "ai");
    assert_eq!(pkg_row["constraints"]["provenance"]["employee_id"], emp_id);

    // cycle detail shows all steps
    let cycles = auth(client.get(format!("{base}/employees/{emp_id}/cycles")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let cyc_id = cycles["items"][0]["id"].as_str().unwrap().to_string();
    let detail = auth(client.get(format!("{base}/cycles/{cyc_id}")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let steps: Vec<&str> = detail["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["step"].as_str().unwrap())
        .collect();
    for s in [
        "observe",
        "understand",
        "decide",
        "plan",
        "create",
        "review",
        "execute",
        "learn",
        "adapt",
    ] {
        assert!(steps.contains(&s), "missing step {s}");
    }

    // memory + insight written
    let mem = auth(client.get(format!("{base}/employees/{emp_id}/memory")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(!mem["items"].as_array().unwrap().is_empty());
    let ins = auth(client.get(format!("{base}/employees/{emp_id}/insights")))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(!ins["items"].as_array().unwrap().is_empty());

    // approve + publish → device route (verified device exists)
    auth(client.post(format!("{base}/packages/{pkg}/approve")))
        .send()
        .await
        .unwrap();
    let pub_res = auth(client.post(format!("{base}/tasks/publish")).json(&json!({
        "brand_id": brand_id, "platform": "x", "action": "publish_post",
        "package_id": pkg, "package_version": 1, "idempotency_key": format!("emp-{emp_id}")
    })))
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(pub_res["route"], "device");
    let task_id = pub_res["task_id"].as_str().unwrap().to_string();
    let row = sqlx::query("SELECT state FROM device_tasks WHERE id = $1")
        .bind(Uuid::parse_str(&task_id).unwrap())
        .fetch_one(pool)
        .await
        .unwrap();
    use sqlx::Row;
    assert_eq!(row.get::<String, _>(0), "QUEUED");

    // auto_publish=true path: patch and run another cycle → task created inline
    auth(
        client
            .patch(format!("{base}/employees/{emp_id}"))
            .json(&json!({"auto_publish": true})),
    )
    .send()
    .await
    .unwrap();
    // second trend so the auto_publish cycle has a fresh opportunity to pick
    auth(client.post(format!("{base}/intel/ingest")).json(&json!({
        "source_id": "trendradar", "kind": "trend", "dedup_key": "trend:ai-agents-evals",
        "payload": {"topic":"ai agents evals","platforms":["x"],"volume":8000,"velocity":3.5,"sentiment":0.65,"window_hours":24}
    }))).send().await.unwrap();
    auth(client.post(format!("{base}/intel/opportunities")))
        .send()
        .await
        .unwrap();
    let cycle2 = auth(
        client
            .post(format!("{base}/employees/{emp_id}/cycles"))
            .json(&json!({"trigger":"test-2"})),
    )
    .send()
    .await
    .unwrap()
    .json::<serde_json::Value>()
    .await
    .unwrap();
    assert_eq!(cycle2["state"], "SUCCEEDED");
    assert!(
        !cycle2["task_id"].is_null(),
        "auto_publish should create a task"
    );
    assert_eq!(cycle2["task_state"], "QUEUED");

    // pause/resume/stop lifecycle
    assert_eq!(
        auth(client.post(format!("{base}/employees/{emp_id}/pause")))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["status"],
        "PAUSED"
    );
    assert_eq!(
        auth(client.post(format!("{base}/employees/{emp_id}/resume")))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["status"],
        "RUNNING"
    );
    assert_eq!(
        auth(client.post(format!("{base}/employees/{emp_id}/stop")))
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["status"],
        "STOPPED"
    );
}
