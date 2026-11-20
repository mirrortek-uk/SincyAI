//! Model cost budget (FR-026): per agency/brand monthly budget; over threshold
//! => alert or circuit-break (new generation tasks blocked). Costs are recorded
//! per content item (model_cost_records, migration 0005).

use chrono::{Datelike, Utc};
use rust_decimal::Decimal;
use sincy_common::AgencyId;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum BudgetError {
    #[error("budget exceeded: spent {spent} + est {est} > limit {limit} (agency {agency})")]
    OverBudget {
        agency: String,
        spent: String,
        est: String,
        limit: String,
    },
    #[error("db error: {0}")]
    Db(#[from] sqlx::Error),
}

/// One model-cost line (FR-026 attribution).
pub struct CostRecord<'a> {
    pub brand_id: Option<Uuid>,
    pub content_item_id: Option<Uuid>,
    pub provider: &'a str,
    pub model: &'a str,
    pub kind: &'a str, // llm | image | video | speech
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: Decimal,
}

pub struct BudgetChecker<'a> {
    pool: &'a PgPool,
    /// monthly budget in USD; None = unlimited (dev)
    monthly_limit: Option<Decimal>,
}

impl<'a> BudgetChecker<'a> {
    pub fn new(pool: &'a PgPool, monthly_limit: Option<Decimal>) -> Self {
        Self {
            pool,
            monthly_limit,
        }
    }

    /// Check whether an estimated cost fits the remaining monthly budget.
    pub async fn check(
        &self,
        agency_id: AgencyId,
        brand_id: Option<Uuid>,
        est_cost: Decimal,
    ) -> Result<(), BudgetError> {
        let Some(limit) = self.monthly_limit else {
            return Ok(());
        };
        let now = Utc::now();
        let month_start = chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
            .expect("valid date")
            .and_hms_opt(0, 0, 0)
            .expect("valid time");
        let spent: Decimal = sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_usd), 0) FROM model_cost_records
             WHERE agency_id = $1 AND ($2::uuid IS NULL OR brand_id = $2)
               AND created_at >= $3",
        )
        .bind(agency_id.as_uuid())
        .bind(brand_id)
        .bind(month_start)
        .fetch_one(self.pool)
        .await?;
        if spent + est_cost > limit {
            return Err(BudgetError::OverBudget {
                agency: agency_id.to_string(),
                spent: spent.to_string(),
                est: est_cost.to_string(),
                limit: limit.to_string(),
            });
        }
        Ok(())
    }

    /// Record a real cost against the content item (FR-026: 成本计入内容与报告).
    pub async fn record_cost(
        &self,
        agency_id: AgencyId,
        rec: &CostRecord<'_>,
    ) -> Result<Uuid, BudgetError> {
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO model_cost_records (id, agency_id, brand_id, content_item_id, provider, model, kind, tokens_in, tokens_out, cost_usd)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(agency_id.as_uuid())
        .bind(rec.brand_id)
        .bind(rec.content_item_id)
        .bind(rec.provider)
        .bind(rec.model)
        .bind(rec.kind)
        .bind(rec.tokens_in)
        .bind(rec.tokens_out)
        .bind(rec.cost_usd)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }
}
