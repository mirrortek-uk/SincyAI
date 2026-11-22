//! Rust core for the Android agent, exposed to Kotlin over JNI (ADR-002).
//!
//! All policy logic lives in `sincy-agent` (crates/sincy-agent) and is verified
//! by host tests; this crate only adapts it to JNI + the Android accessibility
//! bridge. Device behavior therefore cannot diverge from server policy.
//!
//! Safety contract (HID design doc red lines), enforced in sincy-agent:
//! signature must verify under the pinned platform key, version_range must
//! cover the target app (else ui_drift with zero side effects), challenge
//! screens are never auto-solved (WAITING_USER).

use sincy_adapter::interpreter::PlatformIo;
use sincy_agent::{execute_signed_adapter, AdapterExecution, TaskOutcome};

/// Pinned platform adapter-signing key. In production builds this is compiled
/// in from a build-time secret (not from network config); the dev build uses
/// the local dev key, matching migrations/0015_seed_x_adapter.sql.
#[cfg(debug_assertions)]
pub const PINNED_SIGNING_KEY: &[u8] = b"sincy-dev-adapter-signing-key";
#[cfg(not(debug_assertions))]
pub const PINNED_SIGNING_KEY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../keys/adapter_signing_key.bin"
));

/// JSON result envelope returned to Kotlin. Shape is part of the JNI contract.
#[derive(Debug, serde::Serialize)]
pub struct RunResult {
    pub outcome: String,
    pub side_effect_done: bool,
    pub adapter_id: String,
    pub adapter_version: i32,
    pub events: Vec<String>,
}

impl From<AdapterExecution> for RunResult {
    fn from(ex: AdapterExecution) -> Self {
        let outcome = match ex.outcome {
            TaskOutcome::Success => "success",
            TaskOutcome::NoSideEffectFailure(_) => "no_side_effect_failure",
            TaskOutcome::Uncertain(_) => "uncertain",
            TaskOutcome::WaitingUser(_) => "waiting_user",
            TaskOutcome::UiDrift(_) => "ui_drift",
            TaskOutcome::Refused(_) => "refused",
        };
        Self {
            outcome: outcome.to_string(),
            side_effect_done: ex.side_effect_done,
            adapter_id: ex.adapter_id,
            adapter_version: ex.adapter_version,
            events: ex.events.iter().map(|e| format!("{e:?}")).collect(),
        }
    }
}

/// Host-testable core: verify + execute a signed adapter (YAML or canonical
/// JSON — YAML is a JSON superset) against the given device IO.
pub fn run_adapter(
    adapter_yaml: &str,
    signature: &str,
    signing_key: &[u8],
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
) -> Result<RunResult, String> {
    let ex = execute_signed_adapter(
        adapter_yaml,
        signature,
        signing_key,
        app_version,
        io,
        media_count,
    )
    .map_err(|e| e.to_string())?;
    Ok(RunResult::from(ex))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sincy_adapter::interpreter::MockSnapshot;
    use sincy_adapter::schema::AdapterDefinition;
    use sincy_adapter::sign::sign_definition;
    use sincy_agent::ScriptedIo;

    #[test]
    fn jni_core_runs_signed_adapter_end_to_end() {
        let yaml = include_str!("../../../adapters/x.publish_post.yaml");
        let def = AdapterDefinition::from_yaml(yaml).unwrap();
        let canonical = serde_json::to_string(&def).unwrap();
        let sig = sign_definition(PINNED_SIGNING_KEY, &canonical);
        let screens = vec![
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
        ];
        let mut io = ScriptedIo::new(screens);
        let res = run_adapter(yaml, &sig, PINNED_SIGNING_KEY, "10.5.0", &mut io, 0).unwrap();
        assert_eq!(res.outcome, "success");
        assert!(res.side_effect_done);
    }

    #[test]
    fn jni_core_refuses_tampered_signature() {
        let yaml = include_str!("../../../adapters/x.publish_post.yaml");
        let mut io = ScriptedIo::new(vec![]);
        assert!(run_adapter(yaml, "forged", PINNED_SIGNING_KEY, "10.5.0", &mut io, 0).is_err());
    }
}

/// JNI bridge — only compiled for Android. Kotlin supplies the current screen
/// (node-tree summary as JSON) and performs actions through the accessibility
/// service; the interpreter decides WHAT to do, never the other way round.
#[cfg(target_os = "android")]
mod jni {
    use super::*;
    use ::jni::objects::{JObject, JString, JValue};
    use ::jni::sys::jstring;
    use ::jni::JNIEnv;
    use serde_json::json;
    use sincy_adapter::humanize::{plan_swipe, plan_tap, HumanProfile, Rng};
    use sincy_adapter::interpreter::{Action, PlatformIo, ScreenSnapshot, SystemOp};
    use sincy_adapter::schema::FlowStep;

    /// Snapshot JSON handed over from Kotlin: ids/texts for anchor match plus a
    /// nodes array with bounds (used to plan humanized gestures) and screen size.
    struct JsonSnapshot(String);

    impl JsonSnapshot {
        fn value(&self) -> serde_json::Value {
            serde_json::from_str(&self.0)
                .unwrap_or_else(|_| json!({"ids": [], "texts": [], "nodes": []}))
        }

        /// Bounds (l, t, r, b) of the first node matching a target anchor.
        fn bounds_for(&self, target: &serde_json::Value) -> Option<(i32, i32, i32, i32)> {
            let v = self.value();
            let nodes = v["nodes"].as_array()?;
            let id = target["id"].as_str().filter(|s| !s.is_empty());
            let text = target["text"].as_str().filter(|s| !s.is_empty());
            let any: Vec<&str> = target["text_any"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            for n in nodes {
                let nid = n["id"].as_str().unwrap_or("");
                let ntext = n["text"].as_str().unwrap_or("");
                let ndesc = n["desc"].as_str().unwrap_or("");
                let hay = format!("{nid} {ntext} {ndesc}");
                let id_ok = id.is_some_and(|id| {
                    !nid.is_empty()
                        && (nid == id
                            || nid.ends_with(&format!("/{id}"))
                            || nid.ends_with(&format!(":{id}")))
                });
                let text_ok = text.is_some_and(|t| ntext == t || ndesc == t);
                let any_ok = !any.is_empty() && any.iter().any(|t| hay.contains(t));
                if id_ok || text_ok || any_ok {
                    let b = n["bounds"].as_array()?;
                    if b.len() == 4 {
                        return Some((
                            b[0].as_i64()? as i32,
                            b[1].as_i64()? as i32,
                            b[2].as_i64()? as i32,
                            b[3].as_i64()? as i32,
                        ));
                    }
                }
            }
            None
        }

        fn screen_size(&self) -> (i32, i32) {
            let v = self.value();
            (
                v["width"].as_i64().unwrap_or(1080) as i32,
                v["height"].as_i64().unwrap_or(2400) as i32,
            )
        }
    }

    impl ScreenSnapshot for JsonSnapshot {
        fn has_node_with_id(&self, id: &str) -> bool {
            serde_json::from_str::<serde_json::Value>(&self.0)
                .ok()
                .and_then(|v| v["ids"].as_array().cloned())
                .map(|a| {
                    a.iter().any(|x| {
                        let x = x.as_str().unwrap_or("");
                        x == id || x.ends_with(&format!("/{id}")) || x.ends_with(&format!(":{id}"))
                    })
                })
                .unwrap_or(false)
        }
        fn has_text(&self, text: &str) -> bool {
            serde_json::from_str::<serde_json::Value>(&self.0)
                .ok()
                .and_then(|v| v["texts"].as_array().cloned())
                .map(|a| {
                    a.iter()
                        .any(|x| x.as_str().map(|t| t.trim() == text.trim()).unwrap_or(false))
                })
                .unwrap_or(false)
        }
    }

    /// Bridge that calls BACK into Kotlin: AdapterRuntime.snapshotJson() before each
    /// interpreter step, AdapterRuntime.performAction(json) for each action. The
    /// Kotlin accessibility service injects the real touch (dispatchGesture).
    /// Humanization lives on the Rust side: the bridge resolves node bounds from
    /// the snapshot and sends CONCRETE gesture plans (coordinates + timing) down.
    struct JniBridge<'a> {
        env: JNIEnv<'a>,
        thiz: JObject<'a>,
        profile: HumanProfile,
        rng: Rng,
        last_snapshot: String,
    }

    impl<'a> PlatformIo for JniBridge<'a> {
        fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
            let json = self
                .env
                .call_method(&self.thiz, "snapshotJson", "()Ljava/lang/String;", &[])
                .and_then(|v| v.l())
                .ok()
                .and_then(|o| {
                    let js = JString::from(o);
                    self.env
                        .get_string(&js)
                        .ok()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "{\"ids\":[],\"texts\":[],\"nodes\":[]}".into());
            self.last_snapshot = json.clone();
            Box::new(JsonSnapshot(json))
        }

        fn perform_system_op(&mut self, op: &SystemOp) -> Result<String, String> {
            // 系统级 op 直接以 JSON 形式交给 Kotlin 侧执行 (下载/安装/轮询/验签/启动),
            // Kotlin 执行后把结果字符串记在 AdapterRuntime.lastOpResult, 这里再读回。
            let payload = json!({"kind": "system_op", "op": op}).to_string();
            let js = self.env.new_string(&payload).map_err(|e| e.to_string())?;
            let ok = self
                .env
                .call_method(
                    &self.thiz,
                    "performAction",
                    "(Ljava/lang/String;)Z",
                    &[JValue::Object(&js)],
                )
                .and_then(|v| v.z())
                .map_err(|e| e.to_string())?;
            if !ok {
                return Err(format!("system op rejected: {payload}"));
            }
            let r = self
                .env
                .call_method(&self.thiz, "lastOpResult", "()Ljava/lang/String;", &[])
                .and_then(|v| v.l())
                .ok()
                .and_then(|o| {
                    let js = JString::from(o);
                    self.env
                        .get_string(&js)
                        .ok()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "ok".into());
            Ok(r)
        }

        fn perform(&mut self, step: &FlowStep) -> Result<(), String> {
            let action = step.to_action(0).ok_or("step has no performable action")?;
            let snap = JsonSnapshot(self.last_snapshot.clone());
            let payload = action_payload(&action, &snap, &self.profile, &mut self.rng);
            let js = self.env.new_string(&payload).map_err(|e| e.to_string())?;
            let ok = self
                .env
                .call_method(
                    &self.thiz,
                    "performAction",
                    "(Ljava/lang/String;)Z",
                    &[JValue::Object(&js)],
                )
                .and_then(|v| v.z())
                .map_err(|e| e.to_string())?;
            if ok {
                Ok(())
            } else {
                Err(format!("performAction rejected: {payload}"))
            }
        }
    }

    /// Serialize an interpreter action into the JSON contract Kotlin parses.
    /// Tap/scroll carry a concrete humanized plan; if the node bounds are not in
    /// the snapshot we fall back to target-based resolution on the Kotlin side.
    fn action_payload(a: &Action, snap: &JsonSnapshot, p: &HumanProfile, rng: &mut Rng) -> String {
        match a {
            Action::Tap(t) => {
                let target = serde_json::to_value(t).unwrap_or_default();
                if let Some(b) = snap.bounds_for(&target) {
                    let plan = plan_tap(b, p, rng);
                    json!({
                        "kind": "tap",
                        "x": plan.x, "y": plan.y,
                        "hold_ms": plan.hold_ms,
                        "pre_ms": plan.pre_ms, "post_ms": plan.post_ms,
                    })
                    .to_string()
                } else {
                    json!({"kind": "tap", "target": t}).to_string()
                }
            }
            Action::Scroll => {
                let (w, h) = snap.screen_size();
                let plan = plan_swipe(
                    w / 2,
                    (h as f64 * 0.72) as i32,
                    w / 2 + rng.int(-30, 30),
                    (h as f64 * 0.28) as i32,
                    p,
                    rng,
                );
                json!({
                    "kind": "swipe",
                    "segments": plan.segments,
                    "post_ms": rng.range(250.0, 700.0) as u64,
                })
                .to_string()
            }
            Action::SetText { target, value } => json!({
                "kind": "set_text",
                "target": target, "value": value,
                "pre_ms": rng.range(120.0, 450.0) as u64,
            })
            .to_string(),
            Action::PickMedia { index } => json!({
                "kind": "pick_media", "index": index,
                "pre_ms": rng.range(120.0, 450.0) as u64,
            })
            .to_string(),
            Action::Back => {
                json!({"kind": "back", "pre_ms": rng.range(120.0, 450.0) as u64}).to_string()
            }
            Action::CloseDialog => {
                json!({"kind": "close_dialog", "pre_ms": rng.range(120.0, 450.0) as u64})
                    .to_string()
            }
        }
    }

    /// Java_com_sincy_agent_adapter_AdapterRuntime_runAdapter
    /// Instance method of the Kotlin `object AdapterRuntime`, so JNI receives
    /// (env, thiz, adapterJson, signature, appVersion) -> result JSON string.
    #[no_mangle]
    pub extern "C" fn Java_com_sincy_agent_adapter_AdapterRuntime_runAdapter(
        mut env: JNIEnv,
        thiz: JObject,
        adapter_json: JString,
        signature: JString,
        app_version: JString,
    ) -> jstring {
        let adapter_json: String = env
            .get_string(&adapter_json)
            .map(|s| s.into())
            .unwrap_or_default();
        let signature: String = env
            .get_string(&signature)
            .map(|s| s.into())
            .unwrap_or_default();
        let app_version: String = env
            .get_string(&app_version)
            .map(|s| s.into())
            .unwrap_or_default();
        let mut bridge = JniBridge {
            env,
            thiz,
            profile: HumanProfile::default(),
            rng: Rng::from_time(),
            last_snapshot: "{\"ids\":[],\"texts\":[],\"nodes\":[]}".into(),
        };
        let result = match run_adapter(
            &adapter_json,
            &signature,
            PINNED_SIGNING_KEY,
            &app_version,
            &mut bridge,
            0,
        ) {
            Ok(r) => serde_json::to_string(&r).unwrap_or_else(|_| "{\"outcome\":\"error\"}".into()),
            Err(e) => serde_json::to_string(&RunResult {
                outcome: "refused".into(),
                side_effect_done: false,
                adapter_id: String::new(),
                adapter_version: 0,
                events: vec![e],
            })
            .unwrap_or_else(|_| "{\"outcome\":\"refused\"}".into()),
        };
        let env = bridge.env;
        let out = env.new_string(result).unwrap_or_default();
        out.into_raw()
    }
}
