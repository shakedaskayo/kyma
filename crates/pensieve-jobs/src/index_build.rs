//! `index_build` executor — builds index sidecars (ANN / FTS) for extents.
//!
//! Payload: [`IndexBuildPayload`]. For each extent the executor:
//!
//! 1. skips it if a sidecar for `(extent, column, kind)` is already
//!    registered (idempotent re-runs),
//! 2. opens it through the [`SegmentFormat`] and streams every block to the
//!    registered [`SidecarBuilder`] for the requested kind,
//! 3. uploads the built blob to `{tenant}/indexes/{extent_id}/{column}.{kind}`,
//! 4. registers the descriptor in the catalog (`extent_indexes`).
//!
//! Upload-then-register ordering means a crash leaves at worst an orphan
//! object — never a catalog row pointing at a missing object.
//!
//! S0 ships no production builders; the builder registry passed at
//! construction is the seam S1's IVF+RaBitQ and tantivy builders plug into.
//! A job for a kind with no registered builder fails terminally with a clear
//! error.

use crate::executor::{JobCtx, JobError, JobExecutor};
use async_trait::async_trait;
use futures::StreamExt;
use kyma_core::catalog::{Catalog, ExtentManifest, PrunePredicate, TableRef};
use kyma_core::fabric::{ClaimedJob, JOB_INDEX_BUILD};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, SidecarBuilder, SidecarKind};
use kyma_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use kyma_core::tenant::TenantId;
use kyma_core::types::{ExtentId, TableId};
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// `jobs.payload` for [`JOB_INDEX_BUILD`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBuildPayload {
    pub table_id: Uuid,
    pub extent_ids: Vec<Uuid>,
    /// Source column to index.
    pub column: String,
    pub kind: SidecarKind,
    /// Builder parameters, forwarded verbatim to the [`SidecarBuilder`].
    #[serde(default)]
    pub params: Json,
}

/// Executor for [`JOB_INDEX_BUILD`] jobs.
pub struct IndexBuildExecutor {
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    store: Arc<dyn ObjectStore>,
    builders: HashMap<SidecarKind, Arc<dyn SidecarBuilder>>,
}

impl IndexBuildExecutor {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        format: Arc<dyn SegmentFormat>,
        store: Arc<dyn ObjectStore>,
        builders: HashMap<SidecarKind, Arc<dyn SidecarBuilder>>,
    ) -> Self {
        Self {
            catalog,
            format,
            store,
            builders,
        }
    }

    /// Object-store path convention for sidecar blobs.
    pub fn object_path(
        tenant: TenantId,
        extent_id: ExtentId,
        column: &str,
        kind: SidecarKind,
    ) -> String {
        format!("{tenant}/indexes/{extent_id}/{column}.{}", kind.as_str())
    }

    /// Resolve a `TableRef` by id, scanning the tenant's databases. The
    /// `Catalog` trait has no direct lookup-by-id; this mirrors the
    /// compaction worker's approach so it works on any backend.
    async fn table_by_id(&self, tenant: TenantId, table_id: TableId) -> Result<Option<TableRef>, JobError> {
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

    /// Build + upload + register one extent's sidecar.
    async fn build_one(
        &self,
        tenant: TenantId,
        table: &TableRef,
        manifest: &ExtentManifest,
        column: &str,
        kind: SidecarKind,
        builder: &Arc<dyn SidecarBuilder>,
    ) -> Result<(), JobError> {
        let reader = self
            .format
            .open_extent(OpenExtentInput {
                extent_id: manifest.id,
                table_id: manifest.table_id,
                schema: table.schema.clone(),
                object_path: manifest.object_path.clone(),
                byte_size: manifest.byte_size,
            })
            .await
            .map_err(|e| JobError::Transient(format!("open extent {}: {e}", manifest.id)))?;

        let blocks = reader
            .pruned_blocks(&BlockPredicate::All)
            .await
            .map_err(|e| JobError::Transient(format!("list blocks of {}: {e}", manifest.id)))?;

        // Stream blocks lazily (all columns — the builder projects by name),
        // in block order so the builder can derive RowAddresses by counting.
        let r = reader.clone();
        let batches = futures::stream::iter(blocks)
            .then(move |bid| {
                let r = r.clone();
                async move { r.read_block(bid, &[]).await }
            })
            .boxed();

        let built = builder
            .build(manifest, column, batches)
            .await
            .map_err(|e| JobError::Transient(format!("build {kind} sidecar for {}: {e}", manifest.id)))?;

        // Upload first, then register: a crash in between leaves only an
        // orphan object, never a dangling catalog row.
        let path = Self::object_path(tenant, manifest.id, column, kind);
        let byte_size = built.bytes.len() as u64;
        self.store
            .put(&ObjPath::from(path.as_str()), built.bytes.into())
            .await
            .map_err(|e| JobError::Transient(format!("upload sidecar {path}: {e}")))?;

        let desc = IndexSidecarDescriptor {
            id: Uuid::new_v4(),
            extent_id: manifest.id,
            table_id: table.id,
            column: column.to_string(),
            kind,
            object_path: path,
            byte_size,
            params: built.params,
            embedding_model_id: built.embedding_model_id,
            created_at: chrono::Utc::now(),
        };
        self.catalog
            .register_index_sidecar(tenant, &desc)
            .await
            .map_err(|e| JobError::Transient(format!("register sidecar: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl JobExecutor for IndexBuildExecutor {
    fn kind(&self) -> &'static str {
        JOB_INDEX_BUILD
    }

    async fn run(&self, ctx: &JobCtx, job: &ClaimedJob) -> Result<Json, JobError> {
        let payload: IndexBuildPayload = serde_json::from_value(job.payload.clone())
            .map_err(|e| JobError::Config(format!("bad index_build payload: {e}")))?;
        let tenant = TenantId::from_uuid(job.tenant_id);

        let Some(builder) = self.builders.get(&payload.kind).cloned() else {
            return Err(JobError::Config(format!(
                "no sidecar builder registered for kind '{}' on this worker \
                 (S1 provides the ivf_rabitq / tantivy_fts builders)",
                payload.kind
            )));
        };

        let table_id = TableId::from_uuid(payload.table_id);
        let Some(table) = self.table_by_id(tenant, table_id).await? else {
            return Err(JobError::Permanent(format!(
                "index_build: table {table_id} no longer exists"
            )));
        };

        let wanted: Vec<ExtentId> = payload
            .extent_ids
            .iter()
            .map(|u| ExtentId::from_uuid(*u))
            .collect();

        // Idempotency: skip extents that already carry a sidecar for this
        // (column, kind) — re-runs and at-least-once delivery are no-ops.
        let existing = self
            .catalog
            .list_index_sidecars(tenant, table_id, &wanted, Some(payload.kind))
            .await
            .map_err(|e| JobError::Transient(format!("list existing sidecars: {e}")))?;
        let done: std::collections::HashSet<ExtentId> = existing
            .iter()
            .filter(|d| d.column == payload.column)
            .map(|d| d.extent_id)
            .collect();

        // Resolve live manifests; extents soft-deleted since enqueue are
        // silently skipped (their replacement got its own job).
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
        let by_id: HashMap<ExtentId, &ExtentManifest> = live.iter().map(|m| (m.id, m)).collect();

        let mut built = 0usize;
        let mut skipped = 0usize;
        for (i, eid) in wanted.iter().enumerate() {
            ctx.progress
                .push(json!({
                    "current_phase": "building",
                    "extent": eid.to_string(),
                    "pct": (i * 100) / wanted.len().max(1),
                }))
                .await;
            if done.contains(eid) {
                skipped += 1;
                continue;
            }
            let Some(manifest) = by_id.get(eid) else {
                skipped += 1;
                continue;
            };
            self.build_one(tenant, &table, manifest, &payload.column, payload.kind, &builder)
                .await?;
            built += 1;
        }

        Ok(json!({ "built": built, "skipped": skipped }))
    }
}
