//! Analytics feedback (FR-016): platform metrics (API or device-read) are
//! imported with source labels, attributed to content versions, and exposed
//! for the strategy layer. 人工改稿差异回流品牌记忆 lands with WP-04 follow-up.

use sincy_common::AgencyId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct MetricImport {
    pub platform: String,
    pub external_content_id: String,
    pub account_id: Option<Uuid>,
    pub metric_date: chrono::NaiveDate,
    /// views/likes/comments/shares...
    pub metrics: serde_json::Value,
    /// api | device | manual — 统计口径必须标记 (TDD 6)
    pub source: String,
}

pub struct Analytics<'a> {
    pool: &'a PgPool,
}

impl<'a> Analytics<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Upsert a platform metric row (tenant-scoped).
    pub async fn import_metrics(
        &self,
        agency_id: AgencyId,
        import: &MetricImport,
    ) -> Result<Uuid, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "INSERT INTO platform_metrics (agency_id, platform, external_content_id, account_id, metric_date, metrics, source)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (agency_id, platform, external_content_id, metric_date, source)
             DO UPDATE SET metrics = EXCLUDED.metrics, fetched_at = now()
             RETURNING id",
        )
        .bind(agency_id.as_uuid())
        .bind(&import.platform)
        .bind(&import.external_content_id)
        .bind(import.account_id)
        .bind(import.metric_date)
        .bind(&import.metrics)
        .bind(&import.source)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.get("id"))
    }

    /// Attribute metrics to a content version snapshot (回流到策略层).
    pub async fn snapshot_performance(
        &self,
        agency_id: AgencyId,
        content_item_id: Uuid,
        content_version: i32,
        package_id: Option<Uuid>,
        platform: &str,
        external_content_id: &str,
    ) -> Result<serde_json::Value, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT metric_date, metrics, source FROM platform_metrics
             WHERE agency_id = $1 AND platform = $2 AND external_content_id = $3
             ORDER BY metric_date",
        )
        .bind(agency_id.as_uuid())
        .bind(platform)
        .bind(external_content_id)
        .fetch_all(self.pool)
        .await?;
        let series: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "date": r.get::<chrono::NaiveDate, _>("metric_date"),
                    "metrics": r.get::<serde_json::Value, _>("metrics"),
                    "source": r.get::<String, _>("source"),
                })
            })
            .collect();
        let snapshot = serde_json::json!({
            "platform": platform,
            "external_content_id": external_content_id,
            "series": series,
        });
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO content_performance_snapshots (agency_id, content_item_id, content_version, package_id, platform, external_content_id, metrics)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(agency_id.as_uuid())
        .bind(content_item_id)
        .bind(content_version)
        .bind(package_id)
        .bind(platform)
        .bind(external_content_id)
        .bind(&snapshot)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(snapshot)
    }
}
