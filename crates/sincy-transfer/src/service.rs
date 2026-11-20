//! Transfer service: session lifecycle + chunk bookkeeping + integrity.

use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sincy_common::AgencyId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::store::{ObjectStore, StoreError};

pub const DEFAULT_CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8 MiB

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("session not found or not accessible (tenant isolation)")]
    NotFound,
    #[error("session expired")]
    Expired,
    #[error("chunk hash mismatch: chunk {index} expected {expected} got {got}")]
    ChunkHashMismatch {
        index: usize,
        expected: String,
        got: String,
    },
    #[error("whole-file hash mismatch: expected {expected} got {got}")]
    FileHashMismatch { expected: String, got: String },
    #[error("session already {0}")]
    TerminalState(String),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("db error: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct TransferSessionInfo {
    pub id: Uuid,
    pub agency_id: AgencyId,
    pub direction: String,
    pub object_key: String,
    pub total_bytes: i64,
    pub chunk_size: i64,
    pub sha256: String,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub struct TransferService<'a, S: ObjectStore> {
    pool: &'a PgPool,
    store: &'a S,
}

impl<'a, S: ObjectStore> TransferService<'a, S> {
    pub fn new(pool: &'a PgPool, store: &'a S) -> Self {
        Self { pool, store }
    }

    /// Create an upload session for `total_bytes` with whole-file sha256.
    pub async fn create_session(
        &self,
        agency_id: AgencyId,
        task_id: Option<Uuid>,
        direction: &str,
        object_key: &str,
        total_bytes: i64,
        sha256: &str,
    ) -> Result<TransferSessionInfo, TransferError> {
        let id = Uuid::new_v4();
        let chunk_size = DEFAULT_CHUNK_SIZE as i64;
        let expires = Utc::now() + Duration::hours(24);
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO transfer_sessions (id, agency_id, task_id, direction, object_key, total_bytes, chunk_size, sha256, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id)
        .bind(agency_id.as_uuid())
        .bind(task_id)
        .bind(direction)
        .bind(object_key)
        .bind(total_bytes)
        .bind(chunk_size)
        .bind(sha256)
        .bind(expires)
        .execute(&mut *tx)
        .await?;
        let n_chunks = (total_bytes + chunk_size - 1) / chunk_size;
        for i in 0..n_chunks {
            let bytes = chunk_size.min(total_bytes - i * chunk_size);
            sqlx::query(
                "INSERT INTO transfer_chunks (session_id, agency_id, chunk_index, bytes, sha256)
                 VALUES ($1, $2, $3, $4, '')",
            )
            .bind(id)
            .bind(agency_id.as_uuid())
            .bind(i as i32)
            .bind(bytes)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(TransferSessionInfo {
            id,
            agency_id,
            direction: direction.to_string(),
            object_key: object_key.to_string(),
            total_bytes,
            chunk_size,
            sha256: sha256.to_string(),
            state: "active".to_string(),
            expires_at: expires,
        })
    }

    /// Upload one chunk. Verifies its SHA-256; marks the chunk done.
    /// Idempotent: re-uploading a done chunk with the same hash is a no-op.
    pub async fn upload_chunk(
        &self,
        agency_id: AgencyId,
        session_id: Uuid,
        index: usize,
        bytes: &[u8],
    ) -> Result<(), TransferError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query(
            "SELECT state, expires_at, sha256, total_bytes, chunk_size, object_key FROM transfer_sessions
             WHERE id = $1 AND agency_id = $2 FOR UPDATE",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(TransferError::NotFound)?; // cross-tenant => NotFound (RLS also blocks)
        let state: String = row.get("state");
        if state != "active" {
            return Err(TransferError::TerminalState(state));
        }
        if row.get::<DateTime<Utc>, _>("expires_at") < Utc::now() {
            return Err(TransferError::Expired);
        }
        let chunk = sqlx::query(
            "SELECT bytes, sha256, state FROM transfer_chunks WHERE session_id = $1 AND agency_id = $2 AND chunk_index = $3",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .bind(index as i32)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(TransferError::NotFound)?;
        let expected_bytes: i64 = chunk.get("bytes");
        if bytes.len() as i64 != expected_bytes {
            return Err(TransferError::ChunkHashMismatch {
                index,
                expected: format!("{expected_bytes} bytes"),
                got: format!("{} bytes", bytes.len()),
            });
        }
        let got_hash = sha256_hex(bytes);
        let prev_state: String = chunk.get("state");
        let prev_hash: String = chunk.get("sha256");
        if prev_state == "done" {
            if prev_hash != got_hash {
                return Err(TransferError::ChunkHashMismatch {
                    index,
                    expected: prev_hash,
                    got: got_hash,
                });
            }
            return Ok(()); // idempotent no-op
        }
        // store chunk bytes under a per-chunk key
        let chunk_key = format!("{}.chunk-{index}", row.get::<String, _>("object_key"));
        self.store.put(&chunk_key, bytes).await?;
        sqlx::query(
            "UPDATE transfer_chunks SET sha256 = $1, state = 'done' WHERE session_id = $2 AND agency_id = $3 AND chunk_index = $4",
        )
        .bind(&got_hash)
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .bind(index as i32)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Which chunks still need uploading (resume support).
    pub async fn pending_chunks(
        &self,
        agency_id: AgencyId,
        session_id: Uuid,
    ) -> Result<Vec<usize>, TransferError> {
        let rows = sqlx::query(
            "SELECT chunk_index FROM transfer_chunks
             WHERE session_id = $1 AND agency_id = $2 AND state != 'done' ORDER BY chunk_index",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| r.get::<i32, _>("chunk_index") as usize)
            .collect())
    }

    /// Complete the session: assemble chunks, verify whole-file SHA-256.
    pub async fn complete(
        &self,
        agency_id: AgencyId,
        session_id: Uuid,
    ) -> Result<String, TransferError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL app.current_agency_id = '{}'",
            agency_id.as_uuid()
        ))
        .execute(&mut *tx)
        .await?;
        let sess = sqlx::query(
            "SELECT object_key, sha256, state FROM transfer_sessions WHERE id = $1 AND agency_id = $2 FOR UPDATE",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(TransferError::NotFound)?;
        if sess.get::<String, _>("state") != "active" {
            return Err(TransferError::TerminalState(sess.get("state")));
        }
        let undone: i64 = sqlx::query(
            "SELECT count(*) FROM transfer_chunks WHERE session_id = $1 AND agency_id = $2 AND state != 'done'",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?
        .get(0);
        if undone > 0 {
            return Err(TransferError::TerminalState(format!(
                "{undone} chunks pending"
            )));
        }
        // assemble
        let object_key: String = sess.get("object_key");
        let mut whole = Vec::new();
        let chunks = sqlx::query(
            "SELECT chunk_index FROM transfer_chunks WHERE session_id = $1 AND agency_id = $2 ORDER BY chunk_index",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        for c in chunks {
            let idx: i32 = c.get("chunk_index");
            let data = self.store.get(&format!("{object_key}.chunk-{idx}")).await?;
            whole.extend_from_slice(&data);
        }
        let got = sha256_hex(&whole);
        let expected: String = sess.get("sha256");
        if got != expected {
            return Err(TransferError::FileHashMismatch { expected, got });
        }
        self.store.put(&object_key, &whole).await?;
        sqlx::query("UPDATE transfer_sessions SET state = 'completed' WHERE id = $1")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(got)
    }

    /// Download path: verify tenant, return whole object bytes.
    pub async fn download(
        &self,
        agency_id: AgencyId,
        session_id: Uuid,
    ) -> Result<Vec<u8>, TransferError> {
        let sess = sqlx::query(
            "SELECT object_key, state FROM transfer_sessions WHERE id = $1 AND agency_id = $2",
        )
        .bind(session_id)
        .bind(agency_id.as_uuid())
        .fetch_optional(self.pool)
        .await?
        .ok_or(TransferError::NotFound)?; // RLS + explicit agency check: cross-tenant rejected
        if sess.get::<String, _>("state") != "completed" {
            return Err(TransferError::TerminalState(sess.get("state")));
        }
        Ok(self.store.get(&sess.get::<String, _>("object_key")).await?)
    }
}
