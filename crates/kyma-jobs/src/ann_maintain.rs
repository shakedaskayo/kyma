//! `ann_maintain` executor: (re)build the global ANN centroid tree (S1.3).
//!
//! Reads every live extent's IVF sidecar centroids for the configured column,
//! clusters them into a global tree ([`kyma_index_vector::global_tree`]),
//! uploads the serialized tree, and upserts the `ann_tree` catalog row. The
//! debounced scheduler enqueues this whenever the set of sidecar-bearing extents
//! changes; the executor is idempotent — if the live extent set still hashes to
//! the stored tree's `extent_fingerprint`, it no-ops.
//!
//! It never touches raw vectors: the per-extent IVF centroids are already
//! computed and stored, so a full rebuild is cheap. Correctness never depends on
//! the tree (the query path falls back to per-extent fan-out + exact rerank), so
//! a failed rebuild only delays a latency win.

use std::sync::Arc;

use crate::executor::{JobCtx, JobError, JobExecutor};
use async_trait::async_trait;
use kyma_core::catalog::{Catalog, PrunePredicate, TableRef};
use kyma_core::crypto::content_hash_hex;
use kyma_core::fabric::JOB_ANN_MAINTAIN;
use kyma_core::index_sidecar::{AnnTreeDescriptor, SidecarKind};
use kyma_core::tenant::TenantId;
use kyma_core::types::{ExtentId, TableId};
use kyma_index_vector::{file as ivf_file, global_tree};
use object_store::{path::Path as ObjPath, ObjectStore};
use serde::Deserialize;
use serde_json::{json, Value as Json};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct Payload {
    table_id: Uuid,
    column: String,
    #[serde(default)]
    embedding_model_id: Option<String>,
}

/// Builds + maintains the global ANN centroid tree from per-extent sidecars.
pub struct AnnMaintainExecutor {
    catalog: Arc<dyn Catalog>,
    store: Arc<dyn ObjectStore>,
}

impl AnnMaintainExecutor {
    pub fn new(catalog: Arc<dyn Catalog>, store: Arc<dyn ObjectStore>) -> Self {
        Self { catalog, store }
    }

    /// Object-store path for a table+column+model tree blob.
    pub fn tree_path(
        tenant: TenantId,
        table_id: TableId,
        column: &str,
        model: Option<&str>,
    ) -> String {
        let m = model.unwrap_or("none").replace('/', "_");
        format!("{tenant}/ann_tree/{table_id}/{column}__{m}.kygt")
    }

    /// Resolve a `TableRef` by id across the tenant's databases (the `Catalog`
    /// trait has no lookup-by-id; mirrors the index_build executor).
    async fn table_by_id(
        &self,
        tenant: TenantId,
        table_id: TableId,
    ) -> Result<Option<TableRef>, JobError> {
        let dbs = self
            .catalog
            .list_databases_in_tenant(tenant)
            .await
            .map_err(|e| JobError::Transient(format!("list databases: {e}")))?;
        for db in dbs {
            let tables = self
                .catalog
                .list_tables_in_database_in_tenant(tenant, &db)
                .await
                .map_err(|e| JobError::Transient(format!("list tables in {db}: {e}")))?;
            if let Some(t) = tables.into_iter().find(|t| t.id == table_id) {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }
}

/// Stable hash of a sorted extent-id set — the staleness signal. Distinct sets
/// (any add/remove) hash differently; the same set always hashes identically.
fn fingerprint(extent_ids: &[ExtentId]) -> String {
    let mut ids: Vec<[u8; 16]> = extent_ids.iter().map(|e| *e.as_uuid().as_bytes()).collect();
    ids.sort_unstable();
    let mut bytes = Vec::with_capacity(ids.len() * 16);
    for id in &ids {
        bytes.extend_from_slice(id);
    }
    content_hash_hex(&bytes)
}

#[async_trait]
impl JobExecutor for AnnMaintainExecutor {
    fn kind(&self) -> &'static str {
        JOB_ANN_MAINTAIN
    }

    async fn run(
        &self,
        ctx: &JobCtx,
        job: &kyma_core::fabric::ClaimedJob,
    ) -> Result<Json, JobError> {
        let payload: Payload = serde_json::from_value(job.payload.clone())
            .map_err(|e| JobError::Config(format!("bad ann_maintain payload: {e}")))?;
        let tenant = TenantId::from_uuid(job.tenant_id);
        let table_id = TableId::from_uuid(payload.table_id);
        let model = payload.embedding_model_id.as_deref();

        let Some(table) = self.table_by_id(tenant, table_id).await? else {
            return Err(JobError::Permanent(format!(
                "ann_maintain: table {table_id} no longer exists"
            )));
        };

        // Live extents + their IvfRabitq sidecars for this column+model.
        let live = self
            .catalog
            .list_extents_in_tenant(
                tenant,
                table_id,
                table.current_snapshot_id,
                &PrunePredicate::default(),
            )
            .await
            .map_err(|e| JobError::Transient(format!("list extents: {e}")))?;
        let extent_ids: Vec<ExtentId> = live.iter().map(|m| m.id).collect();
        let sidecars = self
            .catalog
            .list_index_sidecars(tenant, table_id, &extent_ids, Some(SidecarKind::IvfRabitq))
            .await
            .map_err(|e| JobError::Transient(format!("list sidecars: {e}")))?;
        let mut relevant: Vec<_> = sidecars
            .into_iter()
            .filter(|d| d.column == payload.column && d.embedding_model_id.as_deref() == model)
            .collect();
        relevant.sort_by_key(|d| *d.extent_id.as_uuid());

        // No covered extents → drop any stale tree so the query path fans out.
        if relevant.is_empty() {
            let _ = self
                .catalog
                .delete_ann_tree(tenant, table_id, &payload.column)
                .await;
            return Ok(json!({ "status": "no_sidecars", "global_nlist": 0 }));
        }

        let covered: Vec<ExtentId> = relevant.iter().map(|d| d.extent_id).collect();
        let fp = fingerprint(&covered);

        // Idempotency: unchanged extent set → keep the existing tree.
        let existing = self
            .catalog
            .get_ann_tree(tenant, table_id, &payload.column, model)
            .await
            .map_err(|e| JobError::Transient(format!("get ann_tree: {e}")))?;
        if let Some(ref t) = existing {
            if t.extent_fingerprint == fp {
                return Ok(json!({ "status": "unchanged", "generation": t.generation }));
            }
        }

        // Read each sidecar's centroids.
        let mut extent_centroids: Vec<(Uuid, Vec<Vec<f32>>)> = Vec::with_capacity(relevant.len());
        let mut dim: Option<usize> = None;
        for (i, d) in relevant.iter().enumerate() {
            ctx.progress
                .push(json!({
                    "current_phase": "reading_centroids",
                    "pct": (i * 100) / relevant.len().max(1),
                }))
                .await;
            let bytes = self
                .store
                .get(&ObjPath::from(d.object_path.as_str()))
                .await
                .map_err(|e| JobError::Transient(format!("get sidecar {}: {e}", d.object_path)))?
                .bytes()
                .await
                .map_err(|e| JobError::Transient(format!("read sidecar {}: {e}", d.object_path)))?;
            let header = ivf_file::read_header(&bytes).map_err(|e| {
                JobError::Permanent(format!("sidecar header {}: {e}", d.object_path))
            })?;
            let centroids = ivf_file::read_centroids(&bytes, &header).map_err(|e| {
                JobError::Permanent(format!("sidecar centroids {}: {e}", d.object_path))
            })?;
            match dim {
                None => dim = Some(header.dim as usize),
                Some(prev) if prev != header.dim as usize => {
                    return Err(JobError::Permanent(format!(
                        "ann_maintain: mixed sidecar dims ({prev} vs {})",
                        header.dim
                    )));
                }
                _ => {}
            }
            extent_centroids.push((*d.extent_id.as_uuid(), centroids));
        }
        let dim = dim.ok_or_else(|| JobError::Permanent("ann_maintain: no dim".into()))?;

        let total_partitions: usize = extent_centroids.iter().map(|(_, c)| c.len()).sum();
        let global_nlist = global_tree::choose_global_nlist(total_partitions);
        let Some(tree) = global_tree::build(
            dim,
            ivf_file::METRIC_COSINE,
            model.map(str::to_string),
            &extent_centroids,
            global_nlist,
        ) else {
            return Ok(json!({ "status": "empty", "global_nlist": 0 }));
        };

        // Upload then register (object first, so a crash leaves only an orphan).
        let blob = tree.write();
        let path = Self::tree_path(tenant, table_id, &payload.column, model);
        let byte_size = blob.len() as u64;
        self.store
            .put(&ObjPath::from(path.as_str()), blob.into())
            .await
            .map_err(|e| JobError::Transient(format!("put tree {path}: {e}")))?;

        let generation = existing.map(|t| t.generation + 1).unwrap_or(1);
        let desc = AnnTreeDescriptor {
            id: Uuid::new_v4(),
            table_id,
            column: payload.column.clone(),
            embedding_model_id: payload.embedding_model_id.clone(),
            generation,
            extent_fingerprint: fp,
            object_path: path,
            byte_size,
            params: json!({
                "global_nlist": global_nlist,
                "dim": dim,
                "metric": "cosine",
                "source_extents": covered.len(),
                "total_partitions": total_partitions,
            }),
            created_at: now(),
        };
        self.catalog
            .upsert_ann_tree(tenant, &desc)
            .await
            .map_err(|e| JobError::Transient(format!("upsert ann_tree: {e}")))?;

        Ok(json!({
            "status": "built",
            "generation": generation,
            "global_nlist": global_nlist,
            "source_extents": covered.len(),
            "total_partitions": total_partitions,
        }))
    }
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}
