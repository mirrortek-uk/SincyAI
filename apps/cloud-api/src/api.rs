//! /api/v1 route surface (TDD 12.2). WP-01 ships health + version; resource
//! routers (devices, accounts, capabilities, routing, quota, adapters, tasks,
//! transfers, publishing, evidence, kill-switches) land with their WPs.

use axum::{
    Json, Router,
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use sincy_common::{AppError, ErrorKind, RequestId};

pub fn routes() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/agent/connect", get(agent_connect_placeholder))
        .layer(axum::middleware::from_fn(request_id_middleware))
}

async fn healthz() -> impl IntoResponse {
    Json(json!({ "status": "ok", "ts": chrono::Utc::now().to_rfc3339() }))
}

async fn version() -> impl IntoResponse {
    Json(json!({
        "service": "cloud-api",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_schema": sincy_protocol::PROTOCOL_SCHEMA_VERSION,
    }))
}

/// WP-08 replaces this with the real WSS gateway (device auth, envelope
/// validation, heartbeat). Placeholder returns 501 with unified error body.
async fn agent_connect_placeholder() -> Response {
    app_error_response(
        AppError::new(
            ErrorKind::NonRetryable,
            "not_implemented",
            "agent WSS gateway lands in WP-08",
        )
        .with_request_id(RequestId::new().to_string()),
    )
}

pub async fn not_found() -> Response {
    app_error_response(
        AppError::non_retryable("not_found", "no such route")
            .with_request_id(RequestId::new().to_string()),
    )
}

/// Middleware: attach a fresh request_id header + log line for every request.
pub async fn request_id_middleware(req: Request, next: axum::middleware::Next) -> Response {
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| RequestId::new().to_string());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    tracing::info_span!("http", %rid, %method, %path);
    let mut res = next.run(req).await;
    if let Ok(v) = axum::http::HeaderValue::from_str(&rid) {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}

/// Convert an AppError into the unified JSON error response (TDD 12.1).
pub fn app_error_response(err: AppError) -> Response {
    let status =
        StatusCode::from_u16(err.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut body = err.to_body();
    if body.error.request_id.is_empty() {
        body.error.request_id = RequestId::new().to_string();
    }
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    #[tokio::test]
    async fn healthz_ok() {
        let app = routes();
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_uses_unified_error() {
        let res = not_found().await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn agent_placeholder_is_501_shape() {
        let res = agent_connect_placeholder().await;
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "not_implemented");
        assert_eq!(status.as_u16(), 400); // NonRetryable maps to 400
    }
}
