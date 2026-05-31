//! Error type for the memory layer.

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("embedding: {0}")]
    Embed(String),
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("write: {0}")]
    Write(String),
    #[error("ingest: {0}")]
    Ingest(String),
}

pub type Result<T> = std::result::Result<T, MemoryError>;
