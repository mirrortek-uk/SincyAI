//! Signed-adapter execution on device: verify signature -> version guard ->
//! run the sincy-adapter interpreter against the device's PlatformIo.

use std::collections::VecDeque;

use sincy_adapter::interpreter::{
    ExecEvent, ExecStatus, Interpreter, MockSnapshot, PlatformIo, ScreenSnapshot,
};
use sincy_adapter::schema::{AdapterDefinition, FlowStep};
use sincy_adapter::sign::verify_definition;
use sincy_adapter::version_range::VersionRange;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecuteError {
    #[error("adapter definition invalid: {0}")]
    InvalidDefinition(String),
    #[error("signature invalid: {0}")]
    BadSignature(String),
    #[error("version_range invalid: {0}")]
    BadVersionRange(String),
}

/// Outcome mapped onto the gateway protocol (device.task.result "outcome").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    /// Posted/published and verified on screen.
    Success,
    /// Failed before the side-effect boundary; safe to requeue/degrade.
    NoSideEffectFailure(String),
    /// Side effect may have happened; human must verify. Never auto-retry.
    Uncertain(String),
    /// Challenge screen (captcha/security) — park for a human.
    WaitingUser(String),
    /// Adapter/app mismatch — block, alert, mark adapter degraded.
    UiDrift(String),
    /// Signature/version refusal — zero side effects happened.
    Refused(String),
}

impl TaskOutcome {
    /// String used in the DeviceTaskResult envelope; gateway maps
    /// success/no_side_effect_failure/uncertain, anything else => failed.
    pub fn as_protocol(&self) -> &'static str {
        match self {
            TaskOutcome::Success => "success",
            TaskOutcome::NoSideEffectFailure(_) | TaskOutcome::Refused(_) => {
                "no_side_effect_failure"
            }
            TaskOutcome::Uncertain(_) => "uncertain",
            // WaitingUser and UiDrift are reported via their dedicated envelopes
            // (challenge.detected / ui_drift.detected) plus a result for the audit.
            TaskOutcome::WaitingUser(_) | TaskOutcome::UiDrift(_) => "no_side_effect_failure",
        }
    }
}

#[derive(Debug)]
pub struct AdapterExecution {
    pub outcome: TaskOutcome,
    pub side_effect_done: bool,
    pub events: Vec<ExecEvent>,
    pub adapter_id: String,
    pub adapter_version: i32,
}

/// Verify + execute a signed adapter definition (YAML) against the device IO.
///
/// Order matters and is part of the safety contract:
/// 1. parse + schema-validate the definition;
/// 2. verify the HMAC signature over the canonical JSON — refusal here means
///    the interpreter never runs (zero side effects);
/// 3. version_range guard — out of range => UiDrift, still zero side effects;
/// 4. only then run the interpreter.
pub fn execute_signed_adapter(
    adapter_yaml: &str,
    signature: &str,
    signing_key: &[u8],
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
) -> Result<AdapterExecution, ExecuteError> {
    let def = AdapterDefinition::from_yaml(adapter_yaml)
        .map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    let canonical =
        serde_json::to_string(&def).map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    verify_definition(signing_key, &canonical, signature)
        .map_err(|e| ExecuteError::BadSignature(e.to_string()))?;

    let adapter_id = def.adapter.clone();
    let adapter_version = def.version as i32;

    // version guard BEFORE any interaction with the target app
    let range = VersionRange::parse(&def.app.version_range)
        .map_err(|e| ExecuteError::BadVersionRange(e.to_string()))?;
    let in_range = range
        .contains(app_version)
        .map_err(|e| ExecuteError::BadVersionRange(e.to_string()))?;
    if !in_range {
        return Ok(AdapterExecution {
            outcome: TaskOutcome::UiDrift(format!(
                "app version {app_version} outside adapter range {}",
                def.app.version_range
            )),
            side_effect_done: false,
            events: vec![],
            adapter_id,
            adapter_version,
        });
    }

    let mut interp = Interpreter::new(&def, app_version);
    let rt = interp.run(io, media_count);
    let outcome = match rt.status {
        Some(ExecStatus::Completed) => TaskOutcome::Success,
        Some(ExecStatus::VerificationPending(e)) => TaskOutcome::Uncertain(e),
        Some(ExecStatus::WaitingUser(e)) => TaskOutcome::WaitingUser(e),
        Some(ExecStatus::UiDrift(e)) => TaskOutcome::UiDrift(format!("{e:?}")),
        Some(ExecStatus::Failed(e)) => {
            if rt.side_effect_done {
                TaskOutcome::Uncertain(e)
            } else {
                TaskOutcome::NoSideEffectFailure(e)
            }
        }
        None => TaskOutcome::NoSideEffectFailure("interpreter produced no status".into()),
    };
    Ok(AdapterExecution {
        outcome,
        side_effect_done: rt.side_effect_done,
        events: rt.events,
        adapter_id,
        adapter_version,
    })
}

/// Scripted PlatformIo for the host simulator and tests: replays a fixed list
/// of screen snapshots and records performed actions.
pub struct ScriptedIo {
    pub screens: VecDeque<MockSnapshot>,
    pub performed: Vec<String>,
}

impl ScriptedIo {
    pub fn new(screens: Vec<MockSnapshot>) -> Self {
        Self {
            screens: screens.into(),
            performed: vec![],
        }
    }
}

impl PlatformIo for ScriptedIo {
    fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
        let s = self
            .screens
            .pop_front()
            .unwrap_or_else(|| MockSnapshot::new(vec![], vec![]));
        Box::new(s)
    }
    fn perform(&mut self, step: &FlowStep) -> Result<(), String> {
        self.performed.push(format!("{:?}", step.action));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sincy_adapter::sign::sign_definition;

    const YAML: &str = include_str!("../../../adapters/x.publish_post.yaml");
    const KEY: &[u8] = b"sincy-dev-adapter-signing-key";

    fn signed() -> (String, String) {
        let def = AdapterDefinition::from_yaml(YAML).unwrap();
        let canonical = serde_json::to_string(&def).unwrap();
        (canonical.clone(), sign_definition(KEY, &canonical))
    }

    fn screens() -> Vec<MockSnapshot> {
        vec![
            MockSnapshot::new(
                vec!["com.twitter.android:id/bottom_bar".into()],
                vec!["Home".into()],
            ),
            MockSnapshot::new(
                vec!["com.twitter.android:id/tweet_text".into()],
                vec!["What's happening?".into()],
            ),
            MockSnapshot::new(
                vec!["com.twitter.android:id/tweet_text".into()],
                vec!["What's happening?".into()],
            ),
            MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
            MockSnapshot::new(vec![], vec!["Your post was sent".into()]),
        ]
    }

    #[test]
    fn happy_path_success_with_side_effect() {
        let (_, sig) = signed();
        let mut io = ScriptedIo::new(screens());
        let ex = execute_signed_adapter(YAML, &sig, KEY, "10.5.0", &mut io, 0).unwrap();
        assert_eq!(ex.outcome, TaskOutcome::Success);
        assert!(ex.side_effect_done);
        assert_eq!(ex.adapter_id, "x.publish_post");
    }

    #[test]
    fn tampered_signature_refused_with_zero_side_effects() {
        let mut io = ScriptedIo::new(screens());
        let ex = execute_signed_adapter(YAML, "deadbeef", KEY, "10.5.0", &mut io, 0)
            .expect_err("must refuse");
        assert!(matches!(ex, ExecuteError::BadSignature(_)));
        assert!(
            io.performed.is_empty(),
            "no action may run on bad signature"
        );
    }

    #[test]
    fn version_out_of_range_is_drift_zero_side_effects() {
        let (_, sig) = signed();
        let mut io = ScriptedIo::new(screens());
        let ex = execute_signed_adapter(YAML, &sig, KEY, "11.9.0", &mut io, 0).unwrap();
        assert!(matches!(ex.outcome, TaskOutcome::UiDrift(_)));
        assert!(!ex.side_effect_done);
        assert!(io.performed.is_empty());
    }

    #[test]
    fn challenge_screen_parks_for_human() {
        let (_, sig) = signed();
        let mut screens = screens();
        // third snapshot (before Post tap) is a challenge instead
        screens[2] = MockSnapshot::new(vec![], vec!["Verify you are human".into()]);
        let mut io = ScriptedIo::new(screens);
        let ex = execute_signed_adapter(YAML, &sig, KEY, "10.5.0", &mut io, 0).unwrap();
        assert!(matches!(ex.outcome, TaskOutcome::WaitingUser(_)));
        // the Post tap (the side_effect step, index 2) must never have run:
        // only step 0 (open composer) and step 1 (set text) may be performed
        assert!(!ex.side_effect_done);
        assert_eq!(
            io.performed.len(),
            2,
            "side-effect tap must not run on challenge"
        );
    }
}
