//! Ops tool: print the canonical JSON, sha256 and HMAC signature of an adapter
//! YAML file, ready to paste into an INSERT for app_adapter_versions.
//!
//!   cargo run -p sincy-agent --example sign_adapter -- \
//!     ../../adapters/x.publish_post.yaml sincy-dev-adapter-signing-key

use sincy_adapter::schema::AdapterDefinition;
use sincy_adapter::sign::{definition_sha256, sign_definition};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let key = args[2].as_bytes();
    let yaml = std::fs::read_to_string(path).expect("read yaml");
    let def = AdapterDefinition::from_yaml(&yaml).expect("valid adapter");
    let canonical = serde_json::to_string(&def).expect("canonical json");
    println!("definition_json: {canonical}");
    println!("sha256: {}", definition_sha256(&canonical));
    println!("signature: {}", sign_definition(key, &canonical));
}
