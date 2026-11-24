//! Debug helper: verify which signing key matches a stored adapter signature.
//! cargo run -p sincy-agent --example check_db_sig -- <definition.json> <signature> <key...>
use sincy_adapter::schema::AdapterDefinition;
use sincy_adapter::sign::{sign_definition, verify_definition};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let def_json = std::fs::read_to_string(&args[1]).unwrap();
    let sig = &args[2];
    let def = AdapterDefinition::from_yaml(&def_json).unwrap();
    let canonical = serde_json::to_string(&def).unwrap();
    for key in &args[3..] {
        let kb = if let Some(path) = key.strip_prefix('@') {
            std::fs::read(path).unwrap()
        } else {
            key.as_bytes().to_vec()
        };
        let computed = sign_definition(&kb, &canonical);
        let ok = verify_definition(&kb, &canonical, sig).is_ok();
        println!(
            "key={key:?} computed={} match={ok}",
            &computed[..24.min(computed.len())]
        );
    }
    println!("db sig={}", &sig[..24.min(sig.len())]);
    println!(
        "RESIGN {}",
        sign_definition(b"sincy-dev-adapter-signing-key", &canonical)
    );
}
