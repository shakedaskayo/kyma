# Artifacts as First-Class Graph Entities — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every catalog artifact (CI logs now; object-store/fswatch/agent files via a catch-all) a first-class graph node, linked to its producer by a single `HAS_ARTIFACT` edge, visible + searchable + openable on the graph page.

**Architecture:** Two materialization paths feeding one node contract. (A) *Producer-attached* — the github connector's existing `LogFile` node is relabeled to `Artifact`, carries its catalog `artifact_id`, and its edge becomes `HAS_ARTIFACT` (works in local + server mode). (B) *Catch-all* — a new server-only `ArtifactGraphWriter` mirrors the `artifacts` Postgres catalog into a registered `artifacts` graph for sources with no producer node. The inline log/file preview already exists in the graph sidebar (`LogFileViewer`); its trigger is widened to the `Artifact` label.

**Tech Stack:** Rust (`kyma-connectors`, `kyma-catalog`, new `kyma-artifact-graph` crate, `kyma-ingest-core` `WritePath`, `arrow-schema`, `sqlx`), React + TypeScript (`@kyma-ai/react`, vitest).

**Spec:** `docs/superpowers/specs/2026-06-09-artifacts-as-graph-entities-design.md`

**Phasing:** Phase 1 is independently shippable — it delivers "CI logs appear as Artifact nodes you can open on the graph page" in every deployment mode. Phase 2 (catch-all, Postgres-only) and Phase 3 (polish) build on it.

---

## Phase 1 — GitHub CI logs become Artifact nodes (independently shippable)

### Task 1: Relabel the CI log node to `Artifact`, thread `artifact_id`, standardize the edge

**Files:**
- Modify: `crates/kyma-connectors/src/github/transform.rs` (`log_file_rows`, ~257-288; add a unit test)
- Modify: `crates/kyma-connectors/src/github/joblogs.rs` (capture the `put_and_register` uuid ~161-196; update the integration test ~346-368)

- [ ] **Step 1: Write the failing unit test for the new node shape**

Add to the bottom of `crates/kyma-connectors/src/github/transform.rs` (create a `#[cfg(test)] mod tests { … }` block if none exists; otherwise add the fn inside it):

```rust
#[cfg(test)]
mod artifact_node_tests {
    use super::*;

    #[test]
    fn log_file_rows_emits_artifact_node_and_has_artifact_edge() {
        let (node, edge) = log_file_rows(
            "acme", "app", 900, "build",
            "artifacts/t/github/acme/app/100/900.log.txt",
            "deadbeef", 1234, false,
            Some("11111111-1111-1111-1111-111111111111"),
        );

        // Node is now labeled `Artifact` (was `LogFile`).
        assert_eq!(node["labels"], "Artifact");
        assert_eq!(node["id"], "log:acme/app#900");

        // Props carry the catalog link + retrieval handle + class + source.
        let props: serde_json::Value =
            serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
        assert_eq!(props["artifact_id"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(props["artifact_class"], "log");
        assert_eq!(props["source"], "github");
        assert_eq!(props["retrievable"], true);
        assert_eq!(props["object_path"], "artifacts/t/github/acme/app/100/900.log.txt");

        // Edge type is the single generic `HAS_ARTIFACT` (was `JOB_HAS_LOG`).
        assert_eq!(edge["type"], "HAS_ARTIFACT");
        assert_eq!(edge["src"], "job:acme/app#900");
        assert_eq!(edge["dst"], "log:acme/app#900");
    }

    #[test]
    fn log_file_rows_without_artifact_id_omits_the_prop() {
        let (node, _edge) = log_file_rows(
            "acme", "app", 900, "build", "p", "s", 1, false, None,
        );
        let props: serde_json::Value =
            serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
        assert!(props.get("artifact_id").is_none());
        assert_eq!(props["retrievable"], true);
    }
}
```

- [ ] **Step 2: Run the test — verify it fails to compile (signature mismatch)**

Run: `cargo test -p kyma-connectors --features github log_file_rows_emits_artifact`
Expected: FAIL — compile error, `log_file_rows` takes 8 args not 9 / does not accept the `Option<&str>`.

- [ ] **Step 3: Update `log_file_rows` to the new shape**

Replace the body of `log_file_rows` in `transform.rs` (~257-288) with:

```rust
/// Artifact node for a CI job log (carries the `object_path` retrieval handle +
/// the catalog `artifact_id`) + the `HAS_ARTIFACT` edge (job→artifact).
#[allow(clippy::too_many_arguments)]
pub fn log_file_rows(
    owner: &str,
    repo: &str,
    job_id: i64,
    job_name: &str,
    object_path: &str,
    sha256: &str,
    size_bytes: i64,
    truncated: bool,
    artifact_id: Option<&str>,
) -> (Value, Value) {
    let lid = log_file_node_id(owner, repo, job_id);
    let mut props = Map::new();
    props.insert("object_path".into(), json!(object_path));
    props.insert("sha256".into(), json!(sha256));
    props.insert("size_bytes".into(), json!(size_bytes));
    props.insert("truncated".into(), json!(truncated));
    props.insert("artifact_class".into(), json!("log"));
    props.insert("source".into(), json!("github"));
    props.insert("retrievable".into(), json!(true));
    if let Some(aid) = artifact_id {
        props.insert("artifact_id".into(), json!(aid));
    }
    let name = if job_name.is_empty() {
        format!("log #{job_id}")
    } else {
        format!("{job_name}.log")
    };
    let node = make_node(&lid, "Artifact", &name, props);
    let edge = make_edge(
        &job_node_id(owner, repo, job_id),
        "HAS_ARTIFACT",
        &lid,
        Map::new(),
    );
    (node, edge)
}
```

- [ ] **Step 4: Thread the catalog uuid through in `joblogs.rs`**

In `crates/kyma-connectors/src/github/joblogs.rs`, change the `put_and_register` block (~161-182) to capture the returned id, then pass it into `log_file_rows` (~184-194):

```rust
            let mut artifact_id: Option<String> = None;
            if let Some(store) = artifacts {
                let record = ArtifactRecord {
                    id: None,
                    tenant_id: tenant,
                    object_path: object_path.clone(),
                    source: "github".into(),
                    artifact_class: "log".into(),
                    table_ref: Some(JOB_LOGS_TABLE.to_string()),
                    connector_id: None,
                    size_bytes,
                    sha256: Some(sha.clone()),
                    created_at: None,
                    expires_at: None,
                    deleted_at: None,
                };
                match store
                    .put_and_register(record, bytes::Bytes::from(stored_bytes))
                    .await
                {
                    Ok(id) => artifact_id = Some(id.to_string()),
                    Err(e) => tracing::warn!(
                        owner, repo, job_id, error = %e,
                        "artifact store failed; row emitted without blob"
                    ),
                }
            }

            // Artifact node + HAS_ARTIFACT edge (carries the object_path handle).
            let (log_node, log_edge) = transform::log_file_rows(
                owner,
                repo,
                job_id,
                &job_name,
                &object_path,
                &sha,
                size_bytes,
                truncated,
                artifact_id.as_deref(),
            );
```

- [ ] **Step 5: Update the existing capture integration test assertions**

In `joblogs.rs` `capture_redacts_secrets_before_storing` (~346-368), update the CI-subgraph assertions to the new label/edge and assert the catalog link:

```rust
        // ── CI graph subgraph ──
        // 1 run + 1 job + 1 artifact = 3 nodes; HAS_RUN + RUN_CONTAINS_JOB +
        // HAS_ARTIFACT = 3 edges (no RUN_ON_BRANCH — the mock run has no branch).
        let label = |n: &Value| n["labels"].as_str().unwrap_or("").to_string();
        let id = |n: &Value| n["id"].as_str().unwrap_or("").to_string();
        assert_eq!(res.nodes.len(), 3, "run+job+artifact nodes");
        assert!(res.nodes.iter().any(|n| label(n) == "WorkflowRun" && id(n) == "run:acme/app#100"));
        assert!(res.nodes.iter().any(|n| label(n) == "Job" && id(n) == "job:acme/app#900"));
        let log_node = res
            .nodes
            .iter()
            .find(|n| label(n) == "Artifact" && id(n) == "log:acme/app#900")
            .expect("Artifact node");
        let lprops: Value =
            serde_json::from_str(log_node["props"].as_str().unwrap()).unwrap();
        assert!(
            lprops["object_path"].as_str().unwrap().contains(&job_log_key(tenant, "acme", "app", 100, 900)),
            "Artifact node carries the object_path retrieval handle"
        );
        assert!(lprops.get("artifact_id").is_some(), "Artifact node carries the catalog id");

        let etype = |e: &Value| e["type"].as_str().unwrap_or("").to_string();
        let etypes: Vec<String> = res.edges.iter().map(etype).collect();
        assert!(etypes.contains(&"HAS_RUN".to_string()));
        assert!(etypes.contains(&"RUN_CONTAINS_JOB".to_string()));
        assert!(etypes.contains(&"HAS_ARTIFACT".to_string()));
```

- [ ] **Step 6: Run the connector tests — verify they pass**

Run: `cargo test -p kyma-connectors --features github`
Expected: PASS — `log_file_rows_*` unit tests and `capture_redacts_secrets_before_storing` all green.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-connectors/src/github/transform.rs crates/kyma-connectors/src/github/joblogs.rs
git commit -m "feat(connectors): CI logs become Artifact nodes (HAS_ARTIFACT + artifact_id)"
```

---

### Task 2: Widen the inline viewer trigger to the `Artifact` label

**Files:**
- Modify: `packages/react/src/graph/GraphSidebar.tsx` (extract + widen the trigger ~531-534)
- Test: `packages/react/src/graph/GraphSidebar.test.tsx` (create)

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/graph/GraphSidebar.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { artifactViewerPath } from "./GraphSidebar";

const node = (labels: string[], properties: Record<string, unknown>) =>
  ({ id: "n", labels, properties }) as Parameters<typeof artifactViewerPath>[0];

describe("artifactViewerPath", () => {
  it("returns the object_path for an Artifact-labeled node", () => {
    expect(
      artifactViewerPath(node(["Artifact"], { object_path: "a/b.log" })),
    ).toBe("a/b.log");
  });

  it("still fires for legacy LogFile-labeled nodes", () => {
    expect(
      artifactViewerPath(node(["LogFile"], { object_path: "a/b.log" })),
    ).toBe("a/b.log");
  });

  it("reads object_path nested in a JSON props blob", () => {
    expect(
      artifactViewerPath(node(["Artifact"], { props: '{"object_path":"x/y.log"}' })),
    ).toBe("x/y.log");
  });

  it("returns null for non-artifact nodes", () => {
    expect(artifactViewerPath(node(["Job"], { object_path: "a/b.log" }))).toBeNull();
  });

  it("returns null when there is no object_path", () => {
    expect(artifactViewerPath(node(["Artifact"], {}))).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test — verify it fails**

Run: `pnpm --filter @kyma-ai/react exec vitest run src/graph/GraphSidebar.test.tsx`
Expected: FAIL — `artifactViewerPath` is not exported.

- [ ] **Step 3: Extract + widen the trigger**

In `packages/react/src/graph/GraphSidebar.tsx`, add an exported helper just below `objectPathOf` (~449), importing `GraphNode` (already imported at the top from `@kyma-ai/client`):

```tsx
/** object_path for a node that should show the inline artifact viewer — any
 *  node labeled `Artifact` (or the legacy `LogFile`) that carries an
 *  `object_path`. Returns null otherwise. */
export function artifactViewerPath(
  node: Pick<GraphNode, "labels" | "properties">,
): string | null {
  const isArtifact = node.labels.some((l) => {
    const ll = l.toLowerCase();
    return ll === "artifact" || ll === "logfile";
  });
  return isArtifact ? objectPathOf(node.properties) : null;
}
```

Then in `NodeInspector`, replace the `logPath` memo (~531-534) with a call to the helper:

```tsx
  // Artifact nodes (CI job logs, object-store blobs, fs-watch files) carry an
  // `object_path` — surface the inline viewer that streams the blob.
  const logPath = useMemo(() => artifactViewerPath(node), [node]);
```

- [ ] **Step 4: Run the test — verify it passes**

Run: `pnpm --filter @kyma-ai/react exec vitest run src/graph/GraphSidebar.test.tsx`
Expected: PASS — all five cases green.

- [ ] **Step 5: Run the package test + typecheck**

Run: `pnpm --filter @kyma-ai/react test`
Expected: PASS — no regressions in the graph suite.

- [ ] **Step 6: Commit**

```bash
git add packages/react/src/graph/GraphSidebar.tsx packages/react/src/graph/GraphSidebar.test.tsx
git commit -m "feat(react): inline artifact viewer fires for Artifact-labeled nodes"
```

**At this point Phase 1 is complete and shippable: new CI captures render as `Artifact` nodes, filterable via the `Artifact` label, openable inline on the graph page.**

---

## Phase 2 — Catch-all `artifacts` graph (server / Postgres only)

> The `artifacts` catalog table exists only on `PostgresCatalog` (artifacts.rs header). So this phase materializes catalog artifacts that have **no** producer node (object-store blobs, agent-contributed files, fs-watch snapshots) into a dedicated `artifacts` graph. In `kyma local` (sqlite) there is no artifacts catalog, so nothing to sync — Phase 1 still works there.

### Task 3: New crate `kyma-artifact-graph` — the node contract + writer (provision + write)

**Files:**
- Create: `crates/kyma-artifact-graph/Cargo.toml`
- Create: `crates/kyma-artifact-graph/src/lib.rs`
- Create: `crates/kyma-artifact-graph/tests/writer_it.rs`
- Modify: `Cargo.toml` (workspace `members`)

- [ ] **Step 1: Scaffold the crate manifest**

Create `crates/kyma-artifact-graph/Cargo.toml` (mirror `crates/kyma-memory/Cargo.toml` deps; copy exact versions from there):

```toml
[package]
name = "kyma-artifact-graph"
version = "0.1.0"
edition = "2021"

[dependencies]
kyma-core = { path = "../kyma-core" }
kyma-catalog = { path = "../kyma-catalog" }
kyma-ingest-core = { path = "../kyma-ingest-core" }
arrow-schema = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
kyma-catalog-sqlite = { path = "../kyma-catalog-sqlite" }
kyma-format-tlm = { path = "../kyma-format-tlm" }
kyma-storage = { path = "../kyma-storage" }
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

(If `arrow-schema` / a dep is not declared `workspace = true` in the root `Cargo.toml`, copy the concrete version string used in `crates/kyma-memory/Cargo.toml`.)

- [ ] **Step 2: Register the crate in the workspace**

In the root `/Users/shakedaskayo/shaked/projects/kyma/Cargo.toml`, add `"crates/kyma-artifact-graph"` to `[workspace] members` (keep the list sorted as the file is).

- [ ] **Step 3: Write the failing integration test**

Create `crates/kyma-artifact-graph/tests/writer_it.rs` (harness copied from `crates/kyma-memory/tests/file_candidates_it.rs:38-52`):

```rust
use std::sync::Arc;

use chrono::Utc;
use kyma_artifact_graph::{artifact_node_row, ArtifactGraphWriter, ARTIFACTS_DB, GRAPH_NAME};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::catalog::{Catalog, GraphSpec};
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;
use kyma_format_tlm::TelemetryFormat;
use kyma_storage::{build_object_store, StorageConfig};
use uuid::Uuid;

async fn writer() -> (Arc<dyn Catalog>, ArtifactGraphWriter) {
    let catalog: Arc<dyn Catalog> = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory().await.unwrap(),
    );
    let tmp = std::env::temp_dir().join(format!("kyma-ag-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .unwrap();
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "test"));
    (catalog.clone(), ArtifactGraphWriter::new(catalog, format))
}

fn rec(source: &str, class: &str, path: &str) -> ArtifactRecord {
    ArtifactRecord {
        id: Some(Uuid::new_v4()),
        tenant_id: TenantId::from_uuid(Uuid::nil()),
        object_path: path.into(),
        source: source.into(),
        artifact_class: class.into(),
        table_ref: None,
        connector_id: None,
        size_bytes: 42,
        sha256: Some("abc".into()),
        created_at: Some(Utc::now()),
        expires_at: None,
        deleted_at: None,
    }
}

#[test]
fn node_row_shape_is_artifact_labeled() {
    let r = rec("fswatch", "file", "snapshots/x.txt");
    let node = artifact_node_row(&r);
    assert_eq!(node["labels"], "Artifact");
    assert_eq!(node["id"], format!("artifact::{}", r.id.unwrap()));
    let props: serde_json::Value =
        serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
    assert_eq!(props["artifact_class"], "file");
    assert_eq!(props["source"], "fswatch");
    assert_eq!(props["object_path"], "snapshots/x.txt");
    assert_eq!(props["retrievable"], true);
}

#[tokio::test]
async fn provision_then_sync_registers_graph_and_writes_nodes() {
    let (catalog, w) = writer().await;

    // github artifacts are producer-attached → skipped by the catch-all.
    let records = vec![
        rec("fswatch", "file", "snapshots/x.txt"),
        rec("agent", "file", "uploads/y.bin"),
        rec("github", "log", "artifacts/t/github/a/b/1/2.log.txt"),
    ];
    let written = w.sync(&records).await.unwrap();
    assert_eq!(written, 2, "github record is skipped");

    // Graph is registered and discoverable.
    let g = catalog.get_graph(ARTIFACTS_DB, GRAPH_NAME).await.unwrap();
    assert!(g.is_some(), "artifacts graph registered");

    // Re-syncing the same records is idempotent (no error, dedup downstream).
    let again = w.sync(&records).await.unwrap();
    assert_eq!(again, 2);

    // Provisioning is idempotent.
    w.ensure_provisioned().await.unwrap();
    let _ = GraphSpec::with_defaults("artifact_nodes", "artifact_edges");
}
```

- [ ] **Step 4: Run the test — verify it fails**

Run: `cargo test -p kyma-artifact-graph`
Expected: FAIL — crate has no `lib.rs` symbols.

- [ ] **Step 5: Implement `lib.rs`**

Create `crates/kyma-artifact-graph/src/lib.rs`:

```rust
//! Catch-all `artifacts` property-graph: mirrors the Postgres `artifacts`
//! catalog into graph nodes for artifacts that have no producer-graph node
//! (object-store blobs, agent files, fs-watch snapshots). github CI-log
//! artifacts are already nodes in the `github` graph and are skipped here.
//!
//! Modeled on `kyma_memory::MemoryWriter` (provision-on-demand + WritePath
//! append) but without embeddings — artifact nodes are plain Utf8 rows.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::catalog::{Catalog, GraphSpec, TableConfig};
use kyma_core::segment_format::SegmentFormat;
use kyma_ingest_core::WritePath;
use serde_json::{json, Map, Value};

/// Database + graph that hold the catch-all artifact nodes.
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
    /// exist. Idempotent; concurrent first-writes surface as "exists" and are
    /// ignored. (Mirrors `MemoryWriter::ensure_provisioned`.)
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
    /// producer-attached. Returns the number of nodes written. Idempotent:
    /// re-syncing identical rows is keyed on the node id so it is a no-op.
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
        // Idempotency key over the row payload: identical re-syncs are no-ops.
        let key = format!("artifacts:{:x}", seahash_like(&buf));
        let batches = kyma_ingest_core::parse_ndjson(&buf, tref.schema.clone())?;
        self.write
            .ingest_with_idempotency(ARTIFACTS_DB, &tref, batches, Some(&key))
            .await?;
        Ok(())
    }
}

/// Tiny stable hash for the idempotency key (no external dep).
fn seahash_like(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}
```

> If `WritePath::ingest_with_idempotency` has a different signature than the memory writer uses (`crates/kyma-memory/src/writer.rs:309-312`), match that call exactly — copy the argument order from there.

- [ ] **Step 6: Run the test — verify it passes**

Run: `cargo test -p kyma-artifact-graph`
Expected: PASS — node-shape and provision/sync tests green.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-artifact-graph Cargo.toml
git commit -m "feat(artifact-graph): catch-all Artifact node writer + graph provisioner"
```

---

### Task 4: `list_live_artifacts` on `PostgresCatalog`

**Files:**
- Modify: `crates/kyma-catalog/src/artifacts.rs` (add the method ~123)

- [ ] **Step 1: Add the method**

In `crates/kyma-catalog/src/artifacts.rs`, inside `impl PostgresCatalog`, add after `get_artifact_in_tenant`:

```rust
    /// All live (not soft-deleted) artifacts for `tenant`, newest first. Used by
    /// the catch-all graph sync to materialize artifact nodes.
    pub async fn list_live_artifacts(&self, tenant: TenantId) -> Result<Vec<ArtifactRecord>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, object_path, source, artifact_class, table_ref,
                    connector_id, size_bytes, sha256, created_at, expires_at, deleted_at
             FROM artifacts
             WHERE tenant_id = $1 AND deleted_at IS NULL
             ORDER BY created_at DESC",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        rows.iter().map(row_to_artifact).collect()
    }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p kyma-catalog`
Expected: PASS — compiles clean.

- [ ] **Step 3: Add a Postgres-gated integration test**

Locate the crate's existing Postgres test harness first:

Run: `grep -rln "register_artifact\|PgPool\|#\\[ignore\\]\|KYMA_TEST_PG\|connect(" crates/kyma-catalog/tests crates/kyma-catalog/src/artifacts.rs`

Then add a test that follows that harness (e.g. a `#[tokio::test]` gated on the same env/`#[ignore]` convention the crate already uses for Postgres). The test:

```rust
// after registering two artifacts (one with deleted_at set via soft_delete),
// list_live_artifacts returns only the live one, scoped to the tenant.
```

Mirror the exact connection setup the crate's other Postgres tests use (do not invent a new harness). If the crate has **no** Postgres test harness, leave a `// integration: covered by the server e2e in Task 6` comment and rely on Task 6's coverage rather than fabricating a harness.

- [ ] **Step 4: Run the test (or build if PG-gated/ignored)**

Run: `cargo test -p kyma-catalog list_live_artifacts` (add the crate's PG env var if its harness requires one)
Expected: PASS, or correctly SKIPPED under the crate's existing `#[ignore]`/env gate.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-catalog/src/artifacts.rs crates/kyma-catalog/tests
git commit -m "feat(catalog): list_live_artifacts for the artifact-graph sync"
```

---

### Task 5: Sync driver + startup backfill + periodic refresh

**Files:**
- Find + modify: the server artifact retention worker (locate via grep below) and the server startup wiring.

- [ ] **Step 1: Locate the retention worker + the place writers are constructed**

Run:
```bash
grep -rn "ArtifactRetentionWorker\|soft_delete_expired_artifacts\|artifact_gc_candidates" crates/kyma-server/src crates/kyma-bin/src
grep -rn "shared.format\|shared.catalog\|MemoryWriter::new" crates/kyma-server/src/agent | head
```
This gives the worker's tick loop and the `catalog` + `format` (`SegmentFormat`) handles needed to build an `ArtifactGraphWriter`.

- [ ] **Step 2: Add a free function the worker and startup both call**

Create `crates/kyma-server/src/agent/artifact_graph_sync.rs`:

```rust
//! Drives the catch-all `artifacts` graph: reads the Postgres artifact catalog
//! and materializes nodes for non-producer-attached artifacts.

use std::sync::Arc;

use kyma_artifact_graph::ArtifactGraphWriter;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;

/// Materialize artifact nodes for one tenant. No-op when the catalog is not
/// Postgres (local mode has no artifacts table). Returns nodes written.
pub async fn sync_artifact_nodes(
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    tenant: TenantId,
) -> anyhow::Result<usize> {
    let Some(pg) = catalog.as_ref_any().downcast_ref::<PostgresCatalog>() else {
        return Ok(0);
    };
    let records = pg.list_live_artifacts(tenant).await?;
    let writer = ArtifactGraphWriter::new(catalog.clone(), format);
    let n = writer.sync(&records).await?;
    Ok(n)
}
```

> Use the **same** `as_ref_any().downcast_ref::<PostgresCatalog>()` accessor the retention worker already uses to reach the Postgres-only artifact methods (the artifacts.rs header documents this path). Copy its exact form from the worker located in Step 1.

Register the module: add `pub mod artifact_graph_sync;` to `crates/kyma-server/src/agent/mod.rs` (next to the other `pub mod` lines).

- [ ] **Step 3: Add a dependency on the new crate**

In `crates/kyma-server/Cargo.toml` `[dependencies]`, add:
```toml
kyma-artifact-graph = { path = "../kyma-artifact-graph" }
```

- [ ] **Step 4: Call it from the retention worker tick + once at startup**

In the retention worker tick loop found in Step 1, after the GC passes, for each tenant the worker already iterates, add:
```rust
if let Err(e) = crate::agent::artifact_graph_sync::sync_artifact_nodes(
    catalog.clone(), format.clone(), tenant,
).await {
    tracing::warn!(error = %e, "artifact-graph sync failed");
}
```
Use the worker's existing `catalog`/`format`/`tenant` bindings (match their names). If the worker has no `format` handle, thread `shared.format.clone()` in from the same place it gets `catalog` (see the construction site from Step 1).

- [ ] **Step 5: Verify it compiles**

Run: `cargo build -p kyma-server`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/src/agent/artifact_graph_sync.rs crates/kyma-server/src/agent/mod.rs crates/kyma-server/Cargo.toml crates/kyma-server/src
git commit -m "feat(server): drive catch-all artifact-graph sync from the retention worker"
```

---

## Phase 3 — Polish: icon, discovery/search smoke test, docs

### Task 6: `Artifact` icon + end-to-end discovery/search check

**Files:**
- Modify: `packages/react/src/graph/graph-icons.tsx` (add an `artifact` mapping)
- Test: extend `packages/react/src/graph/GraphSidebar.test.tsx` or add an icon-map test

- [ ] **Step 1: Find the icon map**

Run: `grep -n "logfile\|LogFile\|ScrollText\|resolveGraphIcon\|=> ({" packages/react/src/graph/graph-icons.tsx`
This shows the label→icon map and the fallback.

- [ ] **Step 2: Write a failing test for the Artifact icon**

Add to `GraphSidebar.test.tsx` (or a new `graph-icons.test.tsx`):

```tsx
import { resolveGraphIcon } from "./graph-icons";

it("resolves an icon for the Artifact label", () => {
  expect(resolveGraphIcon(["Artifact"], {})).not.toBeNull();
});
```

- [ ] **Step 3: Run — verify it fails only if no fallback**

Run: `pnpm --filter @kyma-ai/react exec vitest run src/graph/GraphSidebar.test.tsx`
Expected: If `resolveGraphIcon` already returns a non-null fallback for unknown labels, this PASSES immediately — then make the change cosmetic (map `artifact` → `ScrollText`/`FileBox`) for a nicer glyph and keep the test green. If it returns null for unknown labels, it FAILS first.

- [ ] **Step 4: Add the mapping**

In `graph-icons.tsx`, add a case mapping the lowercased label `artifact` (and keep `logfile`) to a lucide icon already imported there (e.g. `ScrollText`), following the file's existing map syntax exactly.

- [ ] **Step 5: Run — verify pass**

Run: `pnpm --filter @kyma-ai/react test`
Expected: PASS.

- [ ] **Step 6: Manual discovery/search smoke check (documented, not automated)**

In a server-mode dev environment with at least one captured CI log:
1. Open the graph page → the `Artifact` label appears in the **Node types** legend.
2. Toggle every other label off → only `Artifact` nodes remain (the all-artifacts view).
3. Type `artifact` in the sidebar search → artifact nodes match (search keys off id + label).
4. Click an `Artifact` node → the inspector shows **View log** → the redacted blob loads and pages.

- [ ] **Step 7: Commit**

```bash
git add packages/react/src/graph/graph-icons.tsx packages/react/src/graph/GraphSidebar.test.tsx
git commit -m "feat(react): Artifact node icon + graph discovery polish"
```

---

### Task 7: Documentation + spec cross-reference

**Files:**
- Find + modify: the platform-enrichment / artifacts doc page (locate via grep)

- [ ] **Step 1: Locate the artifacts docs**

Run: `grep -rln "artifact\|github_job_logs\|object-store\|HAS_RUN" docs/ --include=*.md | grep -iv superpowers`

- [ ] **Step 2: Add a short "Artifacts on the graph" section**

In the located page (or a new short page under the platform-enrichment docs), document, in developer/operator voice (command/endpoint-first, no marketing — matches the repo's docs convention):
- CI logs render as `Artifact` nodes in the `github` graph, linked from their `Job` by `HAS_ARTIFACT`.
- Other artifacts (object-store, fs-watch, agent files) appear in the `artifacts` graph (server mode).
- Filter the unified canvas by the `Artifact` label; click a node to stream the blob.
- Cross-artifact content search is the unified `/v1/search` (Piece 1), not here.

- [ ] **Step 3: Commit**

```bash
git add docs
git commit -m "docs: artifacts on the graph page"
```

---

## Self-Review

**Spec coverage:**
- Node contract (single `Artifact` label, `artifact_id`, props) → Task 1 (github) + Task 3 (`artifact_node_row`). ✓
- Single `HAS_ARTIFACT` edge / `JOB_HAS_LOG` standardized → Task 1. ✓
- All sources: github producer-attached → Task 1; object-store/fs-watch/agent via catch-all → Tasks 3-5. ✓
- Catch-all `artifacts` graph (tables, registration, provisioner) → Task 3; sync/backfill → Tasks 4-5. ✓
- Searchability via id+label → verified in Task 6 Step 6 (no code change needed — rides existing `search_sql`). ✓
- Graph UI node + inline preview → Task 2 (widen existing `LogFileViewer` trigger) + Task 6 (icon). ✓
- Forward-only github relabel (no historical rewrite) → reflected: Task 1 changes emission only; documented in spec. ✓
- Out of scope (content search → Piece 1) → Task 7 doc note. ✓

**Placeholder scan:** No "TBD"/"implement later"/"add error handling". The two grep-to-locate steps (Tasks 5, 6, 7) carry the exact code to insert and the precise call to add; only the line number is discovered at runtime, which is unavoidable for worker/icon-map insertion points.

**Type consistency:**
- `log_file_rows(..., artifact_id: Option<&str>)` — defined Task 1 Step 3, called Task 1 Step 4 with `artifact_id.as_deref()`. ✓
- `ArtifactGraphWriter::{new, ensure_provisioned, sync}`, `artifact_node_row`, consts `ARTIFACTS_DB`/`GRAPH_NAME`/`NODE_TABLE`/`EDGE_TABLE` — defined Task 3, used in its test and Task 5. ✓
- `list_live_artifacts(tenant) -> Result<Vec<ArtifactRecord>>` — defined Task 4, called Task 5. ✓
- `artifactViewerPath(node)` — defined + used Task 2, tested Task 2/6. ✓

**Known runtime-verification points (flagged, not placeholders):** `WritePath::ingest_with_idempotency` argument order (copy from `memory/src/writer.rs:309`); the worker's `as_ref_any().downcast_ref::<PostgresCatalog>()` accessor + its `catalog`/`format`/`tenant` bindings; `arrow-schema` workspace-dep declaration. Each step names the exact precedent file to copy from.
