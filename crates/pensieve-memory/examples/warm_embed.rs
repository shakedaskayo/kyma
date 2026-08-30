//! Downloads + warms the configured embedding backend (so the model is cached
//! on disk before the server's first memory write). Run from the repo root:
//!   cargo run -p kyma-memory --example warm_embed

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = kyma_embed::EmbeddingConfig::from_env();
    let backend = kyma_memory::build_embedding_backend(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let v = backend
        .embed(&["warmup".to_string()])
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("embedded ok: id={} dim={}", backend.id(), v[0].len());
    Ok(())
}
