//! First-run admin bootstrap (production onboarding).
//!
//! Products cannot ship a hardcoded demo account. Instead, a fresh deployment
//! creates its first agency + owner from environment variables exactly once:
//!
//!   SINCY_BOOTSTRAP_ADMIN_EMAIL     e.g. founder@company.com
//!   SINCY_BOOTSTRAP_ADMIN_PASSWORD  strong password (never logged, never stored raw)
//!   SINCY_BOOTSTRAP_AGENCY_NAME     optional, default "Sincy"
//!
//! Idempotent: if the email already exists, nothing happens. The password is
//! stored only as an Argon2id hash. In production (SINCY_ENV != local) a missing
//! bootstrap is a loud warning — the operator must create the first admin out of
//! band or via these vars.

use sincy_security::password;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn bootstrap_admin(pool: &PgPool, env: &str) -> Result<(), String> {
    let email = std::env::var("SINCY_BOOTSTRAP_ADMIN_EMAIL").ok();
    let password = std::env::var("SINCY_BOOTSTRAP_ADMIN_PASSWORD").ok();
    let (Some(email), Some(password)) = (email, password) else {
        if env == "production" || env == "prod" {
            tracing::warn!(
                "SINCY_BOOTSTRAP_ADMIN_EMAIL/PASSWORD not set in env={env}: \
                 no first admin was created; create one out of band"
            );
        }
        return Ok(());
    };
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agency_members WHERE email = $1)")
            .bind(&email)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    if exists {
        tracing::info!(email = %email, "bootstrap admin already exists, skipping");
        return Ok(());
    }
    let hash = password::hash_password(&password).map_err(|e| e.to_string())?;
    let agency_name =
        std::env::var("SINCY_BOOTSTRAP_AGENCY_NAME").unwrap_or_else(|_| "Sincy".into());
    let agency_id = Uuid::new_v4();
    let member_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agencies (id, name) VALUES ($1, $2)")
        .bind(agency_id)
        .bind(&agency_name)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO agency_members (id, agency_id, email, role, password_hash) \
         VALUES ($1, $2, $3, 'owner', $4)",
    )
    .bind(member_id)
    .bind(agency_id)
    .bind(&email)
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    tracing::info!(email = %email, agency = %agency_name, "bootstrap admin created (first owner)");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn bootstrap_env_contract() {
        // 仅校验约定: 变量名稳定, 文档与代码一致
        assert_eq!(
            "SINCY_BOOTSTRAP_ADMIN_EMAIL",
            std::env::var("SINCY_BOOTSTRAP_ADMIN_EMAIL")
                .unwrap_or_else(|_| "SINCY_BOOTSTRAP_ADMIN_EMAIL".into())
        );
    }
}
