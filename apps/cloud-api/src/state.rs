//! Shared application state: DB pool (optional in local-no-db mode).

use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: Option<PgPool>,
    pub env: String,
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
