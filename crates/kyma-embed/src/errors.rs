use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("embedding backend not configured: {0}")]
    NotConfigured(String),

    #[error("embedding request failed: {0}")]
    Request(String),

    #[error("embedding backend returned dimension {got}, expected {expected}")]
    DimensionMismatch { got: u16, expected: u16 },

    #[error("embedding model load failed: {0}")]
    ModelLoad(String),

    #[error("internal: {0}")]
    Internal(String),
}
