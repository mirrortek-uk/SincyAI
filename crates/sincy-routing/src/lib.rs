//! sincy-routing: FR-019 / TDD 7 — capability-matrix driven Execution Router.
//!
//! The router NEVER executes external actions itself; it selects a route and
//! records why. Degradation is only allowed for failures classified as
//! `NoSideEffect`; `Uncertain` results must go to VERIFICATION_PENDING.

pub mod action;
pub mod capability;
pub mod router;

pub use action::{ChannelAction, ExecOutcome, Precheck, VerifyOutcome};
pub use capability::{CapabilityStatus, ChannelCapability, Route};
pub use router::{RouteDecision, RouteReason, Router, SkippedPath};
