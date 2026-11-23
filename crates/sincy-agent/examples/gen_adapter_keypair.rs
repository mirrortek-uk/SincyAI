//! 生成 adapter 签名 Ed25519 密钥对: 私钥写 keys/adapter_signing_ed25519.key (32B seed),
//! 公钥 base64 打印 (烧进设备端 PINNED_VERIFYING_KEY)。
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use ed25519_dalek::SigningKey;

fn main() {
    let mut seed = [0u8; 32];
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key();
    std::fs::create_dir_all("keys").ok();
    std::fs::write("keys/adapter_signing_ed25519.key", seed).expect("write private key");
    println!("private_key_written=keys/adapter_signing_ed25519.key");
    println!("public_key_b64={}", B64.encode(pk.to_bytes()));
    println!("public_key_hex={}", hex::encode(pk.to_bytes()));
}
