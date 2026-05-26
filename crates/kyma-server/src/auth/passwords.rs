//! Argon2id password hashing and verification.
//!
//! `hash_password` produces a PHC-format string (argon2id, random salt).
//! `verify_password` checks a plaintext password against a PHC string.
//!
//! Do NOT use this for high-entropy API tokens — those use SHA-256 (see
//! `db_backend.rs`).

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};

/// Hash a password using argon2id with a fresh random salt.
///
/// Returns a PHC string (e.g. `$argon2id$v=19$...`) suitable for storage.
///
/// # Errors
///
/// Returns an error string if hashing fails (should not happen in practice
/// with a well-configured Argon2 instance).
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

/// Verify a plaintext `password` against a stored PHC string `phc`.
///
/// Returns `true` if the password matches, `false` otherwise (including on
/// any parse/verification error — treats malformed PHC strings as mismatches).
pub fn verify_password(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_verify_roundtrip() {
        let hash = hash_password("correct-horse-battery-staple").unwrap();
        assert!(
            verify_password("correct-horse-battery-staple", &hash),
            "correct password should verify"
        );
    }

    #[test]
    fn wrong_password_fails() {
        let hash = hash_password("supersecret").unwrap();
        assert!(
            !verify_password("wrongpassword", &hash),
            "wrong password should not verify"
        );
    }

    #[test]
    fn two_hashes_of_same_password_differ() {
        let h1 = hash_password("same_password").unwrap();
        let h2 = hash_password("same_password").unwrap();
        assert_ne!(h1, h2, "random salt should produce unique hashes");
        // But both should verify correctly.
        assert!(verify_password("same_password", &h1));
        assert!(verify_password("same_password", &h2));
    }
}
