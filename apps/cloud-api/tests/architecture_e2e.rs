//! GET /api/v1/architecture — 结构化架构模型必须自洽: 边引用的节点都存在,
//! 层/流非空。不需要数据库 (端点为纯静态描述)。

use serde_json::Value;
use std::net::SocketAddr;
use uuid::Uuid;

#[tokio::test]
async fn architecture_model_is_well_formed() {
    cloud_api::auth::init_auth(b"arch-test-secret", 60);
    let issuer = sincy_security::JwtIssuer::new(b"arch-test-secret", 60);
    let token = issuer
        .issue(
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
            vec!["owner".into()],
        )
        .unwrap();

    let state = cloud_api::state_for_test::test_state(None);
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

    // unauthenticated is rejected
    let unauth = client
        .get(format!("{base}/architecture"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    let doc: Value = client
        .get(format!("{base}/architecture"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let layers = doc["layers"].as_array().unwrap();
    assert!(layers.len() >= 5, "expected at least 5 layers");
    let node_ids: std::collections::HashSet<&str> = layers
        .iter()
        .flat_map(|l| l["nodes"].as_array().unwrap().iter())
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(node_ids.len() >= 10);
    // every node has title/tech/desc
    for l in layers {
        for n in l["nodes"].as_array().unwrap() {
            assert!(n["title"].is_string() && n["tech"].is_string() && n["desc"].is_string());
        }
    }
    // every edge references known nodes and a declared kind
    let kinds: std::collections::HashSet<&str> = doc["edge_kinds"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    let edges = doc["edges"].as_array().unwrap();
    assert!(edges.len() >= 15);
    for e in edges {
        assert!(
            node_ids.contains(e["from"].as_str().unwrap()),
            "unknown from {}",
            e["from"]
        );
        assert!(
            node_ids.contains(e["to"].as_str().unwrap()),
            "unknown to {}",
            e["to"]
        );
        assert!(
            kinds.contains(e["kind"].as_str().unwrap()),
            "unknown kind {}",
            e["kind"]
        );
        assert!(e["label"].is_string());
    }
    // key flows documented
    let flows = doc["flows"].as_array().unwrap();
    assert!(flows.len() >= 5);
    let ids: Vec<&str> = flows.iter().map(|f| f["id"].as_str().unwrap()).collect();
    for f in ["F1", "F2", "F3", "F4", "F5"] {
        assert!(ids.contains(&f), "missing flow {f}");
    }
}
