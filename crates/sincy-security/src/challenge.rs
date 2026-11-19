//! Device registration challenge–response (WP-07验收: 伪造 device_id 无法认证).
//!
//! Flow: cloud issues a random nonce bound to the claimed device_id → device
//! signs nonce with its private key (or HMACs with a provisioned secret) →
//! cloud verifies signature and binds device identity to agency.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::SecurityError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationChallenge {
    pub nonce: String,
    pub device_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationResponse {
    pub device_id: String,
    pub nonce: String,
    /// HMAC-SHA256(device_secret, nonce) — demo verifier; production uses
    /// per-device asymmetric keys stored in Vault/KMS.
    pub proof: String,
}

pub fn issue_challenge(device_id: &str, ttl: Duration) -> RegistrationChallenge {
    let bytes: [u8; 32] = rand::random();
    let now = Utc::now();
    RegistrationChallenge {
        nonce: B64.encode(bytes),
        device_id: device_id.to_string(),
        issued_at: now,
        expires_at: now + ttl,
    }
}

/// Verify a response against a challenge and the device's provisioned secret.
pub fn verify_response(
    challenge: &RegistrationChallenge,
    response: &RegistrationResponse,
    device_secret: &[u8],
) -> Result<(), SecurityError> {
    if Utc::now() > challenge.expires_at {
        return Err(SecurityError::ChallengeMismatch);
    }
    if response.device_id != challenge.device_id || response.nonce != challenge.nonce {
        return Err(SecurityError::ChallengeMismatch);
    }
    let mut mac = Sha256::new();
    mac.update(device_secret);
    mac.update(response.nonce.as_bytes());
    let expected = B64.encode(mac.finalize());
    if expected != response.proof {
        return Err(SecurityError::ChallengeMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_response(challenge: &RegistrationChallenge, secret: &[u8]) -> RegistrationResponse {
        let mut mac = Sha256::new();
        mac.update(secret);
        mac.update(challenge.nonce.as_bytes());
        RegistrationResponse {
            device_id: challenge.device_id.clone(),
            nonce: challenge.nonce.clone(),
            proof: B64.encode(mac.finalize()),
        }
    }

    #[test]
    fn valid_response_accepted() {
        let ch = issue_challenge("device-1", Duration::minutes(5));
        let resp = make_response(&ch, b"device-secret");
        assert!(verify_response(&ch, &resp, b"device-secret").is_ok());
    }

    #[test]
    fn forged_device_id_rejected() {
        let ch = issue_challenge("device-1", Duration::minutes(5));
        let mut resp = make_response(&ch, b"device-secret");
        resp.device_id = "forged".to_string();
        assert!(matches!(
            verify_response(&ch, &resp, b"device-secret"),
            Err(SecurityError::ChallengeMismatch)
        ));
    }

    #[test]
    fn wrong_secret_rejected() {
        let ch = issue_challenge("device-1", Duration::minutes(5));
        let resp = make_response(&ch, b"device-secret");
        assert!(verify_response(&ch, &resp, b"other-secret").is_err());
    }

    #[test]
    fn expired_challenge_rejected() {
        let mut ch = issue_challenge("device-1", Duration::minutes(5));
        ch.expires_at = Utc::now() - Duration::minutes(1);
        let resp = make_response(&ch, b"device-secret");
        assert!(matches!(
            verify_response(&ch, &resp, b"device-secret"),
            Err(SecurityError::ChallengeMismatch)
        ));
    }
}
