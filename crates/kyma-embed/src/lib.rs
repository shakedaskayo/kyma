//! Embedding primitives for kyma.
//!
//! `EmbeddingBackend` is the single trait. The default impl is
//! `FastembedBackend` (ONNX, runs in-process, no external service).
//! Provider-backed impls (Ollama, OpenAI-compatible, Gemini) are
//! feature-gated.

#![forbid(unsafe_code)]

pub mod config;
pub mod errors;

#[cfg(feature = "fastembed-backend")]
pub mod fastembed;

#[cfg(feature = "ollama")]
pub mod ollama;

#[cfg(feature = "openai-compat")]
pub mod openai_compat;

#[cfg(feature = "gemini")]
pub mod gemini;

pub mod matryoshka;

#[cfg(feature = "fastembed-backend")]
pub mod rerank;

pub use config::{EmbeddingConfig, EmbeddingProvider};
pub use errors::EmbedError;
pub use matryoshka::MatryoshkaTruncate;

use async_trait::async_trait;

/// Turns text into dense vectors.
///
/// Implementations MUST be deterministic for the same input when
/// `backend.id()` stays the same — this is a load-bearing property
/// of the kyma replay cache.
#[async_trait]
pub trait EmbeddingBackend: Send + Sync + std::fmt::Debug {
    /// Stable identifier, e.g. `"fastembed/bge-small-en-v1.5"` or
    /// `"openai/text-embedding-3-small"`. Mixing IDs across writes
    /// is a correctness bug (distance-space mismatch) — the engine
    /// tags every vector column with its `model_id`.
    fn id(&self) -> &str;

    /// Output dimension. Must be stable for the lifetime of the instance.
    fn dimension(&self) -> u16;

    /// Compute embeddings for a batch. Order of outputs matches inputs.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Cross-encoder reranker: scores `(query, document)` pairs jointly (S1.6).
///
/// Unlike a bi-encoder embedding (each side encoded independently then cosine'd),
/// a cross-encoder attends over the concatenated pair, so it is more accurate but
/// far costlier — used only as a final re-scoring stage over a small candidate
/// set (e.g. the post-fusion top-N), never over the whole corpus.
#[async_trait]
pub trait RerankBackend: Send + Sync + std::fmt::Debug {
    /// Stable identifier, e.g. `"fastembed/bge-reranker-base"`.
    fn id(&self) -> &str;

    /// Relevance score for each document against `query`, aligned to the input
    /// order (NOT sorted). Higher is more relevant. The caller re-ranks.
    async fn rerank(&self, query: &str, docs: &[String]) -> Result<Vec<f32>, EmbedError>;
}

#[cfg(feature = "fastembed-backend")]
pub use fastembed::FastembedBackend;

#[cfg(feature = "fastembed-backend")]
pub use rerank::FastembedReranker;

#[cfg(feature = "ollama")]
pub use ollama::OllamaBackend;

#[cfg(feature = "openai-compat")]
pub use openai_compat::OpenAICompatBackend;

#[cfg(feature = "gemini")]
pub use gemini::GeminiBackend;
