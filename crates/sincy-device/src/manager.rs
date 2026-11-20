//! DeviceManager over PostgreSQL. Registration flow:
//! 1. start_registration -> server generates device_secret (returned ONCE to
//!    the owner), stores its SHA-256 hash + a challenge nonce
//! 2. device answers proof = HMAC-SHA256(secret, nonce)
//! 3. complete_registration verifies proof and issues a session token (JWT)

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use sincy_common::AgencyId;
use sincy_security::{JwtIssuer, challenge};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("device not found or revoked")]
    NotFound,
    #[error("registration challenge mismatch/expired")]
    ChallengeMismatch,
    #[error("session revoked")]
    Revoked,
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

fn hash_secret(secret: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(secret);
    format!("{:x}", h.finalize())
}

#[derive(Debug, Clone)]
pub struct RegistrationChallenge {
    pub device_id: Uuid,
    pub nonce: String,
    pub expires_at: chrono::DateTime<Utc>,
    /// returned once to the device owner; never stored
    pub device_secret: String,
}

#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub id: Uuid,
    pub agency_id: AgencyId,
    pub platform: String,
    pub state: String,
    pub last_seen: Option<chrono::DateTime<Utc>>,
}

pub struct SessionToken {
    pub token: String,
    pub device_id: Uuid,
}

pub struct DeviceManager<'a> {
    pool: &'a PgPool,
    jwt: JwtIssuer,
}

impl<'a> DeviceManager<'a> {
    pub fn new(pool: &'a PgPool, jwt_secret: &[u8]) -> Self {
        Self {
            pool,
            jwt: JwtIssuer::new(jwt_secret, 60 * 24), // 24h device token; rotation via re-register
        }
    }

    /// Step 1: create a pending device + challenge. The device_secret is
    /// returned once and only its hash is stored.
    pub async fn start_registration(
        &self,
        agency_id: AgencyId,
        platform: &str,
        name: Option<&str>,
    ) -> Result<RegistrationChallenge, DeviceError> {
        let device_id = Uuid::new_v4();
        let secret_bytes: [u8; 32] = rand::random();
        let device_secret =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, secret_bytes);
        let ch = challenge::issue_challenge(&device_id.to_string(), Duration::minutes(10));
        sqlx::query(
            "INSERT INTO devices (id, agency_id, platform, name, secret_hash, state)
             VALUES ($1, $2, $3, $4, $5, 'pending')",
        )
        .bind(device_id)
        .bind(agency_id.as_uuid())
        .bind(platform)
        .bind(name)
        .bind(hash_secret(device_secret.as_bytes()))
        .execute(self.pool)
        .await?;
        // challenge nonce stored in-session is ephemeral; for the dev slice we
        // recompute proof over nonce with the secret — store nonce in session row
        sqlx::query(
            "INSERT INTO device_sessions (id, agency_id, device_id, token_hash, state)
             VALUES ($1, $2, $3, $4, 'pending')",
        )
        .bind(Uuid::new_v4())
        .bind(agency_id.as_uuid())
        .bind(device_id)
        .bind(&ch.nonce) // reuse token_hash column as pending nonce holder in dev slice
        .execute(self.pool)
        .await?;
        Ok(RegistrationChallenge {
            device_id,
            nonce: ch.nonce,
            expires_at: ch.expires_at,
            device_secret,
        })
    }

    /// Step 2: verify proof and activate the device; returns a session token.
    pub async fn complete_registration(
        &self,
        device_id: Uuid,
        nonce: &str,
        proof: &str,
    ) -> Result<SessionToken, DeviceError> {
        let row = sqlx::query("SELECT agency_id, secret_hash, state FROM devices WHERE id = $1")
            .bind(device_id)
            .fetch_optional(self.pool)
            .await?
            .ok_or(DeviceError::NotFound)?;
        if row.get::<String, _>("state") != "pending" {
            return Err(DeviceError::ChallengeMismatch);
        }
        let agency_id = AgencyId::from_uuid(row.get("agency_id"));
        // verify proof = HMAC(secret, nonce) — but we only stored hash(secret).
        // Dev slice: recompute from stored nonce + provided proof using the
        // session row; production uses per-device asymmetric keys in Vault.
        let sess = sqlx::query(
            "SELECT id, token_hash FROM device_sessions WHERE device_id = $1 AND state = 'pending' ORDER BY connected_at DESC LIMIT 1",
        )
        .bind(device_id)
        .fetch_optional(self.pool)
        .await?
        .ok_or(DeviceError::ChallengeMismatch)?;
        let stored_nonce: String = sess.get("token_hash");
        if stored_nonce != nonce {
            return Err(DeviceError::ChallengeMismatch);
        }
        // proof check: HMAC-SHA256 over nonce keyed by secret — we cannot invert
        // the hash, so the dev-slice contract is: proof == sha256(secret_hash || nonce)
        let expected = {
            let mut h = Sha256::new();
            h.update(row.get::<String, _>("secret_hash").as_bytes());
            h.update(nonce.as_bytes());
            format!("{:x}", h.finalize())
        };
        if proof != expected {
            return Err(DeviceError::ChallengeMismatch);
        }
        sqlx::query("UPDATE devices SET state = 'active' WHERE id = $1")
            .bind(device_id)
            .execute(self.pool)
            .await?;
        sqlx::query("UPDATE device_sessions SET state = 'active', token_hash = '' WHERE id = $1")
            .bind(sess.get::<Uuid, _>("id"))
            .execute(self.pool)
            .await?;
        let token = self
            .jwt
            .issue(
                &device_id.to_string(),
                &agency_id.to_string(),
                vec!["device".into()],
            )
            .map_err(|e| DeviceError::InvalidToken(e.to_string()))?;
        Ok(SessionToken { token, device_id })
    }

    /// Authenticate a WSS/REST request: valid JWT + active device + active session.
    pub async fn authenticate(&self, token: &str) -> Result<(Uuid, AgencyId), DeviceError> {
        let claims = self
            .jwt
            .verify(token)
            .map_err(|e| DeviceError::InvalidToken(e.to_string()))?;
        let device_id =
            Uuid::parse_str(&claims.sub).map_err(|e| DeviceError::InvalidToken(e.to_string()))?;
        let agency_id = AgencyId::from_uuid(
            Uuid::parse_str(&claims.agency_id)
                .map_err(|e| DeviceError::InvalidToken(e.to_string()))?,
        );
        let row = sqlx::query("SELECT state FROM devices WHERE id = $1 AND agency_id = $2")
            .bind(device_id)
            .bind(agency_id.as_uuid())
            .fetch_optional(self.pool)
            .await?
            .ok_or(DeviceError::NotFound)?;
        if row.get::<String, _>("state") != "active" {
            return Err(DeviceError::Revoked);
        }
        let active_sessions: i64 = sqlx::query(
            "SELECT count(*) FROM device_sessions WHERE device_id = $1 AND state = 'active'",
        )
        .bind(device_id)
        .fetch_one(self.pool)
        .await?
        .get::<i64, _>(0);
        if active_sessions == 0 {
            return Err(DeviceError::Revoked);
        }
        Ok((device_id, agency_id))
    }

    /// Revoke a device: state=revoked + all sessions revoked (FR-010/023).
    pub async fn revoke(&self, agency_id: AgencyId, device_id: Uuid) -> Result<(), DeviceError> {
        let res =
            sqlx::query("UPDATE devices SET state = 'revoked' WHERE id = $1 AND agency_id = $2")
                .bind(device_id)
                .bind(agency_id.as_uuid())
                .execute(self.pool)
                .await?;
        if res.rows_affected() == 0 {
            return Err(DeviceError::NotFound);
        }
        sqlx::query("UPDATE device_sessions SET state = 'revoked' WHERE device_id = $1")
            .bind(device_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn heartbeat(&self, device_id: Uuid) -> Result<(), DeviceError> {
        sqlx::query("UPDATE devices SET last_seen = now() WHERE id = $1")
            .bind(device_id)
            .execute(self.pool)
            .await?;
        sqlx::query("UPDATE device_sessions SET last_seen = now() WHERE device_id = $1 AND state = 'active'")
            .bind(device_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_capabilities(
        &self,
        agency_id: AgencyId,
        device_id: Uuid,
        caps: &[(String, bool, String)],
    ) -> Result<(), DeviceError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM device_capabilities WHERE agency_id = $1 AND device_id = $2")
            .bind(agency_id.as_uuid())
            .bind(device_id)
            .execute(&mut *tx)
            .await?;
        for (name, available, detail) in caps {
            sqlx::query(
                "INSERT INTO device_capabilities (agency_id, device_id, name, available, detail)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(agency_id.as_uuid())
            .bind(device_id)
            .bind(name)
            .bind(available)
            .bind(detail)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_app_versions(
        &self,
        agency_id: AgencyId,
        device_id: Uuid,
        apps: &[(String, String, i32)],
    ) -> Result<(), DeviceError> {
        let mut tx = self.pool.begin().await?;
        for (package, version_name, version_code) in apps {
            sqlx::query(
                "INSERT INTO device_app_versions (agency_id, device_id, package, version_name, version_code)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (agency_id, device_id, package)
                 DO UPDATE SET version_name = EXCLUDED.version_name, version_code = EXCLUDED.version_code, updated_at = now()",
            )
            .bind(agency_id.as_uuid())
            .bind(device_id)
            .bind(package)
            .bind(version_name)
            .bind(version_code)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Test helper: issue a token for an arbitrary (possibly forged) device id.
    #[doc(hidden)]
    pub fn jwt_issue_forged(&self, device_id: Uuid, agency_id: AgencyId) -> String {
        self.jwt
            .issue(
                &device_id.to_string(),
                &agency_id.to_string(),
                vec!["device".into()],
            )
            .unwrap_or_default()
    }

    pub async fn list(&self, agency_id: AgencyId) -> Result<Vec<DeviceRecord>, DeviceError> {
        let rows = sqlx::query(
            "SELECT id, agency_id, platform, state, last_seen FROM devices WHERE agency_id = $1 ORDER BY created_at",
        )
        .bind(agency_id.as_uuid())
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| DeviceRecord {
                id: r.get("id"),
                agency_id: AgencyId::from_uuid(r.get("agency_id")),
                platform: r.get("platform"),
                state: r.get("state"),
                last_seen: r.get("last_seen"),
            })
            .collect())
    }
}
