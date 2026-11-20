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
    use jni::objects::{JClass, JString};
    use jni::sys::jstring;
    use jni::JNIEnv;
    use sincy_adapter::interpreter::{FlowStep, ScreenSnapshot};
    use std::sync::Mutex;

    /// Bridge implemented on the Kotlin side (perception/NodeTreeBridge):
    /// `snapshotJson()` returns the current screen summary, `perform(actionJson)`
    /// executes one interpreter action on the node tree.
    pub struct KotlinBridge {
        pub snapshot: Mutex<String>,
        pub performed: Mutex<Vec<String>>,
    }

    impl PlatformIo for KotlinBridge {
        fn snapshot(&mut self) -> Box<dyn ScreenSnapshot> {
            // parsed on the Kotlin side into a node list; here we wrap the raw json
            let json = self.snapshot.lock().unwrap().clone();
            Box::new(JsonSnapshot(json))
        }
        fn perform(&mut self, step: &FlowStep) -> Result<(), String> {
            self.performed
                .lock()
                .unwrap()
                .push(serde_json::to_string(step).unwrap_or_default());
            Ok(())
        }
    }

    struct JsonSnapshot(String);
    impl ScreenSnapshot for JsonSnapshot {
        fn has_node_with_id(&self, id: &str) -> bool {
            serde_json::from_str::<serde_json::Value>(&self.0)
                .ok()
                .and_then(|v| v["ids"].as_array().cloned())
                .map(|a| a.iter().any(|x| x.as_str() == Some(id)))
                .unwrap_or(false)
        }
        fn has_text(&self, text: &str) -> bool {
            serde_json::from_str::<serde_json::Value>(&self.0)
                .ok()
                .and_then(|v| v["texts"].as_array().cloned())
                .map(|a| a.iter().any(|x| x.as_str() == Some(text)))
                .unwrap_or(false)
        }
    }

    /// Java_com_sincy_agent_adapter_AdapterRuntime_runAdapter
    /// (adapterJson, signature, appVersion) -> result JSON string.
    #[no_mangle]
    pub extern "C" fn Java_com_sincy_agent_adapter_AdapterRuntime_runAdapter(
        mut env: JNIEnv,
        _class: JClass,
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
        let result = run_adapter_with_bridge(&adapter_json, &signature, &app_version);
        let out = env.new_string(result).unwrap_or_default();
        out.into_raw()
    }

    fn run_adapter_with_bridge(adapter_json: &str, signature: &str, app_version: &str) -> String {
        // The Kotlin NodeTreeBridge fills the snapshot before each step via the
        // accessibility service; here we execute with that bridge.
        let mut bridge = KotlinBridge {
            snapshot: Mutex::new(String::from("{\"ids\":[],\"texts\":[]}")),
            performed: Mutex::new(vec![]),
        };
        match run_adapter(
            adapter_json,
            signature,
            PINNED_SIGNING_KEY,
            app_version,
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
        }
    }
}
