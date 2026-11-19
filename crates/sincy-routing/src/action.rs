//! Unified channel action interface (TDD 7.3).
//!
//! API Adapters implement this in Cloud; the device route is implemented by a
//! DeviceRouteProxy that converts execution into a Device Task and awaits the
//! result. Upper layers only depend on action semantics.

use sincy_common::AppError;

/// Result of a precheck before execution.
#[derive(Debug, Clone)]
pub struct Precheck {
    pub ok: bool,
    /// why the route is not usable right now (quota exhausted, offline, ...)
    pub reason: Option<String>,
}

impl Precheck {
    pub fn ok() -> Self {
        Self {
            ok: true,
            reason: None,
        }
    }
    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
        }
    }
}

/// Outcome of an execution attempt. Not Clone: carries AppError with sources.
#[derive(Debug)]
pub enum ExecOutcome {
    Success {
        external_content_id: Option<String>,
        receipt: serde_json::Value,
    },
    /// Failed before any external side effect — eligible for degradation.
    NoSideEffectFailure(AppError),
    /// Side effect may have happened — must NOT retry or degrade.
    Uncertain(AppError),
}

#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub verified: bool,
    pub external_content_id: Option<String>,
    pub evidence: serde_json::Value,
}

/// Capability descriptor of an action implementation.
#[derive(Debug, Clone)]
pub struct CapabilityDescriptor {
    pub platform: String,
    pub action: String,
    pub route: crate::capability::Route,
    pub constraints: serde_json::Value,
}

/// One "platform × action" executable path.
#[async_trait::async_trait]
pub trait ChannelAction: Send + Sync {
    fn capability(&self) -> CapabilityDescriptor;

    async fn precheck(&self, ctx: &ExecCtx, pkg: &PublishingPackageRef) -> Precheck;

    async fn execute(&self, ctx: &ExecCtx, pkg: &PublishingPackageRef) -> ExecOutcome;

    async fn verify(&self, ctx: &ExecCtx, attempt: &str) -> VerifyOutcome;
}

/// Execution context handed to actions (tenant, account, task, route, quota).
#[derive(Debug, Clone)]
pub struct ExecCtx {
    pub agency_id: String,
    pub brand_id: String,
    pub account_id: String,
    pub task_id: String,
    pub route_id: String,
    pub idempotency_key: String,
}

/// Reference to an immutable Publishing Package (TDD 13) — never inline media.
#[derive(Debug, Clone)]
pub struct PublishingPackageRef {
    pub package_id: String,
    pub version: u32,
    pub platform: String,
    pub action: String,
}
