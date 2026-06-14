//! Embedding-backend construction. Memory is the first server-side consumer of
//! `kyma-embed`; the backend (an ONNX model for fastembed) is built once and
//! shared process-wide via a `OnceCell` so the model loads at most once.

use std::sync::Arc;

use kyma_embed::{EmbeddingBackend, EmbeddingConfig, EmbeddingProvider, RerankBackend};
use tokio::sync::OnceCell;

use crate::error::{MemoryError, Result};

static SHARED: OnceCell<Arc<dyn EmbeddingBackend>> = OnceCell::const_new();
static RERANKER: OnceCell<Option<Arc<dyn RerankBackend>>> = OnceCell::const_new();

/// The process-wide embedding backend, lazily built from
/// [`EmbeddingConfig::from_env`] on first use.
pub async fn shared_embedding() -> Result<Arc<dyn EmbeddingBackend>> {
    SHARED
        .get_or_try_init(|| async { build_embedding_backend(&EmbeddingConfig::from_env()).await })
        .await
        .map(|b| b.clone())
}

/// The process-wide cross-encoder reranker (S1.6), or `None` when reranking is
/// disabled (the default — zero overhead, no model download). Enable by setting
/// `KYMA_RERANK_MODEL` (e.g. `"bge-reranker-base"`); `KYMA_RERANK_MODEL_PATH`
/// overrides the ONNX cache dir for air-gapped installs. Built once on first
/// use; a build failure logs and disables reranking rather than erroring the
/// query path.
pub async fn shared_reranker() -> Option<Arc<dyn RerankBackend>> {
    RERANKER
        .get_or_init(|| async {
            let model = std::env::var("KYMA_RERANK_MODEL").ok()?;
            if model.trim().is_empty() {
                return None;
            }
            let path = std::env::var("KYMA_RERANK_MODEL_PATH").ok();
            match kyma_embed::FastembedReranker::new(&model, path.as_deref()).await {
                Ok(r) => Some(Arc::new(r) as Arc<dyn RerankBackend>),
                Err(e) => {
                    tracing::warn!(error = %e, model = %model, "reranker unavailable; reranking disabled");
                    None
                }
            }
        })
        .await
        .clone()
}

/// Construct an embedding backend from config. Only providers whose
/// `kyma-embed` feature is enabled are available; others return an error.
pub async fn build_embedding_backend(cfg: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingBackend>> {
    match &cfg.provider {
        EmbeddingProvider::Fastembed { id, model_path } => {
            let b = kyma_embed::FastembedBackend::new(id, model_path.as_deref())
                .await
                .map_err(|e| MemoryError::Embed(e.to_string()))?;
            Ok(Arc::new(b))
        }
        other => Err(MemoryError::Embed(format!(
            "embedding provider not enabled in this build: {other:?}; \
             rebuild kyma-memory with the matching kyma-embed feature"
        ))),
    }
}
