//! Catch-all `artifacts` property-graph: mirrors the Postgres `artifacts`
//! catalog into graph nodes for artifacts that have no producer-graph node
//! (object-store blobs, agent files, fs-watch snapshots). github CI-log
//! artifacts are already nodes in the `github` graph and are skipped here.
//!
//! Modeled on `kyma_memory::MemoryWriter` (provision-on-demand + `WritePath`
//! append) but without embeddings — artifact nodes are plain Utf8 rows.

pub mod content;

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::catalog::{Catalog, GraphSpec, TableConfig};
use kyma_core::segment_format::SegmentFormat;
use kyma_ingest_core::WritePath;
use serde_json::{json, Map, Value};

pub const ARTIFACTS_DB: &str = "artifacts";
pub const GRAPH_NAME: &str = "artifacts";
pub const NODE_TABLE: &str = "artifact_nodes";
pub const EDGE_TABLE: &str = "artifact_edges";

/// Sources whose artifacts already appear as nodes in their own producer graph
/// (so the catch-all skips them).
pub const PRODUCER_ATTACHED_SOURCES: &[&str] = &["github"];

fn nodes_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("labels", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("props", DataType::Utf8, true),
    ]))
}

fn edges_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("src", DataType::Utf8, true),
        Field::new("dst", DataType::Utf8, true),
        Field::new("type", DataType::Utf8, true),
        Field::new("props", DataType::Utf8, true),
    ]))
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Shape one catalog artifact into an `Artifact`-labeled graph node row whose
/// columns match `nodes_schema()`. Pure — unit-testable without a catalog.
pub fn artifact_node_row(rec: &ArtifactRecord) -> Value {
    let id = match rec.id {
        Some(u) => format!("artifact::{u}"),
        None => format!("artifact::{}", rec.object_path),
    };
    let mut props = Map::new();
    props.insert("object_path".into(), json!(rec.object_path));
    if let Some(s) = &rec.sha256 {
        props.insert("sha256".into(), json!(s));
    }
    props.insert("size_bytes".into(), json!(rec.size_bytes));
    props.insert("artifact_class".into(), json!(rec.artifact_class));
    props.insert("source".into(), json!(rec.source));
    props.insert("retrievable".into(), json!(rec.deleted_at.is_none()));
    if let Some(u) = rec.id {
        props.insert("artifact_id".into(), json!(u.to_string()));
    }
    if let Some(ts) = rec.created_at {
        props.insert("created_at".into(), json!(ts.to_rfc3339()));
    }
    json!({
        "id": id,
        "labels": "Artifact",
        "name": basename(&rec.object_path),
        "props": serde_json::to_string(&props).unwrap_or_default(),
    })
}

/// Writes catch-all artifact nodes and provisions the `artifacts` graph on demand.
#[derive(Clone)]
pub struct ArtifactGraphWriter {
    catalog: Arc<dyn Catalog>,
    write: WritePath,
}

impl ArtifactGraphWriter {
    pub fn new(catalog: Arc<dyn Catalog>, format: Arc<dyn SegmentFormat>) -> Self {
        let write = WritePath::new(catalog.clone(), format);
        Self { catalog, write }
    }

    /// Ensure the `artifacts` database, node/edge tables, and graph registration
    /// exist. Idempotent. Mirrors `MemoryWriter::ensure_provisioned`.
    pub async fn ensure_provisioned(&self) -> anyhow::Result<()> {
        if self.catalog.lookup_table(ARTIFACTS_DB, NODE_TABLE).await.is_ok() {
            return Ok(());
        }
        let db_id = match self.catalog.lookup_database(ARTIFACTS_DB).await? {
            Some(id) => id,
            None => self.catalog.create_database(ARTIFACTS_DB).await?,
        };
        let _ = self
            .catalog
            .create_table(db_id, NODE_TABLE, nodes_schema(), TableConfig::default())
            .await;
        let _ = self
            .catalog
            .create_table(db_id, EDGE_TABLE, edges_schema(), TableConfig::default())
            .await;
        if let Err(e) = self
            .catalog
            .create_graph(ARTIFACTS_DB, GRAPH_NAME, GraphSpec::with_defaults(NODE_TABLE, EDGE_TABLE))
            .await
        {
            let msg = e.to_string();
            if !(msg.contains("exists") || msg.contains("duplicate")) {
                return Err(anyhow::anyhow!("create artifacts graph: {msg}"));
            }
        }
        Ok(())
    }

    /// Materialize node rows for every record whose `source` is not
    /// producer-attached. Returns the number of nodes written. Idempotent.
    pub async fn sync(&self, records: &[ArtifactRecord]) -> anyhow::Result<usize> {
        self.ensure_provisioned().await?;
        let rows: Vec<Value> = records
            .iter()
            .filter(|r| r.deleted_at.is_none())
            .filter(|r| !PRODUCER_ATTACHED_SOURCES.contains(&r.source.as_str()))
            .map(artifact_node_row)
            .collect();
        let n = rows.len();
        self.append_rows(NODE_TABLE, rows).await?;
        Ok(n)
    }

    async fn append_rows(&self, table: &str, json_rows: Vec<Value>) -> anyhow::Result<()> {
        if json_rows.is_empty() {
            return Ok(());
        }
        let tref = self.catalog.lookup_table(ARTIFACTS_DB, table).await?;
        let mut buf = Vec::with_capacity(json_rows.len() * 128);
        for r in &json_rows {
            serde_json::to_writer(&mut buf, r)?;
            buf.push(b'\n');
        }
        let key = format!("artifacts:{:x}", stable_hash(&buf));
        let batches = kyma_ingest_core::parse_ndjson(&buf, tref.schema.clone())?;
        self.write
            .ingest_with_idempotency(ARTIFACTS_DB, &tref, batches, Some(&key))
            .await?;
        Ok(())
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}
