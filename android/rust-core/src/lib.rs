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
use sincy_agent::{execute_signed_adapter_ctx_v2, AdapterExecution, TaskOutcome};

/// Pinned platform adapter-signing key. In production builds this is compiled
/// in from a build-time secret (not from network config); the dev build uses
/// the local dev key, matching migrations/0015_seed_x_adapter.sql.
/// Ed25519 公钥 (烧录进设备, 只有公钥 —— 私钥只在云端签名端)。
/// 对应 keys/adapter_signing_ed25519.key (gitignored)。
pub const PINNED_VERIFYING_KEY_B64: &str = "PBiZF2NoULhRKTsdvX68s7ygJEOQYXUyBRUvPU3tMSg=";

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
    /// 失败/漂移原因 (诊断用), 成功时为空
    pub reason: String,
}

impl From<AdapterExecution> for RunResult {
    fn from(ex: AdapterExecution) -> Self {
        let (outcome, reason) = match ex.outcome {
            TaskOutcome::Success => ("success", String::new()),
            TaskOutcome::NoSideEffectFailure(m) => ("no_side_effect_failure", m),
            TaskOutcome::Uncertain(m) => ("uncertain", m),
            TaskOutcome::WaitingUser(m) => ("waiting_user", m),
            TaskOutcome::UiDrift(m) => ("ui_drift", m),
            TaskOutcome::Refused(m) => ("refused", m),
        };
        Self {
            outcome: outcome.to_string(),
            side_effect_done: ex.side_effect_done,
            adapter_id: ex.adapter_id,
            adapter_version: ex.adapter_version,
            events: ex.events.iter().map(|e| format!("{e:?}")).collect(),
            reason,
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
    task_context: Option<serde_json::Value>,
) -> Result<RunResult, String> {
    run_adapter_scheme(
        adapter_yaml,
        signature,
        signing_key,
        "hmac-sha256",
        app_version,
        io,
        media_count,
        task_context,
    )
}

/// 带签名方案的入口: ed25519 时用烧录的公钥验签 (PINNED_VERIFYING_KEY), 私钥永不下发设备。
#[allow(clippy::too_many_arguments)]
pub fn run_adapter_scheme(
    adapter_yaml: &str,
    signature: &str,
    signing_key: &[u8],
    signature_scheme: &str,
    app_version: &str,
    io: &mut dyn PlatformIo,
    media_count: usize,
    task_context: Option<serde_json::Value>,
) -> Result<RunResult, String> {
    let key_material: &[u8] = if signature_scheme == "ed25519" {
        // 编译期嵌入的公钥 (PINNED_VERIFYING_KEY_B64 的字节形式)
        &PINNED_VERIFYING_KEY_BYTES
    } else {
        signing_key
    };
    let ex = execute_signed_adapter_ctx_v2(
        adapter_yaml,
        signature,
        key_material,
        signature_scheme,
        app_version,
        io,
        media_count,
        task_context,
    )
    .map_err(|e| e.to_string())?;
    Ok(RunResult::from(ex))
}

/// PINNED_VERIFYING_KEY_B64 解码后的 32 字节公钥 (lazy static, 避免引入 once_cell)。
static PINNED_VERIFYING_KEY_BYTES: [u8; 32] = [
    0x3c, 0x18, 0x99, 0x17, 0x63, 0x68, 0x50, 0xb8, 0x51, 0x29, 0x3b, 0x1d, 0xbd, 0x7e, 0xbc, 0xb3,
    0xbc, 0xa0, 0x24, 0x43, 0x90, 0x61, 0x75, 0x32, 0x05, 0x15, 0x2f, 0x3d, 0x4d, 0xed, 0x31, 0x28,
];

#[cfg(test)]
mod tests {
    use super::*;
    use sincy_adapter::interpreter::MockSnapshot;
    use sincy_adapter::schema::AdapterDefinition;
    use sincy_adapter::sign::{
        sign_definition, sign_definition_ed25519, verify_definition_ed25519,
    };
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
        let res = run_adapter(yaml, &sig, PINNED_SIGNING_KEY, "10.5.0", &mut io, 0, None).unwrap();
        assert_eq!(res.outcome, "success");
        assert!(res.side_effect_done);
    }

    #[test]
    fn jni_core_refuses_tampered_signature() {
        let yaml = include_str!("../../../adapters/x.publish_post.yaml");
        let mut io = ScriptedIo::new(vec![]);
        assert!(run_adapter(
            yaml,
            "forged",
            PINNED_SIGNING_KEY,
            "10.5.0",
            &mut io,
            0,
            None
        )
        .is_err());
    }
    #[test]
    fn pinned_ed25519_verifying_key_matches_private_key() {
        // 私钥文件 (gitignored) 签名, 设备端烧录的公钥必须能验过
        let seed = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../keys/adapter_signing_ed25519.key"
        ))
        .expect("private key file");
        let def = r#"{"adapter":"x","version":1}"#;
        let sig = sign_definition_ed25519(&seed, def).unwrap();
        assert!(
            verify_definition_ed25519(&PINNED_VERIFYING_KEY_BYTES, def, &sig).is_ok(),
            "pinned public key must verify signatures from keys/adapter_signing_ed25519.key"
        );
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
            // Tap targets must intersect the visible screen: hidden ViewPager pages
            // (e.g. the inactive home tab) carry duplicate nodes shifted off-screen,
            // and a gesture at those coordinates is a silent no-op.
            let (sw, sh) = self.screen_size();
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
                        let (l, t, r, bo) = (
                            b[0].as_i64()? as i32,
                            b[1].as_i64()? as i32,
                            b[2].as_i64()? as i32,
                            b[3].as_i64()? as i32,
                        );
                        if r <= l || bo <= t || r <= 0 || l >= sw || bo <= 0 || t >= sh {
                            continue; // zero-area or off-screen node
                        }
                        return Some((l, t, r, bo));
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
            // Challenge phrases must match a UI control (clickable/editable node) or a
            // short label — never a long post body, otherwise tweet content quoting
            // "captcha"/"unusual activity" would falsely park the task as WAITING_USER.
            serde_json::from_str::<serde_json::Value>(&self.0)
                .ok()
                .and_then(|v| v["nodes"].as_array().cloned())
                .map(|a| {
                    a.iter().any(|n| {
                        let matches = ["text", "desc"].iter().any(|k| {
                            n[k].as_str()
                                .map(|t| t.trim() == text.trim())
                                .unwrap_or(false)
                        });
                        if !matches {
                            return false;
                        }
                        let clickable = n["clickable"].as_bool().unwrap_or(false);
                        let editable = n["editable"].as_bool().unwrap_or(false);
                        let short = n["text"].as_str().map(|t| t.len() <= 48).unwrap_or(false)
                            || n["desc"].as_str().map(|t| t.len() <= 48).unwrap_or(false);
                        clickable || editable || short
                    })
                })
                .unwrap_or(false)
        }
        fn image_base64(&self) -> Option<String> {
            self.value()["screenshot"].as_str().map(String::from)
        }
        fn describe(&self) -> String {
            let v = self.value();
            let ids = v["ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let texts = v["texts"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let nodes = v["nodes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|n| {
                            let id = n["id"].as_str().unwrap_or("");
                            let text = n["text"].as_str().unwrap_or("");
                            let desc = n["desc"].as_str().unwrap_or("");
                            let idx = n["idx"].as_u64().unwrap_or(0);
                            let b = n["bounds"].as_array()?;
                            let bb = b.iter().filter_map(|x| x.as_i64()).collect::<Vec<_>>();
                            let mut s = format!("{id}|{text}|{desc}|{bb:?}");
                            if idx > 0 {
                                s.push_str(&format!("|idx={idx}"));
                            }
                            Some(s)
                        })
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .unwrap_or_default();
            format!(
                "screen {}x{} ids=[{ids}] texts=[{texts}] nodes=[{nodes}]",
                v["width"].as_i64().unwrap_or(0),
                v["height"].as_i64().unwrap_or(0)
            )
        }
        fn elements_summary(&self) -> String {
            let v = self.value();
            let mut out = String::new();
            if let Some(nodes) = v["nodes"].as_array() {
                for n in nodes {
                    let idx = n["idx"].as_u64().unwrap_or(0);
                    if idx == 0 {
                        continue;
                    }
                    let id = n["id"].as_str().unwrap_or("");
                    let text = n["text"].as_str().unwrap_or("");
                    let desc = n["desc"].as_str().unwrap_or("");
                    let label = if !text.is_empty() {
                        text
                    } else if !desc.is_empty() {
                        desc
                    } else {
                        id.rsplit('/').next().unwrap_or(id)
                    };
                    let b: Option<Vec<i64>> = n["bounds"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect::<Vec<_>>());
                    if let Some(b) = b.filter(|b| b.len() == 4) {
                        use std::fmt::Write as _;
                        let _ = writeln!(
                            out,
                            "idx={idx} {} @ [{},{},{},{}]",
                            label.chars().take(60).collect::<String>(),
                            b[0],
                            b[1],
                            b[2],
                            b[3]
                        );
                    }
                }
            }
            out
        }
        fn element_bounds(&self, idx: u32) -> Option<(i32, i32, i32, i32)> {
            let v = self.value();
            let nodes = v["nodes"].as_array()?;
            for n in nodes {
                if n["idx"].as_u64() == Some(idx as u64) {
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

        fn llm_decide(&mut self, req: &serde_json::Value) -> Result<serde_json::Value, String> {
            // LLM decision via Kotlin callback: POST {cloud}/api/v1/ai/generate with
            // the device token; the reply text may wrap the JSON, so extract the
            // first balanced {...} block.
            let payload = req.to_string();
            let js = self.env.new_string(&payload).map_err(|e| e.to_string())?;
            let raw = self
                .env
                .call_method(
                    &self.thiz,
                    "llmDecide",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[JValue::Object(&js)],
                )
                .and_then(|v| v.l())
                .ok()
                .and_then(|o| {
                    let js = JString::from(o);
                    self.env
                        .get_string(&js)
                        .ok()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .ok_or("llmDecide returned null")?;
            let start = raw.find('{').ok_or("no json object in llm reply")?;
            let end = raw.rfind('}').ok_or("no json object end in llm reply")?;
            serde_json::from_str(&raw[start..=end])
                .map_err(|e| format!("bad llm json: {e}: {}", &raw[..raw.len().min(300)]))
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
            // 视觉/坐标动作: 以坐标为中心造一个小包围盒, 走同样的人性化 tap 规划
            Action::TapAt { x, y } => {
                let b = (x - 8, y - 8, x + 8, y + 8);
                let plan = plan_tap(b, p, rng);
                json!({
                    "kind": "tap",
                    "x": plan.x, "y": plan.y,
                    "hold_ms": plan.hold_ms,
                    "pre_ms": plan.pre_ms, "post_ms": plan.post_ms,
                })
                .to_string()
            }
            Action::SwipePath { x1, y1, x2, y2, ms } => {
                let plan = plan_swipe(*x1, *y1, *x2, *y2, p, rng);
                let mut segs = plan.segments;
                // respect requested duration roughly by scaling segment ms
                let total: u64 = segs.iter().map(|s| s.ms).sum();
                if total > 0 && *ms > 0 {
                    let scale = *ms as f64 / total as f64;
                    for s in segs.iter_mut() {
                        s.ms = ((s.ms as f64) * scale).max(16.0) as u64;
                    }
                }
                json!({
                    "kind": "swipe",
                    "segments": segs,
                    "post_ms": rng.range(250.0, 700.0) as u64,
                })
                .to_string()
            }
            Action::Wait { ms } => json!({"kind": "wait", "ms": ms}).to_string(),
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
        signature_scheme: JString,
        app_version: JString,
        task_context: JString,
    ) -> jstring {
        let adapter_json: String = env
            .get_string(&adapter_json)
            .map(|s| s.into())
            .unwrap_or_default();
        let signature: String = env
            .get_string(&signature)
            .map(|s| s.into())
            .unwrap_or_default();
        let signature_scheme: String = env
            .get_string(&signature_scheme)
            .map(|s| s.into())
            .unwrap_or_else(|_| "hmac-sha256".into());
        let app_version: String = env
            .get_string(&app_version)
            .map(|s| s.into())
            .unwrap_or_default();
        let task_context_json: String = env
            .get_string(&task_context)
            .map(|s| s.into())
            .unwrap_or_default();
        let task_context: Option<serde_json::Value> = if task_context_json.is_empty() {
            None
        } else {
            serde_json::from_str(&task_context_json).ok()
        };
        let mut bridge = JniBridge {
            env,
            thiz,
            profile: HumanProfile::default(),
            rng: Rng::from_time(),
            last_snapshot: "{\"ids\":[],\"texts\":[],\"nodes\":[]}".into(),
        };
        // catch_unwind: 任何 Rust panic 都不能穿过 JNI abort 掉整个进程
        // (进程一死, MIUI 会把无障碍服务标记为 crashed, 任务全部卡死)
        // 同时用 panic hook 把 panic 信息存下来, 随结果回传便于诊断
        static PANIC_MSG: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("{info}");
            if let Ok(mut g) = PANIC_MSG.lock() {
                *g = Some(msg);
            }
        }));
        let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_adapter_scheme(
                &adapter_json,
                &signature,
                PINNED_SIGNING_KEY,
                &signature_scheme,
                &app_version,
                &mut bridge,
                0,
                task_context,
            )
        })) {
            Ok(Ok(r)) => {
                serde_json::to_string(&r).unwrap_or_else(|_| "{\"outcome\":\"error\"}".into())
            }
            Ok(Err(e)) => serde_json::to_string(&RunResult {
                outcome: "refused".into(),
                reason: e.clone(),
                side_effect_done: false,
                adapter_id: String::new(),
                adapter_version: 0,
                events: vec![e],
            })
            .unwrap_or_else(|_| "{\"outcome\":\"refused\"}".into()),
            Err(_) => {
                let msg = PANIC_MSG
                    .lock()
                    .ok()
                    .and_then(|mut g| g.take())
                    .unwrap_or_else(|| "unknown panic".into());
                serde_json::to_string(&RunResult {
                    outcome: "no_side_effect_failure".into(),
                    reason: format!("rust panic caught at JNI boundary: {msg}"),
                    side_effect_done: false,
                    adapter_id: String::new(),
                    adapter_version: 0,
                    events: vec!["panic".to_string()],
                })
                .unwrap_or_else(|_| "{\"outcome\":\"error\"}".into())
            }
        };
        std::panic::set_hook(prev_hook);
        let env = bridge.env;
        let out = env.new_string(result).unwrap_or_default();
        out.into_raw()
    }
}
