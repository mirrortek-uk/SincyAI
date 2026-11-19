//! Password hashing with Argon2id.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::SecurityError;

pub fn hash_password(password: &str) -> Result<String, SecurityError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| SecurityError::InvalidToken(e.to_string()))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, encoded: &str) -> Result<bool, SecurityError> {
    let parsed = PasswordHash::new(encoded).map_err(|_| SecurityError::BadPassword)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify() {
        let h = hash_password("s3cret!").expect("hash");
        assert!(verify_password("s3cret!", &h).expect("verify"));
        assert!(!verify_password("wrong", &h).expect("verify"));
    }
}
