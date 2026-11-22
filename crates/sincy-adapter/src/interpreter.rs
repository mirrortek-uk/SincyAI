//! Adapter interpreter: the TDD 9.1 execution loop, platform-neutral.
//! The Android runtime supplies a ScreenSnapshot (accessibility tree + OCR)
//! and an action sink; this crate drives the loop and the safety guards:
//! challenge screens => WAITING_USER; unknown screens => whitelisted safe
//! recovery at most N times then WAITING_USER; side_effect steps are tracked
//! so the caller can forbid retry/degrade afterwards.

use std::collections::BTreeMap as Map;
#[allow(unused_imports)]
use std::collections::HashMap;

use crate::drift::{DriftVerdict, anchor_match_rate, judge};
use crate::schema::{ActionKind, AdapterDefinition, Anchor, FlowStep, ScreenDef};
use crate::version_range::VersionRange;

/// A platform-neutral view of the current screen.
pub trait ScreenSnapshot {
    fn has_node_with_id(&self, id: &str) -> bool;
    fn has_text(&self, text: &str) -> bool;
}

/// Test/mock snapshot.
pub struct MockSnapshot {
    node_ids: Vec<String>,
    texts: Vec<String>,
}

impl MockSnapshot {
    pub fn new(node_ids: Vec<String>, texts: Vec<String>) -> Self {
        Self { node_ids, texts }
    }
}

impl ScreenSnapshot for MockSnapshot {
    fn has_node_with_id(&self, id: &str) -> bool {
        self.node_ids.iter().any(|n| n == id)
    }
    fn has_text(&self, text: &str) -> bool {
        self.texts.iter().any(|t| t == text)
    }
}

/// Platform I/O the device runtime provides.
pub trait PlatformIo {
    fn snapshot(&mut self) -> Box<dyn ScreenSnapshot>;
    /// Perform a step action (tap / set text / pick media / back ...).
    fn perform(&mut self, step: &FlowStep) -> Result<(), String>;
    /// Execute one native system op (system.* adapters only: download+verify,
    /// fire install intent, poll installed, verify signer, launch app).
    /// Default refuses so a runtime that cannot do system ops fails closed.
    /// Returns a short result string ("ok" / details) on success.
    fn perform_system_op(&mut self, op: &SystemOp) -> Result<String, String> {
        let _ = op;
        Err("system ops unsupported on this io".into())
    }
}

/// Native device operations for signed `system.*` adapters.
/// Params come from the signed adapter definition, so they are tamper-proof.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SystemOp {
    /// 下载文件到设备私有目录并校验 SHA-256; 不匹配必须失败。
    DownloadFile {
        url: String,
        sha256: String,
    },
    /// 调起系统安装器 (人手确认弹窗是 Android 硬要求, 不可跳过)。
    InstallApk,
    /// 轮询直到目标包安装完成或超时。
    WaitInstalled {
        package: String,
        timeout_ms: u64,
    },
    /// 校验已安装应用的签名证书 SHA-256 (防篡改替换)。
    VerifySigner {
        package: String,
        cert_sha256: String,
    },
    /// 启动目标 app。
    LaunchApp {
        package: String,
    },
    WaitMs {
        ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Tap(Anchor),
    SetText { target: Anchor, value: String },
    PickMedia { index: usize },
    Scroll,
    Back,
    CloseDialog,
}

impl FlowStep {
    pub fn to_action(&self, media_count: usize) -> Option<Action> {
        match self.action {
            ActionKind::Tap => self.target.clone().map(Action::Tap),
            ActionKind::SetText => match (&self.target, &self.value) {
                (Some(t), Some(v)) => Some(Action::SetText {
                    target: t.clone(),
                    value: v.clone(),
                }),
                _ => None,
            },
            ActionKind::PickMedia => Some(Action::PickMedia { index: media_count }),
            ActionKind::Scroll => Some(Action::Scroll),
            ActionKind::Back => Some(Action::Back),
            ActionKind::CloseDialog => Some(Action::CloseDialog),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub step_index: usize,
    pub screen_id: String,
    pub action: Option<Action>,
    pub side_effect: bool,
    pub evidence: String, // redacted refs only; never raw screenshots here
}

#[derive(Debug, Clone)]
pub enum ExecEvent {
    Step(StepOutcome),
    ChallengeDetected {
        screen_id: String,
    },
    UnknownScreen {
        screen_id: String,
        recoveries_left: u32,
    },
    UiDrift(DriftVerdict),
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecStatus {
    Completed,
    /// task must park for a human (challenge / unknown screen exhausted)
    WaitingUser(String),
    /// adapter/app mismatch: block + alert, no blind taps
    UiDrift(String),
    /// side effect may have happened; caller must verify before anything else
    VerificationPending(String),
    Failed(String),
}

/// Why a screen matched.
#[derive(Debug, Clone)]
pub struct ScreenMatch {
    pub screen_id: String,
    pub match_rate: f64,
}

pub fn identify_screen(
    snap: &dyn ScreenSnapshot,
    screens: &Map<String, ScreenDef>,
    challenge: Option<&str>,
) -> ScreenMatch {
    // challenge screen wins first (safety)
    if let Some(ch) = challenge
        && let Some(def) = screens.get(ch)
        && def
            .anchors
            .iter()
            .all(|a| crate::drift::anchor_matches(a, snap))
    {
        return ScreenMatch {
            screen_id: ch.to_string(),
            match_rate: anchor_match_rate(&def.anchors, snap),
        };
    }
    let mut best: Option<ScreenMatch> = None;
    for (name, def) in screens {
        if Some(name.as_str()) == challenge {
            continue;
        }
        if !def.anchors.is_empty()
            && def
                .anchors
                .iter()
                .all(|a| crate::drift::anchor_matches(a, snap))
        {
            let rate = anchor_match_rate(&def.anchors, snap);
            if best.as_ref().map(|b| rate > b.match_rate).unwrap_or(true) {
                best = Some(ScreenMatch {
                    screen_id: name.clone(),
                    match_rate: rate,
                });
            }
        }
    }
    best.unwrap_or(ScreenMatch {
        screen_id: "UNKNOWN".to_string(),
        match_rate: 0.0,
    })
}

/// The interpreter. Owns the loop state; the device runtime calls `run`.
pub struct Interpreter<'a> {
    pub def: &'a AdapterDefinition,
    pub app_version: &'a str,
    pub max_safe_recoveries: u32,
}

#[derive(Debug, Clone)]
pub struct AdapterRuntime {
    pub side_effect_done: bool,
    pub events: Vec<ExecEvent>,
    pub status: Option<ExecStatus>,
}

impl<'a> Interpreter<'a> {
    pub fn new(def: &'a AdapterDefinition, app_version: &'a str) -> Self {
        Self {
            def,
            app_version,
            max_safe_recoveries: 3,
        }
    }

    /// Check the App version against the adapter range (drift guard).
    pub fn version_ok(&self) -> Result<bool, crate::version_range::VersionRangeError> {
        VersionRange::parse(&self.def.app.version_range)?.contains(self.app_version)
    }

    /// Run the whole flow against a platform I/O. Returns the final status.
    pub fn run(&mut self, io: &mut dyn PlatformIo, media_count: usize) -> AdapterRuntime {
        let mut rt = AdapterRuntime {
            side_effect_done: false,
            events: Vec::new(),
            status: None,
        };
        // version guard first: out of range => block, no blind taps
        match self.version_ok() {
            Ok(true) => {}
            Ok(false) => {
                let v = judge(1.0, 0, false);
                rt.events.push(ExecEvent::UiDrift(v.clone()));
                rt.status = Some(ExecStatus::UiDrift(format!(
                    "app version {} out of range {}",
                    self.app_version, self.def.app.version_range
                )));
                return rt;
            }
            Err(e) => {
                rt.status = Some(ExecStatus::Failed(format!("bad version_range: {e}")));
                return rt;
            }
        }

        let challenge = self.def.challenge_screen().map(|s| s.to_string());
        let mut recoveries_left = self.max_safe_recoveries;
        let mut consecutive_timeouts = 0u32;

        for (i, step) in self.def.flow.iter().enumerate() {
            // OBSERVE -> IDENTIFY -> GUARD: re-observe within the SAME step until
            // the expected screen appears, a challenge appears, or safe
            // recovery is exhausted.
            let m = loop {
                let snap = io.snapshot();
                let m = identify_screen(&*snap, &self.def.screens, challenge.as_deref());
                // challenge always wins
                if Some(m.screen_id.as_str()) == challenge.as_deref() {
                    rt.events.push(ExecEvent::ChallengeDetected {
                        screen_id: m.screen_id.clone(),
                    });
                    rt.status = Some(ExecStatus::WaitingUser(format!(
                        "challenge screen {} at step {i}",
                        m.screen_id
                    )));
                    return rt;
                }
                if m.screen_id == step.expect {
                    break m;
                }
                if m.screen_id == "UNKNOWN" {
                    // whitelisted safe recovery first: only Back / CloseDialog
                    if recoveries_left > 0 {
                        recoveries_left -= 1;
                        rt.events.push(ExecEvent::UnknownScreen {
                            screen_id: m.screen_id.clone(),
                            recoveries_left,
                        });
                        let back = FlowStep {
                            expect: step.expect.clone(),
                            action: ActionKind::Back,
                            target: None,
                            value: None,
                            side_effect: false,
                            verify: false,
                        };
                        if io.perform(&back).is_err() {
                            consecutive_timeouts += 1;
                        }
                        continue; // re-observe within the SAME step
                    }
                    // safe recoveries exhausted: now judge as drift
                    if let Some(expected) = self.def.screens.get(&step.expect) {
                        let rate = anchor_match_rate(&expected.anchors, &*snap);
                        let verdict = judge(rate, consecutive_timeouts, true);
                        if verdict.drifted {
                            rt.events.push(ExecEvent::UiDrift(verdict.clone()));
                            rt.status = Some(ExecStatus::UiDrift(format!(
                                "{:?} at step {i} (anchor rate {:.2})",
                                verdict.reason, verdict.anchor_match_rate
                            )));
                            return rt;
                        }
                    }
                    rt.status = Some(ExecStatus::WaitingUser(format!(
                        "unknown screen at step {i}, safe recoveries exhausted"
                    )));
                    return rt;
                }
                // known interrupt?
                if let Some(intr) = self
                    .def
                    .known_interrupts
                    .iter()
                    .find(|intr| intr.screen == m.screen_id)
                {
                    let fake = FlowStep {
                        expect: intr.screen.clone(),
                        action: intr.action,
                        target: Some(intr.target.clone()),
                        value: None,
                        side_effect: false,
                        verify: false,
                    };
                    let _ = io.perform(&fake);
                    continue; // re-observe within the SAME step
                }
                // recognized but unexpected screen: block, no blind taps
                rt.status = Some(ExecStatus::Failed(format!(
                    "expected screen {} but identified {} at step {i}",
                    step.expect, m.screen_id
                )));
                return rt;
            };

            // ACT
            if let Some(action) = step.to_action(media_count) {
                if step.side_effect {
                    // side-effect boundary: after this, results may be uncertain
                    rt.side_effect_done = true;
                }
                if let Err(e) = io.perform(step) {
                    if step.side_effect {
                        rt.status = Some(ExecStatus::VerificationPending(format!(
                            "side_effect step {i} failed after execution: {e}"
                        )));
                        return rt;
                    }
                    rt.status = Some(ExecStatus::Failed(format!("step {i} failed: {e}")));
                    return rt;
                }
                rt.events.push(ExecEvent::Step(StepOutcome {
                    step_index: i,
                    screen_id: m.screen_id.clone(),
                    action: Some(action),
                    side_effect: step.side_effect,
                    evidence: format!("step={i} screen={} se={}", m.screen_id, step.side_effect),
                }));
            }

            // VERIFY: post-condition — next observe must not be UNKNOWN/challenge
            if step.verify {
                let snap2 = io.snapshot();
                let m2 = identify_screen(&*snap2, &self.def.screens, challenge.as_deref());
                if Some(m2.screen_id.as_str()) == challenge.as_deref() {
                    rt.events.push(ExecEvent::ChallengeDetected {
                        screen_id: m2.screen_id.clone(),
                    });
                    rt.status = Some(ExecStatus::WaitingUser(format!(
                        "challenge after side_effect step {i}"
                    )));
                    return rt;
                }
                if m2.screen_id == "UNKNOWN" {
                    rt.status = Some(ExecStatus::VerificationPending(format!(
                        "post-condition of step {i} not met"
                    )));
                    return rt;
                }
            }
        }
        rt.events.push(ExecEvent::Completed);
        rt.status = Some(ExecStatus::Completed);
        rt
    }
}
