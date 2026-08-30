//! Symmetric encryption for credentials stored at rest.
//!
//! AES-256-GCM with a key derived from the `KYMA_SECRET_KEY` environment
//! variable. Ciphertexts are stored as `nonce(12 bytes) || ciphertext(N)` so a
//! single `bytea` column round-trips through Postgres without a separate
//! nonce column. The key is loaded once at server start and held in an `Arc`
//! shared with every subsystem that needs to encrypt or decrypt.
//!
//! Production deployments should set `KYMA_SECRET_KEY` to a base64-encoded
//! random 32 bytes (e.g. `openssl rand -base64 32`). Shorter values are
//! SHA-256 stretched so dev environments can use any string; the server logs
//! a warning when stretching kicks in.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// A symmetric encryption helper. Cheap to clone (the inner key is `Copy`).
#[derive(Clone)]
pub struct Crypto {
    key: Key<Aes256Gcm>,
}

impl Crypto {
    /// Build from the `KYMA_SECRET_KEY` env var. Accepts base64 (preferred,
    /// `openssl rand -base64 32`) or any plain string (SHA-256 stretched).
    pub fn from_env() -> Result<Self> {
        let raw = std::env::var("KYMA_SECRET_KEY")
            .context("KYMA_SECRET_KEY env var not set — required for credential encryption")?;
        Ok(Self::from_secret(&raw))
    }

    /// Build from a secret string of any length (sha256-stretched to 32 bytes).
    pub fn from_secret(secret: &str) -> Self {
        let bytes = STANDARD
            .decode(secret.as_bytes())
            .ok()
            .and_then(|b| if b.len() >= 32 { Some(b) } else { None })
            .unwrap_or_else(|| {
                let mut h = Sha256::new();
                h.update(secret.as_bytes());
                h.finalize().to_vec()
            });
        let key = *Key::<Aes256Gcm>::from_slice(&bytes[..32]);
        Self { key }
    }

    /// Encrypt `plaintext`, returning `nonce(12) || ciphertext`.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(&self.key);
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|e| anyhow!("aes-gcm encrypt: {e}"))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend(ct);
        Ok(out)
    }

    /// Decrypt a blob produced by [`Self::encrypt`].
    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < 13 {
            return Err(anyhow!("ciphertext too short"));
        }
        let cipher = Aes256Gcm::new(&self.key);
        let (nonce_bytes, ct) = blob.split_at(12);
        cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|e| anyhow!("aes-gcm decrypt: {e}"))
    }

    /// Load from `KYMA_SECRET_KEY` env var, or generate + persist a random key
    /// to `key_path` on first use (`0600` permissions). Safe for local mode where
    /// no env var is configured.
    pub fn from_env_or_file(key_path: &str) -> Result<Self> {
        if let Ok(raw) = std::env::var("KYMA_SECRET_KEY") {
            return Ok(Self::from_secret(&raw));
        }
        let path = std::path::Path::new(key_path);
        if path.exists() {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading key file {key_path}"))?;
            return Ok(Self::from_secret(raw.trim()));
        }
        // First boot — generate and persist.
        let key = generate_key();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        std::fs::write(path, &key).with_context(|| format!("writing key file {key_path}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("setting permissions on {key_path}"))?;
        }
        eprintln!("kyma: generated local secret key at {key_path} (first boot)");
        Ok(Self::from_secret(&key))
    }
}

/// Generate a fresh `KYMA_SECRET_KEY` value: base64 of 32 random bytes — the
/// format [`Crypto::from_secret`] accepts without SHA-256 stretching. Used by
/// installers/services that mint a key for a deployed environment.
pub fn generate_key() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    STANDARD.encode(bytes)
}

/// SHA-256 of `data`, hex-encoded (lowercase, 64 chars). The content-hash key
/// for the embedding-backfill cache: identical text always maps to the same
/// hash, so re-ingested or duplicate text is embedded at most once. Stable and
/// deterministic across processes and platforms.
pub fn content_hash_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_is_full_strength_base64() {
        let k = generate_key();
        let decoded = STANDARD.decode(k.as_bytes()).expect("base64");
        assert_eq!(decoded.len(), 32, "exactly 32 random bytes");
        assert_ne!(generate_key(), generate_key(), "random per call");
    }

    #[test]
    fn roundtrip() {
        let c = Crypto::from_secret("test-dev-key");
        let pt = b"hello, world";
        let ct = c.encrypt(pt).unwrap();
        // Distinct from plaintext, nonce-prefixed.
        assert!(ct.len() >= 12 + pt.len());
        assert!(&ct[12..] != pt);
        let round = c.decrypt(&ct).unwrap();
        assert_eq!(round, pt);
    }

    #[test]
    fn distinct_nonces() {
        let c = Crypto::from_secret("test-dev-key");
        let a = c.encrypt(b"x").unwrap();
        let b = c.encrypt(b"x").unwrap();
        // Random nonces → distinct ciphertexts even for the same plaintext.
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_is_deterministic_and_known() {
        // Same input → same hash; different input → different hash.
        assert_eq!(content_hash_hex(b"hello"), content_hash_hex(b"hello"));
        assert_ne!(content_hash_hex(b"hello"), content_hash_hex(b"world"));
        // 64 lowercase hex chars.
        let h = content_hash_hex(b"hello");
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Known SHA-256("") and SHA-256("abc") vectors pin the encoding.
        assert_eq!(
            content_hash_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            content_hash_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
