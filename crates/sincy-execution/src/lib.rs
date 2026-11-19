//! sincy-execution (WP-09 core + FR-025 kill switch).
//!
//! State transitions are event-driven and guarded (TDD 14): illegal
//! transitions are rejected. Once a task passes its side_effect step,
//! `side_effect_done` is set and automatic retry/degrade is forbidden.

pub mod idempotency;
pub mod killswitch;
pub mod state;

pub use idempotency::{IdempotencyStore, MemoryIdempotencyStore};
pub use killswitch::{KillScope, KillSwitchStore, MemoryKillSwitchStore};
pub use state::{ContentState, DeviceTaskState, IllegalTransition, StateMachine, TaskEvent};
