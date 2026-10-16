//! Secrets: passwords, and the random tokens of sessions, API tokens and
//! invitations.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// A new random token, with a prefix that says what it is for.
pub fn token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// What the database keeps of a token: its SHA-256, in hex.
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Hashes a password with Argon2id. Slow on purpose: call it off the
/// async runtime.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hashing a password: {e}"))?;
    Ok(hash.to_string())
}

/// Whether `password` matches `hash`. Slow on purpose, as above.
pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_verify() {
        let hash = hash_password("correct horse").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("battery staple", &hash));
        assert!(!verify_password("correct horse", "not a hash"));
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let (a, b) = (token("stm_"), token("stm_"));
        assert!(a.starts_with("stm_"));
        assert_ne!(a, b);
        assert_eq!(token_hash(&a), token_hash(&a));
        assert_eq!(token_hash(&a).len(), 64);
    }
}
