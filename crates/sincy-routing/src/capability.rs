//! Capability matrix model (TDD 7.1): one row per "platform × action × route".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Execution path kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    Api,
    Device,
    Assisted,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Route::Api => "api",
            Route::Device => "device",
            Route::Assisted => "assisted",
        }
    }
}

/// Verification status of a capability row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Verified,
    Unverified,
    Degraded,
    Unsupported,
}

/// channel_capabilities row (platform-level table, Platform Admin writable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelCapability {
    pub platform: String, // youtube, instagram, xiaohongshu ...
    pub action: String,   // publish_video, publish_image, schedule, reply_comment, read_metrics ...
    pub route: Route,
    pub status: CapabilityStatus,
    /// media spec, account type, max length, quota cost ...
    pub constraints: serde_json::Value,
    /// lower = preferred
    pub priority: i32,
    pub verified_at: Option<DateTime<Utc>>,
    /// doc URL / test run id
    pub evidence_ref: Option<String>,
}

impl ChannelCapability {
    /// Routes eligible for selection (TDD 7.2: verified | degraded).
    pub fn selectable(&self) -> bool {
        matches!(
            self.status,
            CapabilityStatus::Verified | CapabilityStatus::Degraded
        )
    }
}
