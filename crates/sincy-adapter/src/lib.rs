//! sincy-adapter (WP-13, FR-021/022, TDD 9):
//! - adapters are DATA, not code: declarative YAML/JSON definitions, schema
//!   validated, signed by Sincy, and only executed when the target App version
//!   matches `version_range` (device-side signature verification);
//! - the interpreter runs the OBSERVE -> IDENTIFY -> GUARD -> ACT -> VERIFY ->
//!   EVIDENCE loop over a platform-neutral screen snapshot;
//! - challenge screens always park the task (WAITING_USER); unknown screens
//!   only allow whitelisted safe actions, at most N times, then WAITING_USER;
//! - visual models never drive side-effecting steps;
//! - UI drift (version out of range / low anchor match / step timeouts) marks
//!   the adapter degraded and blocks execution.

pub mod drift;
pub mod interpreter;
pub mod schema;
pub mod sign;
pub mod version_range;

pub use drift::{DriftReason, DriftVerdict, anchor_match_rate};
pub use interpreter::{
    Action, AdapterRuntime, ExecEvent, ExecStatus, Interpreter, ScreenSnapshot, StepOutcome,
};
pub use schema::{AdapterDefinition, AdapterError, FlowStep, ScreenDef};
pub use sign::{SigningError, sign_definition, verify_definition};
pub use version_range::{VersionRange, VersionRangeError};
