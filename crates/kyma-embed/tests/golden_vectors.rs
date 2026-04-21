//! FastembedBackend MUST be bit-exact deterministic. We commit a golden
//! file of (input → first-3 floats + dimension) and assert on it. If
//! fastembed ever changes model output this test surfaces it in CI
//! instead of silently invalidating the replay cache.

#![cfg(feature = "fastembed-backend")]

use kyma_embed::{fastembed::FastembedBackend, EmbeddingBackend};

#[tokio::test]
async fn bge_small_en_v1_5_matches_golden() {
    let b = FastembedBackend::new("bge-small-en-v1.5", None)
        .await
        .expect("model download + load");

    assert_eq!(b.id(), "fastembed/bge-small-en-v1.5");
    assert_eq!(b.dimension(), 384);

    let out = b.embed(&["hello world".into()]).await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].len(), 384);

    // Bge models output L2-normalized vectors. Golden prefix committed
    // after first passing run; bump alongside any intentional model
    // bump. This catches: model download drift, fastembed upgrade,
    // or accidental tokenizer change.
    let prefix: Vec<f32> = out[0][..3].to_vec();
    insta::assert_debug_snapshot!("bge_small_en_v1_5_hello_world_prefix", prefix);

    let norm = out[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "expected L2-normalized, got {norm}");
}
