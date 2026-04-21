use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Arc;
use tokio::sync::Mutex;

/// ONNX-backed embedding via fastembed-rs. Loads the model at construction;
/// inference runs on a tokio blocking thread per batch.
pub struct FastembedBackend {
    id: String,
    dimension: u16,
    inner: Arc<Mutex<TextEmbedding>>,
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
    /// air-gapped deployments (env `KYMA_EMBED_MODEL_PATH`).
    pub async fn new(model_id: &str, model_path: Option<&str>)
        -> Result<Self, EmbedError>
    {
        let em = pick_model(model_id)?;
        let dimension = em_dimension(&em);
        let mut opts = InitOptions::new(em);
        if let Some(path) = model_path {
            opts = opts.with_cache_dir(path.into());
        }
        let model = tokio::task::spawn_blocking(move || TextEmbedding::try_new(opts))
            .await
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?
            .map_err(|e| EmbedError::ModelLoad(e.to_string()))?;
        Ok(Self {
            id: format!("fastembed/{model_id}"),
            dimension,
            inner: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl EmbeddingBackend for FastembedBackend {
    fn id(&self) -> &str { &self.id }
    fn dimension(&self) -> u16 { self.dimension }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() { return Ok(vec![]); }
        let inner = self.inner.clone();
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
                        got: v.len() as u16, expected: dim,
                    });
                }
            }
            Ok(vecs)
        })
        .await
        .map_err(|e| EmbedError::Internal(e.to_string()))?
    }
}

fn pick_model(id: &str) -> Result<EmbeddingModel, EmbedError> {
    match id {
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-base-en-v1.5"  => Ok(EmbeddingModel::BGEBaseENV15),
        "all-MiniLM-L6-v2"  => Ok(EmbeddingModel::AllMiniLML6V2),
        other => Err(EmbedError::NotConfigured(format!(
            "unknown fastembed model: {other}"))),
    }
}

fn em_dimension(em: &EmbeddingModel) -> u16 {
    match em {
        EmbeddingModel::BGESmallENV15 => 384,
        EmbeddingModel::BGEBaseENV15  => 768,
        EmbeddingModel::AllMiniLML6V2 => 384,
        _ => 384,
    }
}
