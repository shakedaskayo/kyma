//! Artifact **content** indexing: extract text from text-like artifact blobs,
//! chunk + embed it, and append rows to `artifacts.artifact_chunks` so the
//! unified data-mode search (lexical token-index + cosine vector legs) can
//! discover artifact content — not just artifact names/properties.
//!
//! Mirrors the node writer in `lib.rs`: provision-on-demand + `WritePath`
//! append with a per-artifact idempotency key, so a re-sync never duplicates
//! chunks and (because the key is checked *before* the blob fetch) never
//! re-downloads or re-embeds an already-indexed artifact.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::catalog::{Catalog, TableConfig};
use kyma_core::segment_format::SegmentFormat;
use kyma_embed::EmbeddingBackend;
use kyma_ingest_core::WritePath;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use serde_json::{json, Value};

use crate::{ARTIFACTS_DB, PRODUCER_ATTACHED_SOURCES};

/// Chunk table next to `artifact_nodes`/`artifact_edges` in the catch-all DB.
pub const CHUNKS_TABLE: &str = "artifact_chunks";

/// Artifact classes whose blobs are text-extraction candidates. Anything else
/// (images, binaries, parquet, …) is skipped without a fetch.
pub const TEXT_CLASSES: &[&str] = &["log", "file", "text", "doc", "config"];

/// Blobs larger than this are skipped — a runaway log should not stall the
/// sync loop or flood the embedder.
pub const MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;

/// Target chunk size in characters (line-aligned, see [`chunk_text`]).
pub const CHUNK_CHARS: usize = 1500;

/// Per-artifact cap on indexed chunks; the head of a huge file still gets
/// indexed, the tail is dropped rather than blowing the embed budget.
pub const MAX_CHUNKS_PER_ARTIFACT: usize = 64;

/// Bytes sniffed by [`looks_textual`].
const SNIFF_BYTES: usize = 8 * 1024;

pub fn chunks_schema(dim: i32) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("artifact_id", DataType::Utf8, false),
        Field::new("object_path", DataType::Utf8, true),
        Field::new("source", DataType::Utf8, true),
        Field::new("artifact_class", DataType::Utf8, true),
        Field::new("table_ref", DataType::Utf8, true),
        Field::new("chunk_index", DataType::Int64, true),
        Field::new("content", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
            true,
        ),
        Field::new("created_at", DataType::Utf8, true),
    ]))
}

/// Whether an artifact class is a candidate for content extraction.
pub fn is_text_class(class: &str) -> bool {
    TEXT_CLASSES.contains(&class)
}

/// Cheap binary sniff over the head of the blob: valid UTF-8 (allowing one
/// truncated trailing codepoint) and no NUL bytes.
pub fn looks_textual(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte codepoint cut at the sniff boundary is fine; an error
        // mid-buffer is binary.
        Err(e) => e.valid_up_to() + 4 >= head.len(),
    }
}

/// Split text into line-aligned chunks of roughly [`CHUNK_CHARS`] characters,
/// capped at [`MAX_CHUNKS_PER_ARTIFACT`]. A single line longer than the target
/// becomes its own (oversized) chunk rather than being split mid-line.
pub fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if !cur.is_empty() && cur.chars().count() + line.chars().count() + 1 > CHUNK_CHARS {
            chunks.push(std::mem::take(&mut cur));
            if chunks.len() >= MAX_CHUNKS_PER_ARTIFACT {
                return chunks;
            }
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks.truncate(MAX_CHUNKS_PER_ARTIFACT);
    chunks
}

/// Stable per-artifact idempotency key: same artifact + same content hash
/// ⇒ same key ⇒ the ingest ledger replays the ack instead of re-indexing.
pub fn idempotency_key(rec: &ArtifactRecord) -> String {
    let id = match rec.id {
        Some(u) => u.to_string(),
        None => rec.object_path.clone(),
    };
    let content_tag = rec.sha256.clone().unwrap_or_else(|| rec.size_bytes.to_string());
    format!("artifact-chunks:{}:{id}:{content_tag}", rec.tenant_id)
}

/// Shape one chunk into an `artifact_chunks` row (columns match
/// [`chunks_schema`]). Pure — unit-testable without a catalog.
pub fn chunk_row(
    rec: &ArtifactRecord,
    chunk_index: usize,
    content: &str,
    embedding: &[f32],
    now: &str,
) -> Value {
    let id = match rec.id {
        Some(u) => u.to_string(),
        None => rec.object_path.clone(),
    };
    json!({
        "artifact_id": id,
        "object_path": rec.object_path,
        "source": rec.source,
        "artifact_class": rec.artifact_class,
        "table_ref": rec.table_ref,
        "chunk_index": chunk_index as i64,
        "content": content,
        "embedding": embedding,
        "created_at": now,
    })
}

/// Extracts, chunks, embeds, and appends artifact content; provisions the
/// `artifact_chunks` table on demand.
pub struct ArtifactContentIndexer {
    catalog: Arc<dyn Catalog>,
    write: WritePath,
    store: Arc<dyn ObjectStore>,
    embed: Arc<dyn EmbeddingBackend>,
}

impl ArtifactContentIndexer {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        format: Arc<dyn SegmentFormat>,
        store: Arc<dyn ObjectStore>,
        embed: Arc<dyn EmbeddingBackend>,
    ) -> Self {
        let write = WritePath::new(catalog.clone(), format);
        Self { catalog, write, store, embed }
    }

    /// Ensure the `artifacts` database and `artifact_chunks` table exist.
    /// Idempotent; safe to race.
    pub async fn ensure_provisioned(&self) -> anyhow::Result<()> {
        if self.catalog.lookup_table(ARTIFACTS_DB, CHUNKS_TABLE).await.is_ok() {
            return Ok(());
        }
        let db_id = match self.catalog.lookup_database(ARTIFACTS_DB).await? {
            Some(id) => id,
            None => self.catalog.create_database(ARTIFACTS_DB).await?,
        };
        let _ = self
            .catalog
            .create_table(
                db_id,
                CHUNKS_TABLE,
                chunks_schema(self.embed.dimension() as i32),
                TableConfig::default(),
            )
            .await;
        Ok(())
    }

    /// Index content for every live, text-classed, non-producer-attached
    /// record not already in the ingest ledger. Returns the number of chunks
    /// written. Per-artifact failures (missing blob, embed error) are skipped
    /// so one bad artifact never poisons the sweep.
    pub async fn index(&self, records: &[ArtifactRecord]) -> anyhow::Result<usize> {
        self.ensure_provisioned().await?;
        let mut total = 0usize;
        for rec in records {
            if rec.deleted_at.is_some()
                || PRODUCER_ATTACHED_SOURCES.contains(&rec.source.as_str())
                || !is_text_class(&rec.artifact_class)
                || rec.size_bytes < 0
                || rec.size_bytes as usize > MAX_BLOB_BYTES
            {
                continue;
            }
            let key = idempotency_key(rec);
            // Ledger check BEFORE the fetch: an already-indexed artifact costs
            // one catalog lookup, not a blob download + embed.
            if self.catalog.lookup_idempotency(&key).await?.is_some() {
                continue;
            }
            match self.index_one(rec, &key).await {
                Ok(n) => total += n,
                Err(e) => tracing::warn!(
                    error = %e,
                    object_path = %rec.object_path,
                    "artifact content indexing failed; skipping"
                ),
            }
        }
        Ok(total)
    }

    async fn index_one(&self, rec: &ArtifactRecord, key: &str) -> anyhow::Result<usize> {
        let path = ObjPath::from(rec.object_path.as_str());
        let Some(bytes) = kyma_storage::get_artifact(&self.store, &path)
            .await
            .map_err(|e| anyhow::anyhow!("artifact fetch: {e}"))?
        else {
            // Tracking row without a blob (row-before-blob write order): the
            // next sweep retries after the data source re-writes it.
            return Ok(0);
        };
        if bytes.len() > MAX_BLOB_BYTES || !looks_textual(&bytes) {
            return Ok(0);
        }
        let text = String::from_utf8_lossy(&bytes);
        let chunks = chunk_text(&text);
        if chunks.is_empty() {
            return Ok(0);
        }
        let vectors = self
            .embed
            .embed(&chunks)
            .await
            .map_err(|e| anyhow::anyhow!("embed: {e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let rows: Vec<Value> = chunks
            .iter()
            .zip(vectors.iter())
            .enumerate()
            .map(|(i, (c, v))| chunk_row(rec, i, c, v, &now))
            .collect();
        let n = rows.len();

        let tref = self.catalog.lookup_table(ARTIFACTS_DB, CHUNKS_TABLE).await?;
        let mut buf = Vec::with_capacity(rows.len() * 256);
        for r in &rows {
            serde_json::to_writer(&mut buf, r)?;
            buf.push(b'\n');
        }
        let batches = kyma_ingest_core::parse_ndjson(&buf, tref.schema.clone())?;
        self.write
            .ingest_with_idempotency(ARTIFACTS_DB, &tref, batches, Some(key))
            .await?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_classes_match() {
        assert!(is_text_class("log"));
        assert!(is_text_class("file"));
        assert!(!is_text_class("image"));
        assert!(!is_text_class("parquet"));
    }

    #[test]
    fn binary_sniff_rejects_nul_and_accepts_utf8() {
        assert!(looks_textual(b"plain ascii log line\n"));
        assert!(looks_textual("unicode — naïve ✓".as_bytes()));
        assert!(!looks_textual(b"\x00\x01\x02binary"));
        assert!(!looks_textual(&[0xff, 0xfe, 0x41, 0x00, 0x42, 0x00])); // UTF-16LE BOM
    }

    #[test]
    fn sniff_allows_codepoint_cut_at_boundary() {
        // 8 KiB of 'a' then a multi-byte char straddling the sniff edge.
        let mut v = vec![b'a'; SNIFF_BYTES - 1];
        v.extend_from_slice("é".as_bytes()); // 2 bytes; first lands inside the sniff window
        assert!(looks_textual(&v));
    }

    #[test]
    fn chunking_is_line_aligned_and_capped() {
        let text = (0..200).map(|i| format!("line {i}: {}", "x".repeat(40))).collect::<Vec<_>>().join("\n");
        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1, "long text must split");
        for c in &chunks {
            assert!(c.chars().count() <= CHUNK_CHARS + 50, "chunk near target size");
            assert!(c.starts_with("line "), "chunks split on line boundaries");
        }
        // Reassembling loses nothing.
        let joined = chunks.join("\n");
        assert_eq!(joined, text);

        let huge = "z\n".repeat(CHUNK_CHARS * MAX_CHUNKS_PER_ARTIFACT);
        assert!(chunk_text(&huge).len() <= MAX_CHUNKS_PER_ARTIFACT);
    }

    #[test]
    fn empty_and_whitespace_text_yields_no_chunks() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n  \n").is_empty());
    }

    #[test]
    fn chunk_row_matches_schema_columns() {
        let rec = ArtifactRecord {
            id: Some(uuid::Uuid::nil()),
            tenant_id: kyma_core::tenant::DEFAULT_TENANT,
            object_path: "artifacts/x/y.log".into(),
            source: "fswatch".into(),
            artifact_class: "log".into(),
            table_ref: Some("prod.ci_logs".into()),
            data_source_id: None,
            size_bytes: 10,
            sha256: Some("abc".into()),
            created_at: None,
            expires_at: None,
            deleted_at: None,
        };
        let row = chunk_row(&rec, 3, "hello", &[0.1, 0.2], "2026-06-10T00:00:00Z");
        let schema = chunks_schema(2);
        for f in schema.fields() {
            assert!(row.get(f.name()).is_some(), "row missing column {}", f.name());
        }
        assert_eq!(row["chunk_index"], 3);
        assert_eq!(row["content"], "hello");
        assert_eq!(row["artifact_id"], uuid::Uuid::nil().to_string());
    }

    #[test]
    fn idempotency_key_is_stable_and_content_addressed() {
        let mut rec = ArtifactRecord {
            id: Some(uuid::Uuid::nil()),
            tenant_id: kyma_core::tenant::DEFAULT_TENANT,
            object_path: "a/b.log".into(),
            source: "fswatch".into(),
            artifact_class: "log".into(),
            table_ref: None,
            data_source_id: None,
            size_bytes: 10,
            sha256: Some("h1".into()),
            created_at: None,
            expires_at: None,
            deleted_at: None,
        };
        let k1 = idempotency_key(&rec);
        assert_eq!(k1, idempotency_key(&rec), "stable");
        rec.sha256 = Some("h2".into());
        assert_ne!(k1, idempotency_key(&rec), "new content hash ⇒ new key");
    }
}
