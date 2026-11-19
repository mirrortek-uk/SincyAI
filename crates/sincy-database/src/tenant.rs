//! Per-transaction tenant context for Row Level Security (ADR-008).
//!
//! `SET LOCAL` scopes the setting to the current transaction only, so pooled
//! connections never leak tenant context between unrelated requests
//! (TDD 11.2: "连接池复用时事务结束清理 tenant context").

use sincy_common::AgencyId;
use sqlx::{Postgres, Transaction};

/// Tenant context handle.
#[derive(Debug, Clone)]
pub struct TenantContext {
    agency_id: AgencyId,
}

impl TenantContext {
    pub fn agency(id: AgencyId) -> Self {
        Self { agency_id: id }
    }

    pub fn agency_id(&self) -> AgencyId {
        self.agency_id
    }

    /// SQL executed at transaction start: `SET LOCAL app.current_agency_id = '...'`
    pub fn set_local_sql(&self) -> String {
        format!(
            "SET LOCAL app.current_agency_id = '{}'",
            self.agency_id.as_uuid()
        )
    }

    /// Apply the tenant context to an open transaction.
    pub async fn apply(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
        sqlx::query(&self.set_local_sql())
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}

/// Run `f` inside a transaction with the tenant context set.
/// Cross-tenant access must be rejected by RLS; agency_id is derived from the
/// authenticated context, never from client input (Guide 6).
pub async fn with_tenant<F, Fut, T>(
    pool: &sqlx::PgPool,
    ctx: TenantContext,
    f: F,
) -> Result<T, crate::DbError>
where
    F: FnOnce(Transaction<'_, Postgres>) -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::DbError>>,
{
    let mut tx = pool.begin().await?;
    ctx.apply(&mut tx).await?;
    let out = f(tx).await?;
    Ok(out)
}
