//! Secret resolution.
//!
//! Slice-1 ships two implementations: a pass-through literal + env-var
//! reference resolver (`EnvSecretStore`), and a trait so Vault / AWS SM
//! can plug in later with no callsite changes.

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
}

pub trait SecretStore: Send + Sync {
    /// Resolve a reference.
    ///
    /// - `"$env:NAME"` → value of environment variable `NAME`.
    /// - Anything else → returned verbatim.
    fn resolve(&self, reference: &str) -> Result<String, SecretError>;
}

#[derive(Default, Clone, Copy)]
pub struct EnvSecretStore;

impl SecretStore for EnvSecretStore {
    fn resolve(&self, reference: &str) -> Result<String, SecretError> {
        if let Some(name) = reference.strip_prefix("$env:") {
            std::env::var(name).map_err(|_| SecretError::NotFound(name.to_string()))
        } else {
            Ok(reference.to_string())
        }
    }
}
