//! Shared application state: DB pool (optional in local-no-db mode), JWT
//! issuer for console sessions, and product safety flags from config.

use sincy_config::AppConfig;
use sincy_security::JwtIssuer;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: Option<PgPool>,
    pub env: String,
    jwt: Arc<JwtIssuer>,
    jwt_secret: Vec<u8>,
    /// Mock channel adapters are forbidden unless this is explicitly true
    /// (local dev only; config validation rejects it outside local).
    pub allow_mock_adapters: bool,
    /// Monthly model-cost budget in USD (FR-026); None = unlimited (dev only).
    pub monthly_budget_usd: Option<f64>,
}

impl AppState {
    pub fn new(pool: Option<PgPool>, env: String) -> Self {
        let secret = std::env::var("SINCY_JWT_SECRET")
            .unwrap_or_else(|_| "dev-only-not-for-production".to_string());
        let ttl: i64 = std::env::var("SINCY_JWT_TTL_MINUTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let allow_mock = std::env::var("SINCY_ALLOW_MOCK_ADAPTERS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let budget = std::env::var("SINCY_MONTHLY_BUDGET_USD")
            .ok()
            .and_then(|v| v.parse().ok());
        // middleware verifier (process-global)
        crate::auth::init_auth(secret.as_bytes(), ttl);
        Self {
            pool,
            env,
            jwt_secret: secret.as_bytes().to_vec(),
            jwt: Arc::new(JwtIssuer::new(secret.as_bytes(), ttl)),
            allow_mock_adapters: allow_mock,
            monthly_budget_usd: budget,
        }
    }

    pub fn from_config(pool: Option<PgPool>, cfg: &AppConfig) -> Self {
        let secret = if cfg.security.jwt_secret.is_empty() {
            "dev-only-not-for-production".to_string()
        } else {
            cfg.security.jwt_secret.clone()
        };
        // middleware verifier (process-global)
        crate::auth::init_auth(secret.as_bytes(), cfg.security.jwt_ttl_minutes);
        Self {
            pool,
            env: cfg.env.clone(),
            jwt_secret: secret.as_bytes().to_vec(),
            jwt: Arc::new(JwtIssuer::new(
                secret.as_bytes(),
                cfg.security.jwt_ttl_minutes,
            )),
            allow_mock_adapters: cfg.security.allow_mock_adapters,
            monthly_budget_usd: cfg.security.monthly_budget_usd,
        }
    }

    pub fn jwt_secret(&self) -> &[u8] {
        &self.jwt_secret
    }

    pub fn jwt(&self) -> &JwtIssuer {
        &self.jwt
    }

    pub fn pool(&self) -> Result<&PgPool, sincy_common::AppError> {
        self.pool.as_ref().ok_or_else(|| {
            sincy_common::AppError::non_retryable(
                "database_unavailable",
                "SINCY_DATABASE_URL not configured",
            )
        })
    }
}
