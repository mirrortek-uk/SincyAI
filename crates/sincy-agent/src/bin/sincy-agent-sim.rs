//! sincy-agent-sim: host simulator of the phone agent for local verification.
//!
//! It runs the REAL agent core (sincy-agent) against a local cloud-api:
//! connects the WSS gateway with a device token, waits for a device.task.assign,
//! fetches the signed task brief over HTTP, and executes the adapter on a
//! scripted screen sequence (instead of a real accessibility node tree).
//!
//! Example:
//!   sincy-agent-sim \
//!     --ws-url "ws://127.0.0.1:18080/api/v1/agent/connect?token=$TOKEN" \
//!     --http-base "http://127.0.0.1:18080/api/v1" \
//!     --device-id "$DEVICE_ID" --agency-id "$AGENCY_ID" \
//!     --app-version "10.5.0" \
//!     --screens '[{"ids":["com.twitter.android:id/bottom_bar"],"texts":["Home"]}, ...]'

use sincy_adapter::interpreter::MockSnapshot;
use sincy_agent::{AgentClient, ScriptedIo, TaskOutcome};

fn arg(name: &str, default: Option<&str>) -> String {
    let args: Vec<String> = std::env::args().collect();
    let flag = format!("--{name}");
    for (i, a) in args.iter().enumerate() {
        if a == &flag {
            return args.get(i + 1).cloned().unwrap_or_else(|| {
                eprintln!("missing value for {flag}");
                std::process::exit(2);
            });
        }
    }
    default.map(|s| s.to_string()).unwrap_or_else(|| {
        eprintln!("missing required {flag}");
        std::process::exit(2);
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let ws_url = arg("ws-url", None);
    let http_base = arg("http-base", Some("http://127.0.0.1:18080/api/v1"));
    let token = arg("token", None);
    let device_id = arg("device-id", None);
    let agency_id = arg("agency-id", None);
    let signing_key = arg("signing-key", Some("sincy-dev-adapter-signing-key"));
    let app_version = arg("app-version", Some("10.5.0"));
    let screens_json = arg("screens", Some("[]"));

    let screens: Vec<MockSnapshot> = serde_json::from_str::<Vec<serde_json::Value>>(&screens_json)
        .expect("screens must be a JSON array of {ids:[], texts:[]}")
        .into_iter()
        .map(|s| {
            MockSnapshot::new(
                s["ids"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                s["texts"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        })
        .collect();

    let client = AgentClient {
        ws_url,
        http_base,
        token,
        device_id,
        agency_id,
        signing_key: signing_key.into_bytes(),
    };

    println!("sim agent: waiting for a device.task.assign ...");
    let result = client
        .run_next_task(
            move || Box::new(ScriptedIo::new(screens)),
            &app_version,
            std::time::Duration::from_secs(30),
        )
        .await;

    match result {
        Ok(ex) => {
            println!(
                "task done: outcome={:?} side_effect_done={}",
                ex.outcome, ex.side_effect_done
            );
            let code = match ex.outcome {
                TaskOutcome::Success => 0,
                _ => 1,
            };
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("agent error: {e}");
            std::process::exit(2);
        }
    }
}
