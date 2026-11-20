//! Device agent core (WP-08/WP-11/WP-13 device side).
//!
//! This crate is the *single* implementation of on-device task execution. The
//! Android app loads it as a Rust cdylib over JNI (android/rust-core), and the
//! host simulator (`sincy-agent-sim` bin + integration tests) runs the exact
//! same code, so local verification exercises the real device path.
//!
//! Hard rules (Implementation Guide / HID design doc red lines):
//! - only run adapters whose HMAC signature verifies under the pinned platform
//!   signing key — unsigned or tampered adapters are refused with ZERO side effects;
//! - version_range guard first: target app version outside the range => ui_drift,
//!   refuse before any action;
//! - challenge screens (captcha/security) are NEVER auto-solved — the task parks
//!   as WAITING_USER and a challenge.detected envelope goes to the cloud;
//! - after the side_effect step, any failure is Uncertain (VERIFICATION_PENDING),
//!   never auto-retried or degraded.

pub mod client;
pub mod execute;

pub use client::{AgentClient, AgentError, TaskBrief};
pub use execute::{AdapterExecution, ScriptedIo, TaskOutcome, execute_signed_adapter};
