//! Short-lived access tokens (TDD 15: 短期访问令牌).

use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::SecurityError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub agency_id: String,
    pub roles: Vec<String>,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
}

pub struct JwtIssuer {
    encoding: EncodingKey,
    decoding: DecodingKey,
    ttl: Duration,
}

impl JwtIssuer {
    pub fn new(secret: &[u8], ttl_minutes: i64) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            ttl: Duration::minutes(ttl_minutes),
        }
    }

    pub fn issue(
        &self,
        subject: &str,
        agency_id: &str,
        roles: Vec<String>,
    ) -> Result<String, SecurityError> {
        let now = Utc::now();
        let claims = Claims {
            sub: subject.to_string(),
            agency_id: agency_id.to_string(),
            roles,
            exp: (now + self.ttl).timestamp(),
            iat: now.timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
        };
        encode(&Header::default(), &claims, &self.encoding)
            .map_err(|e| SecurityError::InvalidToken(e.to_string()))
    }

    pub fn verify(&self, token: &str) -> Result<Claims, SecurityError> {
        let mut validation = Validation::default();
        validation.validate_exp = true;
        let data =
            decode::<Claims>(token, &self.decoding, &validation).map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => SecurityError::TokenExpired,
                _ => SecurityError::InvalidToken(e.to_string()),
            })?;
        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify() {
        let issuer = JwtIssuer::new(b"test-secret", 5);
        let token = issuer
            .issue("user-1", "agency-1", vec!["agency_owner".into()])
            .expect("issue");
        let claims = issuer.verify(&token).expect("verify");
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.agency_id, "agency-1");
    }

    #[test]
    fn wrong_secret_rejected() {
        let a = JwtIssuer::new(b"secret-a", 5);
        let b = JwtIssuer::new(b"secret-b", 5);
        let token = a.issue("u", "ag", vec![]).expect("issue");
        assert!(b.verify(&token).is_err());
    }
}
