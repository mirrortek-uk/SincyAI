//! Vault/KMS secret references (TDD 15: 凭证仅保存 Vault 引用或加密密文).
//!
//! Plaintext secrets must never be stored in the database or logs. This type
//! makes that rule explicit: constructing it from anything that looks like a
//! raw secret is rejected at the type level by requiring a `vault://` URI.

use serde::{Deserialize, Serialize};

use crate::SecurityError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultRef(String);

impl VaultRef {
    /// A reference like `vault://transit/.../oauth-token` or `kms://key-id/ciphertext`.
    pub fn new(uri: impl Into<String>) -> Result<Self, SecurityError> {
        let uri = uri.into();
        if uri.starts_with("vault://") || uri.starts_with("kms://") {
            Ok(Self(uri))
        } else {
            Err(SecurityError::PlaintextSecret)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VaultRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_vault_and_kms_uris() {
        assert!(VaultRef::new("vault://kv/oauth/youtube/token").is_ok());
        assert!(VaultRef::new("kms://key-1/ciphertext").is_ok());
    }

    #[test]
    fn rejects_plaintext() {
        assert!(matches!(
            VaultRef::new("ya29.a0AfH6SMB..."),
            Err(SecurityError::PlaintextSecret)
        ));
    }
}
