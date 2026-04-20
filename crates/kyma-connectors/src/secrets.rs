//! Secret-store abstraction. Implementation lands in Task 4.

pub trait SecretStore: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, SecretError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
}
