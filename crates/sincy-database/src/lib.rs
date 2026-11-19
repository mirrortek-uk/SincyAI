//! sincy-database: connection pool, migrations, tenant context for RLS.
//!
//! Rules (TDD 11.2 / Guide 6):
//! - every tenant-scoped query runs inside [`with_tenant`], which sets
//!   `app.current_agency_id` for PostgreSQL Row Level Security;
//! - tenant context MUST be cleared when a pooled connection is returned —
//!   we do it in the same transaction scope (`SET LOCAL`);
//! - platform-level tables (platform_apps, quota_pools, channel_capabilities,
//!   app_adapters) have no agency_id and are only writable by Platform Admin.

pub mod pool;
pub mod tenant;

pub use pool::{Database, connect};
pub use tenant::{TenantContext, with_tenant};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migrate error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_context_sql_uses_set_local() {
        let agency = sincy_common::AgencyId::new();
        let sql = TenantContext::agency(agency).set_local_sql();
        assert!(sql.contains("SET LOCAL"));
        assert!(sql.contains("app.current_agency_id"));
        assert!(sql.contains(&agency.as_uuid().to_string()));
    }
}
