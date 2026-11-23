//! Adapter bundle signing (TDD 9.3 / 15): Sincy signs the definition; devices
//! verify before executing. Dev slice uses HMAC-SHA256 with a platform signing
//! key; production swaps in Ed25519 without changing the flow.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("signature verification failed")]
    BadSignature,
}

fn hmac(key: &[u8], data: &[u8]) -> String {
    // HMAC-SHA256
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let h = Sha256::digest(key);
        key_block[..32].copy_from_slice(&h);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    for b in key_block.iter() {
        inner.update([b ^ 0x36]);
    }
    inner.update(data);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    for b in key_block.iter() {
        outer.update([b ^ 0x5c]);
    }
    outer.update(inner_hash);
    B64.encode(outer.finalize())
}

pub fn sign_definition(signing_key: &[u8], canonical_definition_json: &str) -> String {
    hmac(signing_key, canonical_definition_json.as_bytes())
}

pub fn verify_definition(
    signing_key: &[u8],
    canonical_definition_json: &str,
    signature: &str,
) -> Result<(), SigningError> {
    let expected = sign_definition(signing_key, canonical_definition_json);
    if expected == signature {
        Ok(())
    } else {
        Err(SigningError::BadSignature)
    }
}

/// 签名方案: hmac-sha256 (dev/MVP, 设备持有对称密钥) / ed25519 (生产, 设备只持有公钥)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureScheme {
    HmacSha256,
    Ed25519,
}

impl SignatureScheme {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignatureScheme::HmacSha256 => "hmac-sha256",
            SignatureScheme::Ed25519 => "ed25519",
        }
    }
    pub fn parse(s: &str) -> Option<SignatureScheme> {
        match s {
            "hmac-sha256" | "hmac" => Some(SignatureScheme::HmacSha256),
            "ed25519" => Some(SignatureScheme::Ed25519),
            _ => None,
        }
    }
}

/// Ed25519 签名 (私钥 = 32 字节 seed), 输出 base64 签名。
pub fn sign_definition_ed25519(
    secret_seed: &[u8],
    canonical_definition_json: &str,
) -> Result<String, SigningError> {
    use ed25519_dalek::{Signature, Signer, SigningKey};
    let seed: [u8; 32] = secret_seed
        .try_into()
        .map_err(|_| SigningError::BadSignature)?;
    let sk = SigningKey::from_bytes(&seed);
    let sig: Signature = sk.sign(canonical_definition_json.as_bytes());
    Ok(B64.encode(sig.to_bytes()))
}

/// Ed25519 验签 (公钥 32 字节)。设备端只需要公钥。
pub fn verify_definition_ed25519(
    public_key: &[u8],
    canonical_definition_json: &str,
    signature: &str,
) -> Result<(), SigningError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pk: [u8; 32] = public_key
        .try_into()
        .map_err(|_| SigningError::BadSignature)?;
    let vk = VerifyingKey::from_bytes(&pk).map_err(|_| SigningError::BadSignature)?;
    let sig_bytes = B64
        .decode(signature)
        .map_err(|_| SigningError::BadSignature)?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| SigningError::BadSignature)?;
    vk.verify(canonical_definition_json.as_bytes(), &sig)
        .map_err(|_| SigningError::BadSignature)
}

/// 按方案验签: key_material 对 ed25519 是 32 字节公钥, 对 hmac 是对称密钥。
pub fn verify_definition_scheme(
    scheme: SignatureScheme,
    key_material: &[u8],
    canonical_definition_json: &str,
    signature: &str,
) -> Result<(), SigningError> {
    match scheme {
        SignatureScheme::HmacSha256 => {
            verify_definition(key_material, canonical_definition_json, signature)
        }
        SignatureScheme::Ed25519 => {
            verify_definition_ed25519(key_material, canonical_definition_json, signature)
        }
    }
}

/// SHA-256 hex of the definition (stored alongside for integrity audits).
pub fn definition_sha256(canonical_definition_json: &str) -> String {
    format!("{:x}", Sha256::digest(canonical_definition_json.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed25519_sign_verify_roundtrip() {
        use ed25519_dalek::{SigningKey, VerifyingKey};
        let seed = [7u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let pk: VerifyingKey = sk.verifying_key();
        let def = r#"{"adapter":"x"}"#;
        let sig = sign_definition_ed25519(&seed, def).unwrap();
        assert!(verify_definition_ed25519(pk.as_bytes(), def, &sig).is_ok());
        assert!(verify_definition_ed25519(pk.as_bytes(), r#"{"adapter":"y"}"#, &sig).is_err());
        // 设备端只有公钥, 用私钥验签必须失败
        let other = SigningKey::from_bytes(&[9u8; 32]);
        assert!(verify_definition_ed25519(other.verifying_key().as_bytes(), def, &sig).is_err());
        // scheme 分发
        assert!(
            verify_definition_scheme(SignatureScheme::Ed25519, pk.as_bytes(), def, &sig).is_ok()
        );
        assert!(SignatureScheme::parse("ed25519") == Some(SignatureScheme::Ed25519));
        assert!(SignatureScheme::parse("hmac-sha256") == Some(SignatureScheme::HmacSha256));
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = b"platform-signing-key";
        let def = r#"{"adapter":"x"}"#;
        let sig = sign_definition(key, def);
        assert!(verify_definition(key, def, &sig).is_ok());
        assert!(verify_definition(key, r#"{"adapter":"y"}"#, &sig).is_err());
        assert!(verify_definition(b"wrong-key", def, &sig).is_err());
    }
}
