//! Content lifecycle (TDD 14.1) and device task lifecycle (TDD 14.2) with
//! guarded transitions. Exceptional states are first-class: PAUSED (kill
//! switch), BLOCKED, VERIFICATION_PENDING, WAITING_USER, QUOTA_WAIT, FAILED.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContentState {
    Discovered,
    Ranked,
    Planned,
    Creating,
    QualityCheck,
    PolicyCheck,
    PackageReady,
    Routing,
    QuotaWait,
    Queued,
    Executing,
    ExecutionResultReceived,
    Published,
    Analytics,
    Learned,
    // exceptional
    Paused,
    Blocked,
    VerificationPending,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeviceTaskState {
    Received,
    Validating,
    Downloading,
    Preparing,
    AdapterCheck,
    ExecutingSteps,
    Verifying,
    Completed,
    // exceptional
    WaitingAuth,
    WaitingPermission,
    WaitingDevice,
    WaitingNetwork,
    WaitingUser,
    UiDriftDetected,
    VerificationPending,
    Failed,
    Blocked,
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
#[error("illegal transition {from:?} -> {to:?} via {event}")]
pub struct IllegalTransition {
    pub from: String,
    pub to: String,
    pub event: String,
}

/// A recorded state transition event (mirrors task_events rows).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub event: String,
    pub from: String,
    pub to: String,
    pub detail: serde_json::Value,
}

/// Guarded state machine: only whitelisted edges are allowed.
pub struct StateMachine<S: Copy + PartialEq + std::fmt::Debug> {
    pub state: S,
    /// true once the task crossed its side_effect step
    pub side_effect_done: bool,
}

/// Allowed edges for the Cloud content lifecycle (TDD 14.1).
fn content_edges() -> &'static [(&'static str, &'static str)] {
    &[
        ("DISCOVERED", "RANKED"),
        ("RANKED", "PLANNED"),
        ("PLANNED", "CREATING"),
        ("CREATING", "QUALITY_CHECK"),
        ("QUALITY_CHECK", "POLICY_CHECK"),
        ("POLICY_CHECK", "PACKAGE_READY"),
        ("PACKAGE_READY", "ROUTING"),
        ("ROUTING", "QUOTA_WAIT"),
        ("ROUTING", "QUEUED"),
        ("QUOTA_WAIT", "QUEUED"),
        ("QUEUED", "EXECUTING"),
        ("EXECUTING", "EXECUTION_RESULT_RECEIVED"),
        ("EXECUTION_RESULT_RECEIVED", "PUBLISHED"),
        ("EXECUTION_RESULT_RECEIVED", "VERIFICATION_PENDING"),
        ("PUBLISHED", "ANALYTICS"),
        ("ANALYTICS", "LEARNED"),
    ]
}

/// Exceptional transitions allowed from any non-final state.
fn exceptional_targets() -> &'static [&'static str] {
    &[
        "PAUSED",
        "BLOCKED",
        "VERIFICATION_PENDING",
        "FAILED",
        "QUOTA_WAIT",
    ]
}

/// Allowed edges for the device task lifecycle (TDD 14.2).
fn device_edges() -> &'static [(&'static str, &'static str)] {
    &[
        ("RECEIVED", "VALIDATING"),
        ("VALIDATING", "DOWNLOADING"),
        ("DOWNLOADING", "PREPARING"),
        ("PREPARING", "ADAPTER_CHECK"),
        ("ADAPTER_CHECK", "EXECUTING_STEPS"),
        ("EXECUTING_STEPS", "VERIFYING"),
        ("VERIFYING", "COMPLETED"),
        ("EXECUTING_STEPS", "VERIFICATION_PENDING"),
        ("VERIFYING", "VERIFICATION_PENDING"),
        // human resolved a WAITING_USER challenge: resume the flow
        ("WAITING_USER", "EXECUTING_STEPS"),
    ]
}

fn device_exceptional() -> &'static [&'static str] {
    &[
        "WAITING_AUTH",
        "WAITING_PERMISSION",
        "WAITING_DEVICE",
        "WAITING_NETWORK",
        "WAITING_USER",
        "UI_DRIFT_DETECTED",
        "VERIFICATION_PENDING",
        "FAILED",
        "BLOCKED",
        "CANCELLED",
    ]
}

/// CamelCase -> SCREAMING_SNAKE_CASE ("QualityCheck" -> "QUALITY_CHECK")
fn name_of<S: Copy + PartialEq + std::fmt::Debug>(s: S) -> String {
    let mut out = String::new();
    for (i, c) in format!("{s:?}").chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

impl StateMachine<ContentState> {
    pub fn new_content() -> Self {
        Self {
            state: ContentState::Discovered,
            side_effect_done: false,
        }
    }

    pub fn transition(
        &mut self,
        event: &str,
        to: ContentState,
    ) -> Result<TaskEvent, IllegalTransition> {
        let from = name_of(self.state);
        let to_s = name_of(to);
        let ok = content_edges().contains(&(from.as_str(), to_s.as_str()))
            || (exceptional_targets().contains(&to_s.as_str())
                && !matches!(
                    self.state,
                    ContentState::Published | ContentState::Analytics | ContentState::Learned
                ));
        if !ok {
            return Err(IllegalTransition {
                from,
                to: to_s,
                event: event.to_string(),
            });
        }
        // side_effect boundary: EXECUTING is where external副作用 happens
        if to == ContentState::Executing {
            self.side_effect_done = true;
        }
        let ev = TaskEvent {
            event: event.to_string(),
            from: from.clone(),
            to: to_s.clone(),
            detail: serde_json::json!({}),
        };
        self.state = to;
        Ok(ev)
    }
}

impl StateMachine<DeviceTaskState> {
    pub fn new_device_task() -> Self {
        Self {
            state: DeviceTaskState::Received,
            side_effect_done: false,
        }
    }

    pub fn transition(
        &mut self,
        event: &str,
        to: DeviceTaskState,
    ) -> Result<TaskEvent, IllegalTransition> {
        let from = name_of(self.state);
        let to_s = name_of(to);
        let ok = device_edges().contains(&(from.as_str(), to_s.as_str()))
            || device_exceptional().contains(&to_s.as_str());
        if !ok {
            return Err(IllegalTransition {
                from,
                to: to_s,
                event: event.to_string(),
            });
        }
        if to == DeviceTaskState::ExecutingSteps {
            // steps may include side_effect steps; router marks side_effect_done
            // when the adapter reports the side_effect step executed.
        }
        let ev = TaskEvent {
            event: event.to_string(),
            from: from.clone(),
            to: to_s.clone(),
            detail: serde_json::json!({}),
        };
        self.state = to;
        Ok(ev)
    }

    /// Can this task be auto-retried right now? Only before any side effect.
    pub fn can_auto_retry(&self) -> bool {
        !self.side_effect_done
            && !matches!(
                self.state,
                DeviceTaskState::Completed
                    | DeviceTaskState::Cancelled
                    | DeviceTaskState::VerificationPending
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_happy_path() {
        let mut sm = StateMachine::new_content();
        let seq = [
            ("rank", ContentState::Ranked),
            ("plan", ContentState::Planned),
            ("create", ContentState::Creating),
            ("quality", ContentState::QualityCheck),
            ("policy", ContentState::PolicyCheck),
            ("package", ContentState::PackageReady),
            ("route", ContentState::Routing),
            ("queue", ContentState::Queued),
            ("execute", ContentState::Executing),
            ("result", ContentState::ExecutionResultReceived),
            ("publish", ContentState::Published),
            ("analyze", ContentState::Analytics),
            ("learn", ContentState::Learned),
        ];
        for (ev, to) in seq {
            sm.transition(ev, to).unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(sm.side_effect_done);
    }

    #[test]
    fn illegal_transition_rejected() {
        let mut sm = StateMachine::new_content();
        let err = sm
            .transition("publish!", ContentState::Published)
            .unwrap_err();
        assert_eq!(err.from, "DISCOVERED");
        assert_eq!(err.to, "PUBLISHED");
    }

    #[test]
    fn kill_switch_pause_from_any_active_state() {
        let mut sm = StateMachine::new_content();
        for (ev, to) in [
            ("rank", ContentState::Ranked),
            ("plan", ContentState::Planned),
            ("create", ContentState::Creating),
            ("quality", ContentState::QualityCheck),
            ("policy", ContentState::PolicyCheck),
            ("package", ContentState::PackageReady),
            ("route", ContentState::Routing),
            ("queue", ContentState::Queued),
        ] {
            sm.transition(ev, to).expect("ok");
        }
        sm.transition("kill", ContentState::Paused)
            .expect("pause ok");
        assert_eq!(sm.state, ContentState::Paused);
    }

    #[test]
    fn device_task_side_effect_blocks_auto_retry() {
        let mut sm = StateMachine::new_device_task();
        assert!(sm.can_auto_retry());
        for (ev, to) in [
            ("validate", DeviceTaskState::Validating),
            ("download", DeviceTaskState::Downloading),
            ("prepare", DeviceTaskState::Preparing),
            ("adapter", DeviceTaskState::AdapterCheck),
            ("steps", DeviceTaskState::ExecutingSteps),
        ] {
            sm.transition(ev, to).expect("ok");
        }
        sm.side_effect_done = true; // adapter reported side_effect step done
        assert!(
            !sm.can_auto_retry(),
            "after side_effect, no auto retry/degrade"
        );
        sm.transition("uncertain", DeviceTaskState::VerificationPending)
            .expect("ok");
        assert!(!sm.can_auto_retry());
    }

    #[test]
    fn challenge_goes_to_waiting_user() {
        let mut sm = StateMachine::new_device_task();
        for (ev, to) in [
            ("validate", DeviceTaskState::Validating),
            ("download", DeviceTaskState::Downloading),
            ("prepare", DeviceTaskState::Preparing),
            ("adapter", DeviceTaskState::AdapterCheck),
            ("steps", DeviceTaskState::ExecutingSteps),
        ] {
            sm.transition(ev, to).expect("ok");
        }
        // captcha / security challenge detected mid-flow => WAITING_USER
        sm.transition("challenge.detected", DeviceTaskState::WaitingUser)
            .expect("waiting_user");
        assert_eq!(sm.state, DeviceTaskState::WaitingUser);
        // human handled it: back to executing steps
        sm.transition("user.resolved", DeviceTaskState::ExecutingSteps)
            .expect("resume");
    }
}
