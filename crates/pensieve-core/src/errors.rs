//! Typed errors used throughout the engine.
//!
//! All fallible public functions return [`Result<T>`] (aliased to
//! `std::result::Result<T, Error>`). Subsystem-specific error types
//! (`CatalogError`, `FormatError`, ...) flatten into [`Error`] via `#[from]`.

use thiserror::Error;

/// Top-level engine error. Subsystem errors convert in via `From`.
#[derive(Debug, Error)]
pub enum Error {
    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    #[error("format error: {0}")]
    Format(#[from] FormatError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("plan error: {0}")]
    Plan(String),

    #[error("execution error: {0}")]
    Execution(String),

    #[error("ingest error: {0}")]
    Ingest(#[from] IngestError),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
pub enum CatalogError {
    /// Optimistic CAS on the snapshot pointer failed — another writer raced
    /// and won. Caller should re-read the current snapshot, re-lineage, retry.
    #[error("concurrent update: parent snapshot no longer current; retry")]
    Conflict,

    #[error("table not found: {database}.{name}")]
    TableNotFound { database: String, name: String },

    #[error("database not found: {0}")]
    DatabaseNotFound(String),

    #[error("schema snapshot not found: {0}")]
    SchemaSnapshotNotFound(uuid::Uuid),

    #[error("dashboard not found: {id}")]
    DashboardNotFound { id: uuid::Uuid },

    #[error("graph registration not found: {database}.{name}")]
    GraphNotFound { database: String, name: String },

    #[error("sql error: {0}")]
    Sql(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("unsupported format version: got {got}, max supported {max_supported}")]
    UnsupportedVersion { got: u32, max_supported: u32 },

    #[error("corrupt extent at {path}: {detail}")]
    Corrupt { path: String, detail: String },

    #[error("invalid magic bytes")]
    InvalidMagic,

    #[error("column type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: String, got: String },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object store error: {0}")]
    ObjectStore(String),

    #[error("cache error: {0}")]
    Cache(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("parse failed: {0}")]
    SyntaxError(String),

    #[error("unsupported operator: {0}")]
    UnsupportedOperator(String),
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("wal write failed: {0}")]
    Wal(String),

    #[error("schema coercion failed: {0}")]
    Coercion(String),

    #[error("backpressure: queue full for table {0}")]
    Backpressure(String),

    #[error("duplicate idempotency key: {0}")]
    Duplicate(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The engine-wide `Result` alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
