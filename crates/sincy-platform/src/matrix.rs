//! PostgreSQL-backed capability matrix (TDD 7.1). channel_capabilities is a
//! PLATFORM-LEVEL table: read without tenant context, written only by
//! Platform Admin (enforced at API layer; see cloud-api admin routes later).

use sincy_routing::capability::{CapabilityStatus, ChannelCapability, Route};
use sincy_routing::router::CapabilityMatrix;
use sqlx::{PgPool, Row};

pub struct PgCapabilityMatrix<'a> {
    pool: &'a PgPool,
}

impl<'a> PgCapabilityMatrix<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn all(
        &self,
        platform: &str,
        action: &str,
    ) -> Result<Vec<ChannelCapability>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT platform, action, route, status, constraints, priority, verified_at, evidence_ref
             FROM channel_capabilities WHERE platform = $1 AND action = $2",
        )
        .bind(platform)
        .bind(action)
        .fetch_all(self.pool)
        .await?;
        Ok(rows.iter().map(row_to_cap).collect())
    }

    /// Mark a route degraded (e.g. after UI drift / API error streak). Platform Admin only.
    pub async fn mark_degraded(
        &self,
        platform: &str,
        action: &str,
        route: Route,
        evidence_ref: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE channel_capabilities
             SET status = 'degraded', evidence_ref = $4, updated_at = now()
             WHERE platform = $1 AND action = $2 AND route = $3",
        )
        .bind(platform)
        .bind(action)
        .bind(route.as_str())
        .bind(evidence_ref)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

fn row_to_cap(row: &sqlx::postgres::PgRow) -> ChannelCapability {
    let route = match row.get::<String, _>("route").as_str() {
        "api" => Route::Api,
        "device" => Route::Device,
        _ => Route::Assisted,
    };
    let status = match row.get::<String, _>("status").as_str() {
        "verified" => CapabilityStatus::Verified,
        "degraded" => CapabilityStatus::Degraded,
        "unsupported" => CapabilityStatus::Unsupported,
        _ => CapabilityStatus::Unverified,
    };
    ChannelCapability {
        platform: row.get("platform"),
        action: row.get("action"),
        route,
        status,
        constraints: row.get("constraints"),
        priority: row.get("priority"),
        verified_at: row.get("verified_at"),
        evidence_ref: row.get("evidence_ref"),
    }
}

impl CapabilityMatrix for PgCapabilityMatrix<'_> {
    fn candidates(&self, _platform: &str, _action: &str) -> Vec<ChannelCapability> {
        // async fetch happens in `all()`; the sync trait is used by the pure
        // router core with an in-memory snapshot (see Router::select callers).
        // PgCapabilityMatrix::snapshot loads rows for the router.
        Vec::new()
    }
}

/// In-memory snapshot adapter: load from PG once, then run the pure router.
pub struct MatrixSnapshot {
    pub rows: Vec<ChannelCapability>,
}

impl CapabilityMatrix for MatrixSnapshot {
    fn candidates(&self, platform: &str, action: &str) -> Vec<ChannelCapability> {
        self.rows
            .iter()
            .filter(|c| c.platform == platform && c.action == action)
            .cloned()
            .collect()
    }
}

impl<'a> PgCapabilityMatrix<'a> {
    pub async fn snapshot(
        &self,
        platform: &str,
        action: &str,
    ) -> Result<MatrixSnapshot, sqlx::Error> {
        Ok(MatrixSnapshot {
            rows: self.all(platform, action).await?,
        })
    }
}
