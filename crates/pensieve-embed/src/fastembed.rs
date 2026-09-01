use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// ONNX-backed embedding via `fastembed-rs`. Loads the model at construction;
/// inference runs on a tokio blocking thread per batch.
///
/// `fastembed`'s `embed` needs `&mut self`, so each `TextEmbedding` instance
/// serializes its calls. A single instance is fine for schema-RAG (low QPS); to
/// parallelize the bulk `embed_backfill` workload across several concurrent jobs
/// we hold a **pool** of instances and round-robin across them. The pool size is
/// `PENSIEVE_EMBED_POOL_SIZE` (default 1 — each extra instance duplicates the model
/// weights in memory, so opt in for high-throughput backfills).
pub struct FastembedBackend {
    id: String,
    dimension: u16,
    pool: Vec<Arc<Mutex<TextEmbedding>>>,
    next: AtomicUsize,
}

impl std::fmt::Debug for FastembedBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastembedBackend")
            .field("id", &self.id)
            .field("dimension", &self.dimension)
            .finish()
    }
}

impl FastembedBackend {
    /// `model_id` is the short name (e.g., `"bge-small-en-v1.5"`).
    /// `model_path` optionally points at a pre-downloaded ONNX dir for
    /// air-gapped deployments (env `PENSIEVE_EMBED_MODEL_PATH`).
    pub async fn new(model_id: &str, model_path: Option<&str>) -> Result<Self, EmbedError> {
        let em = pick_model(model_id)?;
        let dimension = em_dimension(&em);
        // First run downloads the ONNX model (tens of MB) — without a notice
        // it reads as a hang. Say so up front and let fastembed render its
        // progress bar when a human is watching.
        let interactive = std::io::IsTerminal::is_terminal(&std::io::stderr());
        if !model_cached(model_path) {
            eprintln!("downloading the embedding model ({model_id}, one-time, ~30–130 MB)…");
        }
        let pool_size = std::env::var("PENSIEVE_EMBED_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1);
        let mut pool = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            let em = em.clone();
            let model_path = model_path.map(String::from);
            // Show the download progress bar only for the first instance (it
            // populates the cache; the rest load from disk).
            let show_progress = interactive && i == 0;
            let model = tokio::task::spawn_blocking(move || {
                let mut opts = InitOptions::new(em).with_show_download_progress(show_progress);
                if let Some(path) = model_path {
                    opts = opts.with_cache_dir(path.into());
                }
                TextEmbedding::try_new(opts)
            })
            .await
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?;
            pool.push(Arc::new(Mutex::new(model)));
        }
        Ok(Self {
            id: format!("fastembed/{model_id}"),
            dimension,
            pool,
            next: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl EmbeddingBackend for FastembedBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn dimension(&self) -> u16 {
        self.dimension
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        // Round-robin across the pool so concurrent callers use different
        // instances instead of all contending on one mutex.
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.pool.len();
        let inner = self.pool[idx].clone();
        let owned: Vec<String> = texts.to_vec();
        let dim = self.dimension;
        tokio::task::spawn_blocking(move || {
            let guard = inner.blocking_lock();
            let vecs = guard
                .embed(owned, None)
                .map_err(|e| EmbedError::Request(e.to_string()))?;
            for v in &vecs {
                if v.len() != dim as usize {
                    return Err(EmbedError::DimensionMismatch {
                        got: v.len() as u16,
                        expected: dim,
                    });
                }
            }
            Ok(vecs)
        })
        .await
        .map_err(|e| EmbedError::Internal(e.to_string()))?
    }
}

/// Whether the model cache already exists (mirrors fastembed's resolution:
/// explicit path → `FASTEMBED_CACHE_PATH` → `.fastembed_cache` in the cwd).
/// Existence of a non-empty cache dir is a good-enough "no download coming"
/// signal — the goal is only to avoid a misleading silent first run.
fn model_cached(model_path: Option<&str>) -> bool {
    let dir = model_path
        .map(String::from)
        .or_else(|| std::env::var("FASTEMBED_CACHE_PATH").ok())
        .unwrap_or_else(|| ".fastembed_cache".to_string());
    std::fs::read_dir(dir)
        .map(|mut d| d.next().is_some())
        .unwrap_or(false)
}

fn pick_model(id: &str) -> Result<EmbeddingModel, EmbedError> {
    match id {
        // Default — small, fast, 384-d.
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-base-en-v1.5" => Ok(EmbeddingModel::BGEBaseENV15),
        "bge-large-en-v1.5" => Ok(EmbeddingModel::BGELargeENV15),
        "all-MiniLM-L6-v2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "all-MiniLM-L12-v2" => Ok(EmbeddingModel::AllMiniLML12V2),
        // Stronger 2026 options. nomic-v1.5 is Matryoshka-trained (768-d, MRL-
        // truncatable) — pair it with `MatryoshkaTruncate` for a cheap 256-d
        // first-stage index + full-dim rerank.
        "nomic-embed-text-v1.5" => Ok(EmbeddingModel::NomicEmbedTextV15),
        "gte-base-en-v1.5" => Ok(EmbeddingModel::GTEBaseENV15),
        "gte-large-en-v1.5" => Ok(EmbeddingModel::GTELargeENV15),
        "mxbai-embed-large-v1" => Ok(EmbeddingModel::MxbaiEmbedLargeV1),
        // Code-specialized (good for code/graph corpora).
        "jina-embeddings-v2-base-code" => Ok(EmbeddingModel::JinaEmbeddingsV2BaseCode),
        other => Err(EmbedError::NotConfigured(format!(
            "unknown fastembed model: {other} (known: bge-small-en-v1.5, \
             bge-base-en-v1.5, bge-large-en-v1.5, all-MiniLM-L6-v2, \
             all-MiniLM-L12-v2, nomic-embed-text-v1.5, gte-base-en-v1.5, \
             gte-large-en-v1.5, mxbai-embed-large-v1, jina-embeddings-v2-base-code)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_map_to_expected_dimensions() {
        let cases = [
            ("bge-small-en-v1.5", 384),
            ("bge-base-en-v1.5", 768),
            ("bge-large-en-v1.5", 1024),
            ("all-MiniLM-L6-v2", 384),
            ("all-MiniLM-L12-v2", 384),
            ("nomic-embed-text-v1.5", 768),
            ("gte-base-en-v1.5", 768),
            ("gte-large-en-v1.5", 1024),
            ("mxbai-embed-large-v1", 1024),
            ("jina-embeddings-v2-base-code", 768),
        ];
        for (id, dim) in cases {
            let em = pick_model(id).unwrap_or_else(|_| panic!("pick_model({id})"));
            assert_eq!(em_dimension(&em), dim, "dimension for {id}");
        }
    }

    #[test]
    fn unknown_model_errors() {
        assert!(pick_model("not-a-real-model").is_err());
    }
}

fn em_dimension(em: &EmbeddingModel) -> u16 {
    match em {
        EmbeddingModel::BGESmallENV15 => 384,
        EmbeddingModel::BGEBaseENV15 => 768,
        EmbeddingModel::BGELargeENV15 => 1024,
        EmbeddingModel::AllMiniLML6V2 => 384,
        EmbeddingModel::AllMiniLML12V2 => 384,
        EmbeddingModel::NomicEmbedTextV15 => 768,
        EmbeddingModel::GTEBaseENV15 => 768,
        EmbeddingModel::GTELargeENV15 => 1024,
        EmbeddingModel::MxbaiEmbedLargeV1 => 1024,
        EmbeddingModel::JinaEmbeddingsV2BaseCode => 768,
        other => unreachable!(
            "em_dimension: pick_model accepted model {other:?} but em_dimension has no arm. Add the dimension here."),
    }
}
