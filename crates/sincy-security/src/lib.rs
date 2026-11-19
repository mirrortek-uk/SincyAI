//! sincy-security: identity, secrets handling and adapter integrity primitives.
//!
//! Hard rules (TDD 15):
//! - secrets (Platform App keys, OAuth tokens) are stored ONLY as Vault
//!   references or KMS ciphertext — see [`vault::VaultRef`];
//! - logs must never contain secrets;
//! - every device has an independent key/certificate; registration uses a
//!   challenge–response (WP-07);
//! - no captcha solving, device-fingerprint forgery or multi-account
//!   anti-association features are implemented here or anywhere else.

pub mod challenge;
pub mod jwt;
pub mod password;
pub mod vault;

pub use challenge::{RegistrationChallenge, RegistrationResponse};
pub use jwt::{Claims, JwtIssuer};
pub use password::{hash_password, verify_password};
pub use vault::VaultRef;

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("token expired")]
    TokenExpired,
    #[error("password verification failed")]
    BadPassword,
    #[error("challenge mismatch or expired")]
    ChallengeMismatch,
    #[error("vault reference required: plaintext secrets are forbidden")]
    PlaintextSecret,
}
