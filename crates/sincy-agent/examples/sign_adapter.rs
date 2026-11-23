//! Ops tool: print the canonical JSON, sha256 and HMAC signature of an adapter
//! YAML file, ready to paste into an INSERT for app_adapter_versions.
//!
//!   cargo run -p sincy-agent --example sign_adapter -- \
//!     ../../adapters/x.publish_post.yaml sincy-dev-adapter-signing-key

use sincy_adapter::schema::AdapterDefinition;
use sincy_adapter::sign::{definition_sha256, sign_definition, sign_definition_ed25519};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    // key 可以是字面量, 或 @path 从文件读取二进制密钥 (release 用 keys/adapter_signing_key.bin)
    let key: Vec<u8> = if let Some(path) = args[2].strip_prefix('@') {
        std::fs::read(path).expect("read key file")
    } else {
        args[2].clone().into_bytes()
    };
    // 第三个参数可选: 签名方案 ed25519 (默认 hmac-sha256)
    let scheme = args.get(3).map(|s| s.as_str()).unwrap_or("hmac-sha256");
    let yaml = std::fs::read_to_string(path).expect("read yaml");
    let def = AdapterDefinition::from_yaml(&yaml).expect("valid adapter");
    let canonical = serde_json::to_string(&def).expect("canonical json");
    println!("definition_json: {canonical}");
    println!("sha256: {}", definition_sha256(&canonical));
    println!("signature_scheme: {scheme}");
    if scheme == "ed25519" {
        let sig = sign_definition_ed25519(&key, &canonical).expect("ed25519 sign");
        println!("signature: {sig}");
    } else {
        println!("signature: {}", sign_definition(&key, &canonical));
    }
}
