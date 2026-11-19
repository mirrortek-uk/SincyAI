//! Shared quota ledger (TDD 8): reserve -> confirm | release.
//!
//! reserve() is idempotent per (agency_id, task_id): a repeated call returns
//! the existing reservation instead of double-charging (TDD 14 idempotency).

use sincy_common::{AgencyId, AppError};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum QuotaError {
    #[error("quota exhausted for pool {pool}: requested {requested}, available {available}")]
    Exhausted {
        pool: String,
        requested: i64,
        available: i64,
    },
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("application error: {0}")]
    App(#[from] AppError),
}

#[derive(Debug, Clone)]
pub struct Reservation {
    pub reservation_id: Uuid,
    pub pool_id: Uuid,
    pub agency_id: AgencyId,
    pub amount: i64,
    pub state: String,
}

pub struct QuotaLedger<'a> {
    pool: &'a PgPool,
}

impl<'a> QuotaLedger<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Reserve `amount` units from `quota_pool_id` for (agency, task).
    /// Idempotent per (agency_id, task_id); updates pool balance in the same tx.
    pub async fn reserve(
        &self,
        agency_id: AgencyId,
        quota_pool_id: Uuid,
        amount: i64,
        task_id: Uuid,
    ) -> Result<Reservation, QuotaError> {
        let mut tx = self.pool.begin().await?;
        // existing active reservation for this task => return it (idempotent)
        if let Some(row) = sqlx::query(
            "SELECT reservation_id, quota_pool_id, agency_id, amount, state
             FROM quota_ledger
             WHERE agency_id = $1 AND task_id = $2 AND state = 'reserved'",
        )
        .bind(agency_id.as_uuid())
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            return Ok(Reservation {
                reservation_id: row.get("reservation_id"),
                pool_id: row.get("quota_pool_id"),
                agency_id,
                amount: row.get("amount"),
                state: row.get("state"),
            });
        }

        // lock the pool row and check capacity atomically (prevents over-issue)
        let pool_row =
            sqlx::query("SELECT used_value, limit_value FROM quota_pools WHERE id = $1 FOR UPDATE")
                .bind(quota_pool_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| QuotaError::Exhausted {
                    pool: quota_pool_id.to_string(),
                    requested: amount,
                    available: 0,
                })?;
        let used: i64 = pool_row.get("used_value");
        let limit: i64 = pool_row.get("limit_value");
        let available = limit - used;
        if available < amount {
            return Err(QuotaError::Exhausted {
                pool: quota_pool_id.to_string(),
                requested: amount,
                available,
            });
        }

        let reservation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO quota_ledger (reservation_id, agency_id, quota_pool_id, amount, state, task_id)
             VALUES ($1, $2, $3, $4, 'reserved', $5)",
        )
        .bind(reservation_id)
        .bind(agency_id.as_uuid())
        .bind(quota_pool_id)
        .bind(amount)
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE quota_pools SET used_value = used_value + $1, updated_at = now() WHERE id = $2",
        )
        .bind(amount)
        .bind(quota_pool_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Reservation {
            reservation_id,
            pool_id: quota_pool_id,
            agency_id,
            amount,
            state: "reserved".to_string(),
        })
    }

    /// Confirm a reservation after the external side effect succeeded.
    pub async fn confirm(&self, reservation_id: Uuid) -> Result<(), QuotaError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE quota_ledger SET state = 'confirmed', resolved_at = now()
             WHERE reservation_id = $1 AND state = 'reserved'",
        )
        .bind(reservation_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            return Err(AppError::non_retryable(
                "reservation_not_found",
                format!("reservation {reservation_id} not in reserved state"),
            )
            .into());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Release a reservation (failure/timeout): returns units to the pool.
    pub async fn release(&self, reservation_id: Uuid) -> Result<(), QuotaError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "UPDATE quota_ledger SET state = 'released', resolved_at = now()
             WHERE reservation_id = $1 AND state = 'reserved'
             RETURNING quota_pool_id, amount",
        )
        .bind(reservation_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(()); // already resolved: idempotent
        };
        let pool_id: Uuid = row.get("quota_pool_id");
        let amount: i64 = row.get("amount");
        sqlx::query(
            "UPDATE quota_pools SET used_value = GREATEST(0, used_value - $1), updated_at = now() WHERE id = $2",
        )
        .bind(amount)
        .bind(pool_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Available units of a pool right now.
    pub async fn available(&self, quota_pool_id: Uuid) -> Result<i64, QuotaError> {
        let row =
            sqlx::query("SELECT limit_value - used_value AS avail FROM quota_pools WHERE id = $1")
                .bind(quota_pool_id)
                .fetch_one(self.pool)
                .await?;
        Ok(row.get("avail"))
    }
}
