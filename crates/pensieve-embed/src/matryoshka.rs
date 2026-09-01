//! Matryoshka (MRL) truncation wrapper for embedding backends.
//!
//! Matryoshka-trained models (nomic-embed-text-v1.5, EmbeddingGemma, BGE-M3, …)
//! pack the most information into the leading dimensions, so a *prefix* of the
//! full embedding is itself a usable, lower-dimensional embedding after
//! re-normalization. Truncating to e.g. 256-d cuts vector storage + ANN scan
//! cost ~1.5× at 384-d (more at higher dims) and compounds with RaBitQ
//! quantization — the first-stage IVF/RaBitQ index is built on the truncated
//! prefix, while the full-dimensional vectors back the exact rerank.
//!
//! [`MatryoshkaTruncate`] wraps any [`EmbeddingBackend`]: it requests the full
//! embedding from the inner backend, keeps the first `dim` components, and
//! L2-renormalizes (so cosine distance is well-defined on the unit sphere).
//!
//! ## The model-id contract
//!
//! Truncating changes the distance space, so the wrapped backend reports a
//! DISTINCT id — `"{inner_id}@{dim}"` (e.g. `fastembed/bge-small-en-v1.5@256`).
//! This is load-bearing: the engine tags every vector column and every ANN
//! sidecar with its `model_id` and refuses to mix ids in one index. A truncated
//! vector and a full vector from the same base model are NOT comparable, and
//! the distinct id is what keeps them apart.

use crate::{EmbedError, EmbeddingBackend};

/// Wraps an [`EmbeddingBackend`], truncating its output to `dim` leading
/// components and L2-renormalizing. See the module docs.
#[derive(Debug)]
pub struct MatryoshkaTruncate<B: EmbeddingBackend> {
    inner: B,
    dim: u16,
    id: String,
}

impl<B: EmbeddingBackend> MatryoshkaTruncate<B> {
    /// Wrap `inner`, truncating to `dim` dimensions.
    ///
    /// # Errors
    /// Errors if `dim` is 0 or exceeds the inner backend's dimension (a prefix
    /// can't be longer than the whole).
    pub fn new(inner: B, dim: u16) -> Result<Self, EmbedError> {
        let full = inner.dimension();
        if dim == 0 || dim > full {
            return Err(EmbedError::Internal(format!(
                "matryoshka dim {dim} must be in 1..={full} (inner {})",
                inner.id()
            )));
        }
        let id = format!("{}@{dim}", inner.id());
        Ok(Self { inner, dim, id })
    }

    /// The wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

/// Truncate `v` to its first `dim` components and L2-normalize. A (near-)zero
/// prefix is left as zeros (no meaningful direction), matching the engine's
/// normalization convention elsewhere.
pub(crate) fn truncate_normalize(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out: Vec<f32> = v.iter().take(dim).copied().collect();
    let norm = out
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm > 1e-12 {
        for x in &mut out {
            *x = (*x as f64 / norm) as f32;
        }
    }
    out
}

#[async_trait::async_trait]
impl<B: EmbeddingBackend> EmbeddingBackend for MatryoshkaTruncate<B> {
    fn id(&self) -> &str {
        &self.id
    }

    fn dimension(&self) -> u16 {
        self.dim
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let full = self.inner.embed(texts).await?;
        let dim = self.dim as usize;
        Ok(full
            .into_iter()
            .map(|v| truncate_normalize(&v, dim))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeBackend {
        dim: u16,
    }

    #[async_trait::async_trait]
    impl EmbeddingBackend for FakeBackend {
        fn id(&self) -> &str {
            "fake/model"
        }
        fn dimension(&self) -> u16 {
            self.dim
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            // Deterministic ramp per text: [1,2,3,...,dim] so truncation is visible.
            Ok(texts
                .iter()
                .map(|_| (1..=self.dim).map(|i| i as f32).collect())
                .collect())
        }
    }

    #[test]
    fn id_encodes_truncated_dim() {
        let m = MatryoshkaTruncate::new(FakeBackend { dim: 384 }, 256).unwrap();
        assert_eq!(m.id(), "fake/model@256");
        assert_eq!(m.dimension(), 256);
    }

    #[test]
    fn rejects_bad_dims() {
        assert!(MatryoshkaTruncate::new(FakeBackend { dim: 384 }, 0).is_err());
        assert!(MatryoshkaTruncate::new(FakeBackend { dim: 384 }, 385).is_err());
        // Equal is allowed (a no-op truncation, still renormalized).
        assert!(MatryoshkaTruncate::new(FakeBackend { dim: 384 }, 384).is_ok());
    }

    #[tokio::test]
    async fn embed_truncates_and_unit_normalizes() {
        let m = MatryoshkaTruncate::new(FakeBackend { dim: 8 }, 4).unwrap();
        let out = m.embed(&["x".to_string()]).await.unwrap();
        assert_eq!(out.len(), 1);
        let v = &out[0];
        assert_eq!(v.len(), 4, "truncated to 4 dims");
        // Unit norm.
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5, "norm = {n}");
        // Direction preserved: prefix [1,2,3,4] normalized.
        let raw = [1.0f32, 2.0, 3.0, 4.0];
        let rn = (raw.iter().map(|x| x * x).sum::<f32>()).sqrt();
        for (got, r) in v.iter().zip(raw.iter()) {
            assert!((got - r / rn).abs() < 1e-5);
        }
    }

    #[test]
    fn truncate_normalize_handles_zero_prefix() {
        let out = truncate_normalize(&[0.0, 0.0, 5.0, 6.0], 2);
        assert_eq!(out, vec![0.0, 0.0]); // zero prefix → zeros, no NaN.
    }
}
