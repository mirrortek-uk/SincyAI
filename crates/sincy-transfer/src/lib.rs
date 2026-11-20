//! sincy-transfer (WP-10, FR-012): chunked transfer with resume and SHA-256.
//!
//! - metadata + per-chunk hashes live in PostgreSQL (transfer_sessions/chunks);
//! - bytes go through an ObjectStore (S3-compatible in prod; local FS in dev);
//! - resume: only pending/failed chunks are (re)sent;
//! - completion verifies the whole-file SHA-256 before marking completed;
//! - cross-tenant access is rejected (RLS + explicit agency check).

pub mod service;
pub mod store;

pub use service::{TransferError, TransferService, TransferSessionInfo};
pub use store::{LocalObjectStore, ObjectStore};
