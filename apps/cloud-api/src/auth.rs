//! Console/API authentication (正式产品化): agency member login with
//! Argon2id password verification, short-lived JWT (TDD 15), and a middleware
//! that derives the tenant context from the token — agency_id is NEVER taken
//! from client input on protected routes (Guide 6).

use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sincy_common::{AgencyId, AppError};
use sincy_security::{JwtIssuer, password};
use sqlx::Row;
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

use crate::api::app_error_response;
use crate::state::AppState;

/// Authenticated caller, derived from the JWT only. Handlers take this via
/// axum's Extension extractor (middleware inserts it into request extensions).
#[derive(Debug, Clone)]
pub struct AuthCtx {
    pub member_id: Uuid,
    pub agency_id: AgencyId,
    pub roles: Vec<String>,
}

/// Process-global JWT verifier for the auth middleware (set once at startup).
static AUTH_JWT: OnceLock<Arc<JwtIssuer>> = OnceLock::new();

pub fn init_auth(secret: &[u8], ttl_minutes: i64) {
    let _ = AUTH_JWT.set(Arc::new(JwtIssuer::new(secret, ttl_minutes)));
}

pub fn auth_jwt() -> Option<&'static Arc<JwtIssuer>> {
    AUTH_JWT.get()
}

/// Paths that stay public: health/version, login, agency registration (POST
/// /agencies creates the tenant + owner), the device gateway, and the device
/// pairing completion (its proof-of-possession IS the credential).
/// /devices/register/start stays protected: it needs the owner's Bearer JWT.
fn is_public(path: &str, method: &axum::http::Method) -> bool {
    path.ends_with("/healthz")
        || path.ends_with("/version")
        || path.ends_with("/auth/login")
        || path.ends_with("/agent/connect")
        || path.ends_with("/metrics")  // Prometheus scrape, network-isolated
        || (method == axum::http::Method::POST && path.ends_with("/agencies"))
        || (method == axum::http::Method::POST && path.ends_with("/devices/register/complete"))
}

/// Verify `Authorization: Bearer <jwt>` and inject AuthCtx into extensions.
pub async fn auth_middleware(mut req: Request, next: Next) -> Response {
    let Some(jwt) = auth_jwt() else {
        return app_error_response(AppError::non_retryable(
            "auth_not_initialized",
            "server auth is not initialized",
        ));
    };
    let path = req.uri().path().to_string();
    if is_public(&path, req.method()) {
        return next.run(req).await;
    }
    let header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let token = header.strip_prefix("Bearer ").ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "unauthorized",
            "expected Authorization: Bearer <token>",
        ))
    });
    let token = match token {
        Ok(t) => t.to_string(),
        Err(res) => return res,
    };
    match jwt.verify(&token) {
        Ok(claims) => {
            let agency_id = match Uuid::parse_str(&claims.agency_id) {
                Ok(u) => AgencyId::from_uuid(u),
                Err(_) => {
                    return app_error_response(AppError::non_retryable(
                        "unauthorized",
                        "token has invalid agency claim",
                    ));
                }
            };
            req.extensions_mut().insert(AuthCtx {
                member_id: Uuid::parse_str(&claims.sub).unwrap_or_else(|_| Uuid::nil()),
                agency_id,
                roles: claims.roles,
            });
            next.run(req).await
        }
        Err(e) => {
            let _ = StatusCode::UNAUTHORIZED; // app_error_response maps code -> 401
            app_error_response(AppError::non_retryable(
                "unauthorized",
                format!("token rejected: {e}"),
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

/// POST /api/v1/auth/login — email + password -> short-lived JWT.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<Value>, Response> {
    let pool = state.pool().map_err(app_error_response)?;
    let row = sqlx::query(
        "SELECT id, agency_id, role, password_hash FROM agency_members WHERE email = $1",
    )
    .bind(&body.email)
    .fetch_optional(pool)
    .await
    .map_err(|e| app_error_response(AppError::retryable("db", e.to_string())))?
    .ok_or_else(|| {
        app_error_response(AppError::non_retryable(
            "bad_credentials",
            "invalid email or password",
        ))
    })?;
    let hash: Option<String> = row.get("password_hash");
    let ok = match hash {
        Some(h) => password::verify_password(&body.password, &h).map_err(|e| {
            app_error_response(AppError::non_retryable("auth_failed", e.to_string()))
        })?,
        None => false,
    };
    if !ok {
        return Err(app_error_response(AppError::non_retryable(
            "bad_credentials",
            "invalid email or password",
        )));
    }
    let member_id: Uuid = row.get("id");
    let agency_id: Uuid = row.get("agency_id");
    let role: String = row.get("role");
    let token = state
        .jwt()
        .issue(
            &member_id.to_string(),
            &agency_id.to_string(),
            vec![role.clone()],
        )
        .map_err(|e| {
            app_error_response(AppError::non_retryable("token_issue_failed", e.to_string()))
        })?;
    Ok(Json(json!({
        "token": token,
        "token_type": "Bearer",
        "agency_id": agency_id,
        "member_id": member_id,
        "role": role,
    })))
}
