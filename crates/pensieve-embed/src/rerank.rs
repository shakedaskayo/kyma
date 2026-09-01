//! Local cross-encoder reranker over fastembed's ONNX `TextRerank` (S1.6).
//!
//! Scores `(query, document)` pairs jointly. Used as a final re-scoring stage
//! over a small post-fusion candidate set, not over the corpus. Like
//! [`crate::fastembed::FastembedBackend`] it keeps a small instance pool so
//! concurrent rerank calls don't serialize on one model mutex
//! (`PENSIEVE_RERANK_POOL_SIZE`, default 1). Runs the blocking ONNX inference on a
//! blocking thread.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use tokio::sync::Mutex;

use crate::{EmbedError, RerankBackend};

/// Cross-encoder reranker backed by a pool of fastembed `TextRerank` instances.
pub struct FastembedReranker {
    id: String,
    pool: Vec<Arc<Mutex<TextRerank>>>,
    next: AtomicUsize,
}

impl std::fmt::Debug for FastembedReranker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastembedReranker")
            .field("id", &self.id)
            .field("pool", &self.pool.len())
            .finish()
    }
}

impl FastembedReranker {
    /// Build a reranker for `model_id` (e.g. `"bge-reranker-base"`,
    /// `"bge-reranker-v2-m3"`, `"jina-reranker-v1-turbo-en"`). `model_path`
    /// overrides the ONNX cache dir for air-gapped installs.
    pub async fn new(model_id: &str, model_path: Option<&str>) -> Result<Self, EmbedError> {
        let model = pick_model(model_id)?;
        let interactive = std::io::IsTerminal::is_terminal(&std::io::stderr());
        let pool_size = std::env::var("PENSIEVE_RERANK_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1);
        let mut pool = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            let model = model.clone();
            let model_path = model_path.map(String::from);
            let show_progress = interactive && i == 0;
            let rr = tokio::task::spawn_blocking(move || {
                let mut opts =
                    RerankInitOptions::new(model).with_show_download_progress(show_progress);
                if let Some(path) = model_path {
                    opts = opts.with_cache_dir(path.into());
                }
                TextRerank::try_new(opts)
            })
            .await
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?;
            pool.push(Arc::new(Mutex::new(rr)));
        }
        Ok(Self {
            id: format!("fastembed/{model_id}"),
            pool,
            next: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl RerankBackend for FastembedReranker {
    fn id(&self) -> &str {
        &self.id
    }

    async fn rerank(&self, query: &str, docs: &[String]) -> Result<Vec<f32>, EmbedError> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.pool.len();
        let inner = self.pool[idx].clone();
        let query = query.to_string();
        let docs = docs.to_vec();
        let n = docs.len();
        tokio::task::spawn_blocking(move || {
            let model = inner.blocking_lock();
            // return_documents=false: we only need (index, score). The model
            // scores every pair; map back to input order.
            let results = model
                .rerank(
                    query.as_str(),
                    docs.iter().map(|s| s.as_str()).collect(),
                    false,
                    Some(16),
                )
                .map_err(|e| EmbedError::Internal(e.to_string()))?;
            let mut scores = vec![0.0f32; n];
            for r in results {
                if r.index < n {
                    scores[r.index] = r.score;
                }
            }
            Ok(scores)
        })
        .await
        .map_err(|e| EmbedError::Internal(e.to_string()))?
    }
}

/// Map a short model id to a fastembed [`RerankerModel`].
fn pick_model(id: &str) -> Result<RerankerModel, EmbedError> {
    match id {
        "bge-reranker-base" => Ok(RerankerModel::BGERerankerBase),
        "bge-reranker-v2-m3" => Ok(RerankerModel::BGERerankerV2M3),
        "jina-reranker-v1-turbo-en" => Ok(RerankerModel::JINARerankerV1TurboEn),
        "jina-reranker-v2-base-multilingual" => Ok(RerankerModel::JINARerankerV2BaseMultiligual),
        other => Err(EmbedError::NotConfigured(format!(
            "unknown reranker model '{other}' (known: bge-reranker-base, \
             bge-reranker-v2-m3, jina-reranker-v1-turbo-en, \
             jina-reranker-v2-base-multilingual)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_rerankers_resolve_unknown_errors() {
        for id in [
            "bge-reranker-base",
            "bge-reranker-v2-m3",
            "jina-reranker-v1-turbo-en",
            "jina-reranker-v2-base-multilingual",
        ] {
            assert!(pick_model(id).is_ok(), "{id} should resolve");
        }
        assert!(pick_model("not-a-reranker").is_err());
    }
}
