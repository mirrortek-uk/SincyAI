//! sincy-common: shared types and the unified error model.
//!
//! Error taxonomy (Implementation Guide 3.2):
//! - retryable            — transient failure, safe to retry with backoff
//! - non_retryable        — permanent failure, do not retry
//! - needs_user           — requires human action (challenge, consent, approval)
//! - no_side_effect       — failed before any external side effect; may degrade to another route
//! - uncertain            — side effect may have happened; must NOT retry or degrade

pub mod error;
pub mod ids;
pub mod paging;

pub use error::{AppError, ErrorKind};
pub use ids::{AgencyId, BrandId, DeviceId, IdempotencyKey, RequestId, TaskId};
pub use paging::{Page, PageRequest};
