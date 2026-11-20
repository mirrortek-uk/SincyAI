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

/// SHA-256 hex of the definition (stored alongside for integrity audits).
pub fn definition_sha256(canonical_definition_json: &str) -> String {
    format!("{:x}", Sha256::digest(canonical_definition_json.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
