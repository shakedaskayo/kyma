//! Writes memory nodes/edges to Kyma columnar storage and registers the
//! `memory` graph. Append-only: mutations (status/importance/merge) write a new
//! version row; recall dedups to the latest by `updated_at`.

use std::sync::Arc;

use kyma_core::catalog::{Catalog, GraphSpec, TableConfig, TableRef};
use kyma_core::segment_format::SegmentFormat;
use kyma_core::types::DatabaseId;
use kyma_embed::EmbeddingBackend;
use kyma_ingest_core::WritePath;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{MemoryError, Result};
use crate::types::CreateMemory;
use crate::{rows, schema, EDGE_TABLE, GRAPH_NAME, NODE_TABLE};

/// Writes memory data and provisions the memory database/tables/graph on demand.
#[derive(Clone)]
pub struct MemoryWriter {
    catalog: Arc<dyn Catalog>,
    write: WritePath,
    embed: Arc<dyn EmbeddingBackend>,
    database: String,
}

impl MemoryWriter {
    /// Build a writer over the catalog + format engine. Backend-agnostic — works
    /// over the Postgres catalog (server) or the embedded SQLite catalog
    /// (`kyma local`); no direct DB pool is required.
    pub fn new(
        catalog: Arc<dyn Catalog>,
        format: Arc<dyn SegmentFormat>,
        embed: Arc<dyn EmbeddingBackend>,
    ) -> Self {
        let write = WritePath::new(catalog.clone(), format);
        Self {
            catalog,
            write,
            embed,
            database: crate::DEFAULT_DATABASE.to_string(),
        }
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    /// Ensure the memory database, both tables, and the `memory` graph
    /// registration exist. Fast-paths when the node table is already present,
    /// but first backfills any bi-temporal columns a pre-existing store is
    /// missing (non-destructive: historical extents null-fill on read).
    pub async fn ensure_provisioned(&self) -> Result<()> {
        if let Ok(tref) = self.catalog.lookup_table(&self.database, NODE_TABLE).await {
            self.ensure_bitemporal_columns(&tref).await?;
            return Ok(());
        }
        let db_id = self.ensure_database().await?;
        let dim = self.embed.dimension() as i32;
        // Races (two concurrent first-writes) surface as "already exists" — ignore.
        let _ = self
            .catalog
            .create_table(
                db_id,
                NODE_TABLE,
                schema::memory_nodes_schema(dim),
                TableConfig::default(),
            )
            .await;
        let _ = self
            .catalog
            .create_table(
                db_id,
                EDGE_TABLE,
                schema::memory_edges_schema(),
                TableConfig::default(),
            )
            .await;

        let spec = GraphSpec {
            node_table: NODE_TABLE.into(),
            edge_table: EDGE_TABLE.into(),
            id_col: "id".into(),
            label_col: "labels".into(),
            src_col: "src".into(),
            dst_col: "dst".into(),
            type_col: "type".into(),
            realm_col: Some("realm".into()),
        };
        if let Err(e) = self
            .catalog
            .create_graph(&self.database, GRAPH_NAME, spec)
            .await
        {
            let msg = e.to_string();
            if !(msg.contains("exists") || msg.contains("duplicate")) {
                return Err(MemoryError::Catalog(msg));
            }
        }
        Ok(())
    }

    /// Add any bi-temporal columns missing from an already-provisioned
    /// `memory_nodes` table. Stores created before bi-temporal support keep
    /// their extents; the new nullable columns read as NULL ("always valid"),
    /// which the recall validity guard treats correctly. Idempotent.
    async fn ensure_bitemporal_columns(&self, tref: &TableRef) -> Result<()> {
        for col in schema::BITEMPORAL_COLUMNS {
            if tref.schema.field_with_name(col).is_ok() {
                continue;
            }
            if let Err(e) = self
                .catalog
                .alter_table_add_column(tref.id, col, "string")
                .await
            {
                let msg = e.to_string();
                // A concurrent writer may have added it first.
                if !(msg.contains("exists") || msg.contains("duplicate")) {
                    return Err(MemoryError::Catalog(msg));
                }
            }
        }
        Ok(())
    }

    async fn ensure_database(&self) -> Result<DatabaseId> {
        // Resolve via the catalog trait (backend-agnostic), then create if
        // missing. Races (two concurrent first-writes) surface as a unique
        // violation on create — callers treat "exists" as benign.
        if let Some(id) = self
            .catalog
            .lookup_database(&self.database)
            .await
            .map_err(|e| MemoryError::Catalog(e.to_string()))?
        {
            return Ok(id);
        }
        self.catalog
            .create_database(&self.database)
            .await
            .map_err(|e| MemoryError::Catalog(e.to_string()))
    }

    /// Embed + persist a new memory node plus a `REFERENCES` edge per reference.
    /// Returns the new memory uuid.
    pub async fn save(&self, m: &CreateMemory) -> Result<Uuid> {
        self.ensure_provisioned().await?;
        // Strip `<private>…</private>` spans at the store layer (defense in
        // depth beyond the capture hooks) so secrets are never persisted/embedded.
        let red = redact_create(m);
        let m = &red;
        let emb = self.embed_one(&m.content).await?;
        let id = Uuid::new_v4();
        let now = now_rfc3339();
        let node = rows::node_row(&id, m, &emb, &now);
        self.append_rows(NODE_TABLE, vec![node]).await?;

        if !m.references.is_empty() {
            let src = rows::node_id(&id);
            let edges: Vec<Value> = m
                .references
                .iter()
                .map(|r| rows::edge_row(&src, r, "REFERENCES", &m.realm, None, None, &now))
                .collect();
            self.append_rows(EDGE_TABLE, edges).await?;
        }
        Ok(id)
    }

    /// Upsert a new version of an existing memory `id` (same id → latest-wins),
    /// re-embedding the redacted content. Used by topic-key upsert so a repeated
    /// save updates in place instead of creating a duplicate.
    pub async fn save_as(&self, id: Uuid, m: &CreateMemory) -> Result<()> {
        self.ensure_provisioned().await?;
        let red = redact_create(m);
        let m = &red;
        let emb = self.embed_one(&m.content).await?;
        let now = now_rfc3339();
        let node = rows::node_row(&id, m, &emb, &now);
        self.append_rows(NODE_TABLE, vec![node]).await
    }

    /// Append a single edge linking a memory node to another (possibly
    /// cross-graph) entity.
    pub async fn link(
        &self,
        src_node_id: &str,
        dst_node_id: &str,
        rel_type: &str,
        realm: &str,
        target_namespace: Option<&str>,
    ) -> Result<()> {
        self.ensure_provisioned().await?;
        let now = now_rfc3339();
        let edge = rows::edge_row(
            src_node_id,
            dst_node_id,
            rel_type,
            realm,
            target_namespace,
            None,
            &now,
        );
        self.append_rows(EDGE_TABLE, vec![edge]).await
    }

    /// Append pre-built node rows (used by read-then-append mutations).
    pub async fn append_node_rows(&self, node_rows: Vec<Value>) -> Result<()> {
        self.ensure_provisioned().await?;
        self.append_rows(NODE_TABLE, node_rows).await
    }

    /// Append pre-built edge rows.
    pub async fn append_edge_rows(&self, edge_rows: Vec<Value>) -> Result<()> {
        self.ensure_provisioned().await?;
        self.append_rows(EDGE_TABLE, edge_rows).await
    }

    /// [`Self::append_node_rows`] with an ingest idempotency key, so a crash
    /// replay of the same batch is deduped instead of double-applied. Skips
    /// the per-call provisioning — batch callers provision once per flush.
    pub async fn append_node_rows_keyed(
        &self,
        node_rows: Vec<Value>,
        key: Option<&str>,
    ) -> Result<()> {
        self.append_rows_keyed(NODE_TABLE, node_rows, key).await
    }

    /// [`Self::append_edge_rows`] with an ingest idempotency key (see
    /// [`Self::append_node_rows_keyed`]).
    pub async fn append_edge_rows_keyed(
        &self,
        edge_rows: Vec<Value>,
        key: Option<&str>,
    ) -> Result<()> {
        self.append_rows_keyed(EDGE_TABLE, edge_rows, key).await
    }

    /// Embed a single text, returning its vector.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let out = self.embed_batch(&[text.to_string()]).await?;
        out.into_iter()
            .next()
            .ok_or_else(|| MemoryError::Embed("backend returned no vector".into()))
    }

    /// Embed many texts in one backend call. Order of outputs matches inputs —
    /// this is the batching point that turns N queued saves into a single
    /// embedding round-trip.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let out = self
            .embed
            .embed(texts)
            .await
            .map_err(|e| MemoryError::Embed(e.to_string()))?;
        if out.len() != texts.len() {
            return Err(MemoryError::Embed(format!(
                "backend returned {} vectors for {} texts",
                out.len(),
                texts.len()
            )));
        }
        Ok(out)
    }

    async fn append_rows(&self, table: &str, json_rows: Vec<Value>) -> Result<()> {
        self.append_rows_keyed(table, json_rows, None).await
    }

    async fn append_rows_keyed(
        &self,
        table: &str,
        json_rows: Vec<Value>,
        key: Option<&str>,
    ) -> Result<()> {
        if json_rows.is_empty() {
            return Ok(());
        }
        let tref = self
            .catalog
            .lookup_table(&self.database, table)
            .await
            .map_err(|e| MemoryError::Catalog(e.to_string()))?;
        let mut buf = Vec::with_capacity(json_rows.len() * 128);
        for r in &json_rows {
            serde_json::to_writer(&mut buf, r).map_err(|e| MemoryError::Ingest(e.to_string()))?;
            buf.push(b'\n');
        }
        let batches = kyma_ingest_core::parse_ndjson(&buf, tref.schema.clone())
            .map_err(|e| MemoryError::Ingest(e.to_string()))?;
        self.write
            .ingest_with_idempotency(&self.database, &tref, batches, key)
            .await
            .map_err(|e| MemoryError::Write(e.to_string()))?;
        Ok(())
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Return a copy of `m` with `<private>…</private>` spans redacted from its
/// content and title. Cheap clone on the (low-volume) save path.
///
/// Public so the async enqueue path can redact *before* a memory is persisted
/// to the durable job store — secrets must never reach any store, including
/// the queue.
pub fn redact_create(m: &CreateMemory) -> CreateMemory {
    let mut out = m.clone();
    out.content = redact_private(&m.content);
    out.title = m.title.as_deref().map(redact_private);
    out
}

/// Replace every `<private>…</private>` span (case-insensitive tags) with
/// `[redacted]`. An unclosed `<private>` redacts to end-of-string. ASCII tag
/// offsets are char boundaries, so the byte slicing stays UTF-8-safe.
fn redact_private(s: &str) -> String {
    const OPEN: &str = "<private>";
    const CLOSE: &str = "</private>";
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        match lower[i..].find(OPEN) {
            Some(rel) => {
                let start = i + rel;
                out.push_str(&s[i..start]);
                out.push_str("[redacted]");
                let after = start + OPEN.len();
                match lower[after..].find(CLOSE) {
                    Some(crel) => i = after + crel + CLOSE.len(),
                    None => break, // unclosed → drop the rest
                }
            }
            None => {
                out.push_str(&s[i..]);
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod redact_tests {
    use super::redact_private;

    #[test]
    fn strips_private_spans() {
        assert_eq!(
            redact_private("token is <private>sk-abc123</private> ok"),
            "token is [redacted] ok"
        );
    }

    #[test]
    fn unclosed_private_drops_tail() {
        assert_eq!(
            redact_private("safe <PRIVATE>secret tail"),
            "safe [redacted]"
        );
    }

    #[test]
    fn no_tags_unchanged() {
        assert_eq!(redact_private("nothing to redact"), "nothing to redact");
    }
}
