//! Connection pool management.

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::DbError;

/// Thin wrapper over the PostgreSQL pool.
#[derive(Debug, Clone)]
pub struct Database {
    pub pool: PgPool,
}

/// Connect with pool options from config. Runs numbered migrations from
/// `migrations/` — never alters schema at runtime (Guide 3.2).
pub async fn connect(url: &str, max_connections: u32) -> Result<Database, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(url)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(Database { pool })
}

impl Database {
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
