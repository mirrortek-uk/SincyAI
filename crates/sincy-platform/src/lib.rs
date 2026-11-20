//! sincy-platform: Platform App & shared quota (FR-020, TDD 8, ADR-012) and
//! the API-adapter side of the unified ChannelAction interface (TDD 7.3).
//!
//! Rules:
//! - quota_pools/quota_ledger are the AUTHORITY; Redis token buckets are only
//!   fast rate limiting (TDD 3);
//! - reserve/confirm/release must update pool balance in the SAME transaction
//!   to prevent over-issue (Guide 6);
//! - adapters never bypass the ledger;
//! - Mock adapters are explicitly labeled and must never be presented as real
//!   platform capability (Guide 7.6 / DoD).

pub mod matrix;
pub mod quota;
pub mod youtube_mock;

pub use matrix::PgCapabilityMatrix;
pub use quota::{QuotaError, QuotaLedger, Reservation};
pub mod youtube;
pub use youtube::{MediaBlob, QUOTA_COST_VIDEOS_INSERT, YouTubeAdapter, resolve_vault_ref};
pub use youtube_mock::MockYouTubeAdapter;
