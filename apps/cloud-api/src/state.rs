//! Shared application state: DB pool (optional in local-no-db mode).

use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: Option<PgPool>,
    pub env: String,
    jwt_secret: Vec<u8>,
}

impl AppState {
    pub fn new(pool: Option<PgPool>, env: String) -> Self {
        let jwt_secret = std::env::var("SINCY_JWT_SECRET")
            .unwrap_or_else(|_| "dev-only-not-for-production".to_string())
            .into_bytes();
        Self {
            pool,
            env,
            jwt_secret,
        }
    }

    pub fn jwt_secret(&self) -> &[u8] {
        &self.jwt_secret
    }
}

impl AppState {
    pub fn pool(&self) -> Result<&PgPool, sincy_common::AppError> {
        self.pool.as_ref().ok_or_else(|| {
            sincy_common::AppError::non_retryable(
                "database_unavailable",
                "SINCY_DATABASE_URL not configured",
            )
        })
    }
}
