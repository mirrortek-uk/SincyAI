//! Signed-adapter execution on device: verify signature -> version guard ->
//! run the sincy-adapter interpreter against the device's PlatformIo.

use sincy_adapter::StepOutcome;
use std::collections::VecDeque;

use sincy_adapter::interpreter::{
    ExecEvent, ExecStatus, Interpreter, MockSnapshot, PlatformIo, ScreenSnapshot,
};
use sincy_adapter::schema::{AdapterDefinition, FlowStep};
use sincy_adapter::sign::{SignatureScheme, verify_definition, verify_definition_scheme};
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
    execute_signed_adapter_ctx(
        adapter_yaml,
        signature,
        signing_key,
        app_version,
        io,
        media_count,
        None,
    )
}

/// 按签名方案验签: scheme=None 时回退 hmac (兼容旧 brief)。
pub fn verify_signed(
    canonical: &str,
    signature: &str,
    key_material: &[u8],
    scheme: Option<SignatureScheme>,
) -> Result<(), sincy_adapter::sign::SigningError> {
    match scheme {
        Some(sc) => verify_definition_scheme(sc, key_material, canonical, signature),
        None => verify_definition(key_material, canonical, signature),
    }
}

/// v2: 带签名方案的执行入口 (设备端公钥验签 ed25519, 旧 hmac 降级兼容)。
#[allow(clippy::too_many_arguments)]
pub fn execute_signed_adapter_ctx_v2(
    adapter_yaml: &str,
    signature: &str,
    signing_key: &[u8],
    signature_scheme: &str,
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
    task_context: Option<serde_json::Value>,
) -> Result<AdapterExecution, ExecuteError> {
    let scheme = SignatureScheme::parse(signature_scheme);
    let def = AdapterDefinition::from_yaml(adapter_yaml)
        .map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    let canonical =
        serde_json::to_string(&def).map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    verify_signed(&canonical, signature, signing_key, scheme)
        .map_err(|e| ExecuteError::BadSignature(e.to_string()))?;
    execute_verified(def, app_version, io, media_count, task_context)
}

/// Same as execute_signed_adapter, but allows a task context (e.g. agentic goal).
pub fn execute_signed_adapter_ctx(
    adapter_yaml: &str,
    signature: &str,
    signing_key: &[u8],
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
    task_context: Option<serde_json::Value>,
) -> Result<AdapterExecution, ExecuteError> {
    let def = AdapterDefinition::from_yaml(adapter_yaml)
        .map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    let canonical =
        serde_json::to_string(&def).map_err(|e| ExecuteError::InvalidDefinition(e.to_string()))?;
    verify_signed(&canonical, signature, signing_key, None)
        .map_err(|e| ExecuteError::BadSignature(e.to_string()))?;
    execute_verified(def, app_version, io, media_count, task_context)
}

/// 验签通过后的执行体 (v1/v2 共用)。
fn execute_verified(
    def: AdapterDefinition,
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
    task_context: Option<serde_json::Value>,
) -> Result<AdapterExecution, ExecuteError> {
    // 验签之后才注入内容包文本: flow 里 value == "package.text" 的占位符
    // 替换为 task_context.package_text (数据, 不参与签名)
    let mut def = def;
    if let Some(pkg) = task_context
        .as_ref()
        .and_then(|c| c.get("package_text").and_then(|v| v.as_str()))
    {
        for step in def.flow.iter_mut() {
            if step.value.as_deref() == Some("package.text") {
                step.value = Some(pkg.to_string());
            }
        }
    }

    let adapter_id = def.adapter.clone();
    let adapter_version = def.version as i32;

    // system.* adapters run native device ops (install/verify/launch), not the
    // screen interpreter; the version guard is skipped because the target app
    // may legitimately be absent (that is the point of a setup task).
    if adapter_id.starts_with("system.") {
        return run_system_adapter(&def, io, task_context);
    }

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
        // 副作用已发生后再漂移 => 不能当干净失败, 必须人工核验
        Some(ExecStatus::UiDrift(e)) if rt.side_effect_done => {
            TaskOutcome::Uncertain(format!("ui_drift after side effect: {e:?}"))
        }
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

/// Execute a signed `system.*` adapter as a fixed op sequence derived from its
/// (signed) params. Any op failure aborts with zero further side effects.
pub fn run_system_adapter(
    def: &AdapterDefinition,
    io: &mut dyn PlatformIo,
    task_context: Option<serde_json::Value>,
) -> Result<AdapterExecution, ExecuteError> {
    use sincy_adapter::interpreter::SystemOp;
    if def.adapter == "system.agentic" {
        return run_agentic_loop(def, io, task_context);
    }
    let params = def
        .params
        .clone()
        .ok_or_else(|| ExecuteError::InvalidDefinition("system adapter missing params".into()))?;
    let get = |k: &str| {
        params
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let target = get("target_package");
    let mut events: Vec<ExecEvent> = vec![];
    let ops: Vec<SystemOp> = match def.adapter.as_str() {
        // system.setup_install 及其按应用分裂的变体 (system.setup_install.x 等):
        // 参数 (download_url/sha256/target_package) 都来自签名定义, 行为完全一致
        a if a == "system.setup_install" || a.starts_with("system.setup_install.") => {
            let url = get("download_url");
            let sha256 = get("sha256");
            if url.is_empty() || sha256.is_empty() {
                return Err(ExecuteError::InvalidDefinition(
                    "setup_install params need download_url + sha256".into(),
                ));
            }
            let mut ops = vec![
                SystemOp::DownloadFile {
                    url,
                    sha256: sha256.clone(),
                },
                SystemOp::InstallApk,
                SystemOp::WaitInstalled {
                    package: target.clone(),
                    timeout_ms: 180_000,
                },
            ];
            let cert = get("signer_cert_sha256");
            if !cert.is_empty() {
                ops.push(SystemOp::VerifySigner {
                    package: target.clone(),
                    cert_sha256: cert,
                });
            }
            ops.push(SystemOp::LaunchApp {
                package: target.clone(),
            });
            ops
        }
        other => {
            return Err(ExecuteError::InvalidDefinition(format!(
                "unknown system adapter {other}"
            )));
        }
    };
    let mut side_effect_done = false;
    for op in &ops {
        let is_side_effect = matches!(op, SystemOp::InstallApk);
        match io.perform_system_op(op) {
            Ok(res) => {
                side_effect_done = side_effect_done || is_side_effect;
                events.push(ExecEvent::Step(StepOutcome {
                    step_index: events.len(),
                    screen_id: op_name(op).to_string(),
                    action: None,
                    side_effect: is_side_effect,
                    evidence: res,
                }));
            }
            Err(e) => {
                events.push(ExecEvent::Step(StepOutcome {
                    step_index: events.len(),
                    screen_id: op_name(op).to_string(),
                    action: None,
                    side_effect: is_side_effect,
                    evidence: format!("FAILED: {e}"),
                }));
                let outcome = if matches!(op, SystemOp::WaitInstalled { .. }) {
                    // 安装确认弹窗需要人手点击, 超时即转人工
                    TaskOutcome::WaitingUser(format!(
                        "waiting for user to confirm install dialog: {e}"
                    ))
                } else if side_effect_done {
                    TaskOutcome::Uncertain(e)
                } else {
                    TaskOutcome::NoSideEffectFailure(e)
                };
                return Ok(AdapterExecution {
                    outcome,
                    side_effect_done,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
        }
    }
    Ok(AdapterExecution {
        outcome: TaskOutcome::Success,
        side_effect_done,
        events,
        adapter_id: def.adapter.clone(),
        adapter_version: def.version as i32,
    })
}

/// system.agentic: device-side loop — every step sends a screen summary to the
/// cloud LLM for a decision; actions are restricted to the signed whitelist in
/// params; challenge screens always park as WAITING_USER.
fn run_agentic_loop(
    def: &AdapterDefinition,
    io: &mut dyn PlatformIo,
    task_context: Option<serde_json::Value>,
) -> Result<AdapterExecution, ExecuteError> {
    use sincy_adapter::interpreter::SystemOp;
    use sincy_adapter::schema::{ActionKind, Anchor, FlowStep};
    let params = def.params.clone().unwrap_or_default();
    let max_steps = params
        .get("max_steps")
        .and_then(|v| v.as_i64())
        .unwrap_or(30) as usize;
    let allowed: Vec<String> = params
        .get("allowed_actions")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "tap".into(),
                "tap_element".into(),
                "tap_xy".into(),
                "scroll".into(),
                "swipe_xy".into(),
                "set_text".into(),
                "back".into(),
                "close_dialog".into(),
                "wait".into(),
                "launch_app".into(),
            ]
        });
    let challenge_texts: Vec<String> = params
        .get("challenge_texts")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "验证".into(),
                "安全检测".into(),
                "unusual activity".into(),
                "Verify you are human".into(),
            ]
        });
    let goal = task_context
        .as_ref()
        .and_then(|c| c.get("goal").and_then(|g| g.as_str()).map(String::from))
        .or_else(|| {
            params
                .get("goal")
                .and_then(|g| g.as_str())
                .map(String::from)
        })
        .unwrap_or_default();
    if goal.is_empty() {
        return Err(ExecuteError::InvalidDefinition(
            "agentic task missing goal".into(),
        ));
    }

    let mut events: Vec<ExecEvent> = vec![];
    let mut history: Vec<String> = vec![];
    let mut prev_describe = String::new();
    let mut last_decision = String::new();
    let mut repeat_count = 0usize;
    for step in 1..=max_steps {
        let snap = io.snapshot();
        let describe = snap.describe();
        // challenge screens park for a human without relying on the LLM
        for ct in &challenge_texts {
            if snap.has_text(ct) {
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::WaitingUser(format!("challenge screen detected: {ct}")),
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
        }
        // 屏幕没变化时明确告诉 LLM, 防止它对着同一个目标点 30 次
        if !prev_describe.is_empty() && describe == prev_describe && step > 1 {
            history.push(format!(
                "NOTE: screen UNCHANGED after step {} — do not repeat the same action, pick a different target or scroll",
                step - 1
            ));
        }
        let elements = snap.elements_summary();
        let mut req = serde_json::json!({
            "goal": goal,
            "step": step,
            "max_steps": max_steps,
            "allowed_actions": allowed,
            "screen": describe,
            "elements": elements,
            "history": history,
        });
        if let Some(img) = snap.image_base64() {
            req["image"] = serde_json::Value::String(img);
        }
        if let Some(m) = params
            .get("model")
            .and_then(|v| v.as_str())
            .filter(|m| !m.is_empty())
        {
            req["model"] = serde_json::Value::String(m.to_string());
        }
        let decision = match io.llm_decide(&req) {
            Ok(d) => d,
            Err(e) => {
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::NoSideEffectFailure(format!("llm_decide failed: {e}")),
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
        };
        let action = decision
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("");
        // 连续 3 次完全相同的决策 => 死循环, 立即失败而不是烧完 max_steps
        let decision_str = decision.to_string();
        // 只有"决策相同 + 屏幕也没变化"才算死循环; 屏幕在动说明动作有效, 重置计数
        let screen_changed = !prev_describe.is_empty() && describe != prev_describe;
        if decision_str == last_decision
            && !screen_changed
            && action != "done"
            && action != "challenge"
        {
            repeat_count += 1;
            if repeat_count >= 3 {
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::NoSideEffectFailure(format!(
                        "loop detected at step {step}: identical LLM decision 3x ({decision_str})"
                    )),
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
        } else {
            repeat_count = 0;
        }
        last_decision = decision_str;
        match action {
            "done" => {
                events.push(ExecEvent::Completed);
                // done 可带 answer: 找帖/读帖任务用它把结果(帖子全文等)回传云端
                if let Some(ans) = decision.get("answer").and_then(|a| a.as_str())
                    && !ans.is_empty()
                {
                    events.push(ExecEvent::Step(StepOutcome {
                        step_index: step,
                        screen_id: "agentic".into(),
                        action: None,
                        side_effect: false,
                        evidence: format!("ANSWER: {ans}"),
                    }));
                }
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::Success,
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
            "challenge" => {
                let reason = decision
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("challenge")
                    .to_string();
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::WaitingUser(reason),
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
            a if allowed.iter().any(|x| x == a) => {
                // launch_app: recover when the OS dropped the target app to the launcher
                if a == "launch_app" {
                    let pkg = task_context
                        .as_ref()
                        .and_then(|c| c.get("app_package").and_then(|p| p.as_str()))
                        .or_else(|| params.get("target_package").and_then(|p| p.as_str()))
                        .unwrap_or("")
                        .to_string();
                    if pkg.is_empty() {
                        history.push(format!(
                            "step {step}: launch_app skipped: no target_package"
                        ));
                        continue;
                    }
                    match io.perform_system_op(&SystemOp::LaunchApp {
                        package: pkg.clone(),
                    }) {
                        Ok(_) => {
                            let _ = io.perform_system_op(&SystemOp::WaitMs { ms: 4000 });
                            let desc = format!("step {step}: launch_app {a} {pkg}");
                            history.push(desc.clone());
                            events.push(ExecEvent::Step(StepOutcome {
                                step_index: step,
                                screen_id: "agentic".into(),
                                action: None,
                                side_effect: false,
                                evidence: desc,
                            }));
                        }
                        Err(e) => {
                            history.push(format!("step {step}: launch_app failed: {e}"));
                        }
                    }
                    continue;
                }
                // tap_element: Mobilerun-style tap by numbered interactive element
                if a == "tap_element" {
                    let idx = decision
                        .get("element")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let snap2 = io.snapshot();
                    match snap2.element_bounds(idx as u32) {
                        Some((l, t, r, b)) => {
                            let step_def = FlowStep {
                                expect: "agentic".into(),
                                action: ActionKind::TapAt,
                                target: None,
                                value: Some(
                                    serde_json::json!({"x": (l + r) / 2, "y": (t + b) / 2})
                                        .to_string(),
                                ),
                                side_effect: false,
                                verify: false,
                            };
                            match io.perform(&step_def) {
                                Ok(()) => {
                                    let desc = format!(
                                        "step {step}: tap_element idx={idx} @ ({},{})",
                                        (l + r) / 2,
                                        (t + b) / 2
                                    );
                                    history.push(desc.clone());
                                    prev_describe = describe.clone();
                                    events.push(ExecEvent::Step(StepOutcome {
                                        step_index: step,
                                        screen_id: "agentic".into(),
                                        action: None,
                                        side_effect: false,
                                        evidence: desc,
                                    }));
                                }
                                Err(e) => {
                                    history.push(format!(
                                        "step {step}: tap_element idx={idx} failed: {e}"
                                    ));
                                }
                            }
                        }
                        None => {
                            history.push(format!(
                                "step {step}: tap_element idx={idx} not found in elements list — pick a valid idx or use tap_xy"
                            ));
                        }
                    }
                    continue;
                }
                let (kind, target, value) = match a {
                    "tap" => (
                        ActionKind::Tap,
                        decision
                            .get("target")
                            .cloned()
                            .and_then(|t| serde_json::from_value::<Anchor>(t).ok()),
                        None,
                    ),
                    "tap_xy" => (
                        ActionKind::TapAt,
                        None,
                        Some(
                            serde_json::json!({
                                "x": decision.get("x").and_then(|v| v.as_i64()).unwrap_or(0),
                                "y": decision.get("y").and_then(|v| v.as_i64()).unwrap_or(0),
                            })
                            .to_string(),
                        ),
                    ),
                    "swipe_xy" => (
                        ActionKind::SwipePath,
                        None,
                        Some(
                            serde_json::json!({
                                "x1": decision.get("x1").and_then(|v| v.as_i64()).unwrap_or(0),
                                "y1": decision.get("y1").and_then(|v| v.as_i64()).unwrap_or(0),
                                "x2": decision.get("x2").and_then(|v| v.as_i64()).unwrap_or(0),
                                "y2": decision.get("y2").and_then(|v| v.as_i64()).unwrap_or(0),
                                "ms": decision.get("ms").and_then(|v| v.as_u64()).unwrap_or(350),
                            })
                            .to_string(),
                        ),
                    ),
                    "wait" => (
                        ActionKind::Wait,
                        None,
                        Some(
                            decision
                                .get("ms")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(500)
                                .to_string(),
                        ),
                    ),
                    "set_text" => (
                        ActionKind::SetText,
                        decision
                            .get("target")
                            .cloned()
                            .and_then(|t| serde_json::from_value::<Anchor>(t).ok()),
                        decision
                            .get("value")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    ),
                    "scroll" => (ActionKind::Scroll, None, None),
                    "back" => (ActionKind::Back, None, None),
                    "close_dialog" => (ActionKind::CloseDialog, None, None),
                    _ => (ActionKind::Back, None, None),
                };
                let step_def = FlowStep {
                    expect: "agentic".into(),
                    action: kind,
                    target,
                    value,
                    side_effect: false,
                    verify: false,
                };
                let vision = snap.image_base64().is_some();
                match io.perform(&step_def) {
                    Ok(()) => {
                        let desc = format!("step {step}: {a} {} vision={vision}", decision);
                        history.push(desc.clone());
                        prev_describe = describe.clone();
                        events.push(ExecEvent::Step(StepOutcome {
                            step_index: step,
                            screen_id: "agentic".into(),
                            action: None,
                            side_effect: false,
                            evidence: desc,
                        }));
                    }
                    Err(e) => {
                        return Ok(AdapterExecution {
                            outcome: TaskOutcome::NoSideEffectFailure(format!(
                                "step {step} {a} failed: {e}"
                            )),
                            side_effect_done: false,
                            events,
                            adapter_id: def.adapter.clone(),
                            adapter_version: def.version as i32,
                        });
                    }
                }
            }
            other => {
                return Ok(AdapterExecution {
                    outcome: TaskOutcome::NoSideEffectFailure(format!(
                        "LLM action '{other}' not in signed whitelist {allowed:?}"
                    )),
                    side_effect_done: false,
                    events,
                    adapter_id: def.adapter.clone(),
                    adapter_version: def.version as i32,
                });
            }
        }
    }
    Ok(AdapterExecution {
        outcome: TaskOutcome::NoSideEffectFailure(format!(
            "agentic loop exceeded max_steps={max_steps}"
        )),
        side_effect_done: false,
        events,
        adapter_id: def.adapter.clone(),
        adapter_version: def.version as i32,
    })
}

fn op_name(op: &sincy_adapter::interpreter::SystemOp) -> &'static str {
    use sincy_adapter::interpreter::SystemOp;
    match op {
        SystemOp::DownloadFile { .. } => "download_file",
        SystemOp::InstallApk => "install_apk",
        SystemOp::WaitInstalled { .. } => "wait_installed",
        SystemOp::VerifySigner { .. } => "verify_signer",
        SystemOp::LaunchApp { .. } => "launch_app",
        SystemOp::WaitMs { .. } => "wait_ms",
    }
}

/// Scripted PlatformIo for the host simulator and tests: replays a fixed list
/// of screen snapshots and records performed actions.
pub struct ScriptedIo {
    pub screens: VecDeque<MockSnapshot>,
    pub performed: Vec<String>,
    pub system_ops: Vec<String>,
    /// scripted results for system ops; empty => all Ok("ok")
    pub system_results: VecDeque<Result<String, String>>,
    /// scripted LLM decisions for system.agentic; empty => immediate done
    pub llm_script: VecDeque<Result<serde_json::Value, String>>,
}

impl ScriptedIo {
    pub fn new(screens: Vec<MockSnapshot>) -> Self {
        Self {
            screens: screens.into(),
            performed: vec![],
            system_ops: vec![],
            system_results: VecDeque::new(),
            llm_script: VecDeque::new(),
        }
    }
}

impl PlatformIo for ScriptedIo {
    fn perform_system_op(
        &mut self,
        op: &sincy_adapter::interpreter::SystemOp,
    ) -> Result<String, String> {
        self.system_ops
            .push(serde_json::to_string(op).unwrap_or_default());
        self.system_results
            .pop_front()
            .unwrap_or_else(|| Ok("ok".to_string()))
    }

    fn llm_decide(&mut self, _req: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.llm_script
            .pop_front()
            .unwrap_or_else(|| Ok(serde_json::json!({"action": "done"})))
    }

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
        let ex = execute_signed_adapter(YAML, &sig, KEY, "9.9.9", &mut io, 0).unwrap();
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
