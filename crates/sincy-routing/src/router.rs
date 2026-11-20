//! Routing algorithm (TDD 7.2):
//! 1. take candidate routes for (platform, action), keep status ∈ {verified, degraded}
//! 2. apply tenant/account overrides (e.g. a client disables the device route)
//! 3. check per-route preconditions (OAuth/scope/quota | device binding/online/
//!    permissions/consent/adapter | assisted needs a device or online user)
//! 4. pick the first by priority, record every skipped path and why
//! 5. on failure: only NoSideEffectFailure may degrade to the next route;
//!    Uncertain goes to VERIFICATION_PENDING and forbids switching.

use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityStatus, ChannelCapability, Route};

/// Why a candidate path was skipped during selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedPath {
    pub route: Route,
    pub reason: String,
}

/// Why the selected route was chosen (audit: execution_routes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteReason {
    pub selected: Route,
    pub priority: i32,
    pub capability_status: CapabilityStatus,
    pub skipped: Vec<SkippedPath>,
}

/// The router's decision for one task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub platform: String,
    pub action: String,
    pub reason: RouteReason,
}

/// Per-route preconditions, supplied by the caller (account state, quota,
/// device state, adapter match). Keeps the router pure and testable.
#[derive(Debug, Clone, Default)]
pub struct Preconditions {
    /// API route: OAuth valid + scope covers action + quota reservable
    pub api_ready: bool,
    pub api_block_reason: Option<String>,
    /// Device route: bound + online + permissions + automation consent + adapter match
    pub device_ready: bool,
    pub device_block_reason: Option<String>,
    /// Assisted route: a device is available or the user is reachable
    pub assisted_ready: bool,
    pub assisted_block_reason: Option<String>,
}

/// Tenant/account level route overrides (FR-019: 按租户/账号覆盖).
#[derive(Debug, Clone, Default)]
pub struct RoutingOverrides {
    pub disabled_routes: Vec<Route>,
}

/// The capability matrix + override store the router reads.
pub trait CapabilityMatrix: Send + Sync {
    fn candidates(&self, platform: &str, action: &str) -> Vec<ChannelCapability>;
}

pub struct Router<'a> {
    matrix: &'a dyn CapabilityMatrix,
}

impl<'a> Router<'a> {
    pub fn new(matrix: &'a dyn CapabilityMatrix) -> Self {
        Self { matrix }
    }

    /// Select a route for (platform, action). Returns None when no path is usable.
    pub fn select(
        &self,
        platform: &str,
        action: &str,
        overrides: &RoutingOverrides,
        pre: &Preconditions,
    ) -> Option<RouteDecision> {
        let mut candidates = self.matrix.candidates(platform, action);
        candidates.retain(|c| c.selectable());
        candidates.sort_by_key(|c| c.priority);

        let mut skipped = Vec::new();
        for cap in &candidates {
            if overrides.disabled_routes.contains(&cap.route) {
                skipped.push(SkippedPath {
                    route: cap.route,
                    reason: "disabled by tenant/account override".to_string(),
                });
                continue;
            }
            let ready = match cap.route {
                Route::Api => (pre.api_ready, &pre.api_block_reason),
                Route::Device => (pre.device_ready, &pre.device_block_reason),
                Route::Assisted => (pre.assisted_ready, &pre.assisted_block_reason),
            };
            if !ready.0 {
                skipped.push(SkippedPath {
                    route: cap.route,
                    reason: ready
                        .1
                        .clone()
                        .unwrap_or_else(|| "precondition not met".to_string()),
                });
                continue;
            }
            return Some(RouteDecision {
                platform: platform.to_string(),
                action: action.to_string(),
                reason: RouteReason {
                    selected: cap.route,
                    priority: cap.priority,
                    capability_status: cap.status,
                    skipped,
                },
            });
        }
        // everything selectable was skipped or no candidates at all
        if !skipped.is_empty() || !candidates.is_empty() {
            // represent "no usable route" as None; callers record skipped for audit
        }
        None
    }

    /// All skipped paths for a (platform, action) — for audit when selection fails.
    pub fn audit_skipped(
        &self,
        platform: &str,
        action: &str,
        overrides: &RoutingOverrides,
        pre: &Preconditions,
    ) -> Vec<SkippedPath> {
        let mut candidates = self.matrix.candidates(platform, action);
        candidates.retain(|c| c.selectable());
        candidates.sort_by_key(|c| c.priority);
        let mut skipped = Vec::new();
        for cap in &candidates {
            if overrides.disabled_routes.contains(&cap.route) {
                skipped.push(SkippedPath {
                    route: cap.route,
                    reason: "disabled by tenant/account override".to_string(),
                });
                continue;
            }
            let (ready, reason) = match cap.route {
                Route::Api => (pre.api_ready, &pre.api_block_reason),
                Route::Device => (pre.device_ready, &pre.device_block_reason),
                Route::Assisted => (pre.assisted_ready, &pre.assisted_block_reason),
            };
            if !ready {
                skipped.push(SkippedPath {
                    route: cap.route,
                    reason: reason
                        .clone()
                        .unwrap_or_else(|| "precondition not met".to_string()),
                });
            }
        }
        skipped
    }
}

/// Degradation decision after a failed attempt (TDD 7.2):
/// only NoSideEffectFailure may switch to the next route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// switch to the given next route
    Degrade(Route),
    /// result uncertain: VERIFICATION_PENDING, switching forbidden
    VerificationPending,
    /// hard failure, no more routes
    Failed,
}

pub fn fallback_decision(outcome: &crate::action::ExecOutcome, remaining: &[Route]) -> Fallback {
    match outcome {
        crate::action::ExecOutcome::NoSideEffectFailure(_) => match remaining.first() {
            Some(r) => Fallback::Degrade(*r),
            None => Fallback::Failed,
        },
        crate::action::ExecOutcome::Uncertain(_) => Fallback::VerificationPending,
        crate::action::ExecOutcome::Success { .. } => Fallback::Failed, // not called on success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MemMatrix {
        rows: Vec<ChannelCapability>,
    }
    impl CapabilityMatrix for MemMatrix {
        fn candidates(&self, platform: &str, action: &str) -> Vec<ChannelCapability> {
            self.rows
                .iter()
                .filter(|c| c.platform == platform && c.action == action)
                .cloned()
                .collect()
        }
    }

    fn cap(route: Route, status: CapabilityStatus, priority: i32) -> ChannelCapability {
        ChannelCapability {
            platform: "youtube".into(),
            action: "publish_video".into(),
            route,
            status,
            constraints: json!({}),
            priority,
            verified_at: None,
            evidence_ref: None,
        }
    }

    fn all_ready() -> Preconditions {
        Preconditions {
            api_ready: true,
            device_ready: true,
            assisted_ready: true,
            ..Default::default()
        }
    }

    #[test]
    fn picks_highest_priority_ready_route() {
        let m = MemMatrix {
            rows: vec![
                cap(Route::Assisted, CapabilityStatus::Verified, 30),
                cap(Route::Device, CapabilityStatus::Verified, 20),
                cap(Route::Api, CapabilityStatus::Verified, 10),
            ],
        };
        let r = Router::new(&m);
        let d = r
            .select(
                "youtube",
                "publish_video",
                &RoutingOverrides::default(),
                &all_ready(),
            )
            .expect("decision");
        assert!(matches!(d.reason.selected, Route::Api));
        assert_eq!(d.reason.skipped.len(), 0);
    }

    #[test]
    fn api_unavailable_falls_to_device_and_records_reason() {
        let m = MemMatrix {
            rows: vec![
                cap(Route::Api, CapabilityStatus::Verified, 10),
                cap(Route::Device, CapabilityStatus::Degraded, 20),
                cap(Route::Assisted, CapabilityStatus::Verified, 30),
            ],
        };
        let r = Router::new(&m);
        let pre = Preconditions {
            api_ready: false,
            api_block_reason: Some("quota exhausted".into()),
            device_ready: true,
            assisted_ready: true,
            ..Default::default()
        };
        let d = r
            .select(
                "youtube",
                "publish_video",
                &RoutingOverrides::default(),
                &pre,
            )
            .expect("decision");
        assert!(matches!(d.reason.selected, Route::Device));
        assert_eq!(d.reason.skipped.len(), 1);
        assert!(d.reason.skipped[0].reason.contains("quota exhausted"));
    }

    #[test]
    fn override_disables_device_route() {
        let m = MemMatrix {
            rows: vec![
                cap(Route::Api, CapabilityStatus::Verified, 10),
                cap(Route::Device, CapabilityStatus::Verified, 20),
            ],
        };
        let r = Router::new(&m);
        let ov = RoutingOverrides {
            disabled_routes: vec![Route::Device],
        };
        let mut pre = all_ready();
        pre.api_ready = false;
        pre.api_block_reason = Some("oauth expired".into());
        // device would be next but is overridden; nothing left => None
        assert!(r.select("youtube", "publish_video", &ov, &pre).is_none());
        let skipped = r.audit_skipped("youtube", "publish_video", &ov, &pre);
        assert!(
            skipped
                .iter()
                .any(|s| s.route == Route::Device && s.reason.contains("override"))
        );
    }

    #[test]
    fn unverified_and_unsupported_are_never_selected() {
        let m = MemMatrix {
            rows: vec![
                cap(Route::Api, CapabilityStatus::Unverified, 10),
                cap(Route::Device, CapabilityStatus::Unsupported, 20),
            ],
        };
        let r = Router::new(&m);
        assert!(
            r.select(
                "youtube",
                "publish_video",
                &RoutingOverrides::default(),
                &all_ready()
            )
            .is_none()
        );
    }

    #[test]
    fn x_publish_post_prefers_device_over_paid_api() {
        // PRD 5.3 + HID design doc: X posting does not use the paid API;
        // the device route (verified, priority 10) wins over the unverified
        // api route; with no active device it degrades to assisted.
        let xcap = |route: Route, status: CapabilityStatus, priority: i32| ChannelCapability {
            platform: "x".into(),
            action: "publish_post".into(),
            route,
            status,
            constraints: json!({}),
            priority,
            verified_at: None,
            evidence_ref: None,
        };
        let m = MemMatrix {
            rows: vec![
                xcap(Route::Api, CapabilityStatus::Unverified, 50),
                xcap(Route::Device, CapabilityStatus::Verified, 10),
                xcap(Route::Assisted, CapabilityStatus::Verified, 90),
            ],
        };
        let r = Router::new(&m);
        let d = r
            .select(
                "x",
                "publish_post",
                &RoutingOverrides::default(),
                &all_ready(),
            )
            .expect("device route selected");
        assert_eq!(d.reason.selected, Route::Device);
        let no_device = Preconditions {
            device_ready: false,
            ..all_ready()
        };
        let d = r
            .select(
                "x",
                "publish_post",
                &RoutingOverrides::default(),
                &no_device,
            )
            .expect("assisted fallback");
        assert_eq!(d.reason.selected, Route::Assisted);
    }

    #[test]
    fn no_side_effect_failure_degrades_but_uncertain_does_not() {
        let remaining = [Route::Device, Route::Assisted];
        let no_side = crate::action::ExecOutcome::NoSideEffectFailure(
            sincy_common::AppError::no_side_effect("x", "boom"),
        );
        assert_eq!(
            fallback_decision(&no_side, &remaining),
            Fallback::Degrade(Route::Device)
        );
        let uncertain = crate::action::ExecOutcome::Uncertain(sincy_common::AppError::uncertain(
            "y",
            "maybe published",
        ));
        assert_eq!(
            fallback_decision(&uncertain, &remaining),
            Fallback::VerificationPending
        );
        assert_eq!(fallback_decision(&no_side, &[]), Fallback::Failed);
    }
}
