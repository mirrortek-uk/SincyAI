use sincy_adapter::schema::AdapterDefinition;
use sincy_agent_core::{run_adapter, PINNED_SIGNING_KEY};
fn main() {
    let data = std::fs::read_to_string("D:/tmp/reply_def.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&data).unwrap();
    let def_yaml = v["definition"].to_string();
    let sig = v["signature"].as_str().unwrap().to_string();
    let def = AdapterDefinition::from_yaml(&def_yaml).unwrap();
    let canonical = serde_json::to_string(&def).unwrap();
    let yaml = canonical.clone();
    for ver in ["12.44.0-prod.01", "0", "12.44.0"] {
        let mut io = sincy_agent::ScriptedIo::new(vec![]);
        match run_adapter(&yaml, &sig, PINNED_SIGNING_KEY, ver, &mut io, 0, None) {
            Ok(r) => println!("ver={} outcome={} events={:?}", ver, r.outcome, r.events),
            Err(e) => println!("ver={} ERR {}", ver, e),
        }
    }
}
