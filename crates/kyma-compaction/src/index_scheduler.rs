//! Index-build activation scheduler.
//!
//! The per-extent ANN sidecars ([`kyma_index_vector`]) are built by the async
//! `index_build` fabric job, but something has to *enqueue* those jobs.
//! Compaction only propagates sidecars that already existed on the inputs — so
//! without this scheduler a vector column would never get its first sidecar
//! (chicken-and-egg). This scheduler closes that gap: it periodically scans
//! every table for `FixedSizeList<Float32>` embedding columns and enqueues an
//! `index_build` job for any live extent that has no `ivf_rabitq` sidecar yet.
//!
//! The `IndexBuildExecutor` is idempotent (it skips extents already indexed),
//! so a job enqueued twice (e.g. while a prior job is still pending) is
//! harmless — it just no-ops. Coverage therefore converges monotonically.
//!
//! **Scope (v1):** vector columns only. Text-column FTS activation needs a
//! per-table column policy (which text column to index) and lands with the
//! embedding-pipeline phase. Enqueue is Postgres-only (the fabric queue lives
//! in Postgres); a non-Postgres catalog (local SQLite mode) is skipped with a
//! debug log — local mode builds sidecars inline / on demand instead.

use std::sync::Arc;
use std::time::Duration;

use arrow_schema::DataType;
use kyma_core::catalog::{Catalog, TableRef};
use kyma_core::errors::{Error, Result};
use kyma_core::index_sidecar::SidecarKind;
use kyma_core::tenant::TenantId;
use kyma_core::DEFAULT_TENANT;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Periodically enqueues `index_build` jobs for un-indexed vector columns.
pub struct IndexScheduler {
    catalog: Arc<dyn Catalog>,
    /// Sleep between sweeps.
    pub poll_interval: Duration,
    /// Max extents bundled into a single `index_build` job.
    pub max_extents_per_job: usize,
}

impl IndexScheduler {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self {
            catalog,
            poll_interval: Duration::from_secs(
                std::env::var("KYMA_INDEX_SCHED_POLL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            ),
            max_extents_per_job: 32,
        }
    }

    /// Names of `FixedSizeList<Float32>` embedding columns in a table.
    fn vector_columns(table: &TableRef) -> Vec<String> {
        table
            .schema
            .fields()
            .iter()
            .filter(|f| {
                matches!(f.data_type(), DataType::FixedSizeList(inner, _)
                    if matches!(inner.data_type(), DataType::Float32))
            })
            .map(|f| f.name().clone())
            .collect()
    }

    /// Run one sweep across all tables. Returns the number of jobs enqueued.
    pub async fn tick(&self) -> Result<usize> {
        // The fabric queue is Postgres-backed; resolve it once. Non-PG catalogs
        // (local SQLite) have no fabric queue — nothing to enqueue.
        let any_ref: &dyn std::any::Any = self.catalog.as_ref_any();
        let Some(pg) = any_ref.downcast_ref::<kyma_catalog::PostgresCatalog>() else {
            debug!("non-Postgres catalog: index scheduler idle");
            return Ok(0);
        };
        let fabric = kyma_catalog::PgFabricStore::new(pg.pool().clone());

        let mut enqueued = 0usize;
        let databases = self
            .catalog
            .list_databases()
            .await
            .map_err(|e| Error::Internal(format!("list_databases: {e}")))?;
        for db in databases {
            let tables = self.catalog.list_tables_in_database(&db).await?;
            for table in tables {
                let cols = Self::vector_columns(&table);
                if cols.is_empty() {
                    continue;
                }
                for col in cols {
                    enqueued += self
                        .enqueue_missing(&fabric, DEFAULT_TENANT, &table, &col)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(table = %table.name, column = %col, error = %e,
                                  "index enqueue failed");
                            0
                        });
                }
            }
        }
        if enqueued > 0 {
            info!(
                jobs = enqueued,
                "index scheduler enqueued ivf_rabitq build jobs"
            );
        }
        Ok(enqueued)
    }

    /// Enqueue `index_build` jobs for every live extent of `(table, column)`
    /// that lacks an `ivf_rabitq` sidecar. Returns the number of jobs enqueued.
    async fn enqueue_missing(
        &self,
        fabric: &kyma_catalog::PgFabricStore,
        tenant: TenantId,
        table: &TableRef,
        column: &str,
    ) -> Result<usize> {
        let extents = self
            .catalog
            .list_extents_in_tenant(
                tenant,
                table.id,
                table.current_snapshot_id,
                &Default::default(),
            )
            .await?;
        if extents.is_empty() {
            return Ok(0);
        }
        let extent_ids: Vec<_> = extents.iter().map(|m| m.id).collect();
        let have: std::collections::HashSet<_> = self
            .catalog
            .list_index_sidecars(tenant, table.id, &extent_ids, Some(SidecarKind::IvfRabitq))
            .await?
            .into_iter()
            .filter(|d| d.column == column)
            .map(|d| d.extent_id)
            .collect();

        let missing: Vec<_> = extent_ids
            .into_iter()
            .filter(|id| !have.contains(id))
            .collect();
        if missing.is_empty() {
            return Ok(0);
        }

        let mut jobs = 0usize;
        for chunk in missing.chunks(self.max_extents_per_job) {
            let payload = serde_json::json!({
                "table_id": table.id.as_uuid(),
                "extent_ids": chunk.iter().map(|e| e.as_uuid()).collect::<Vec<_>>(),
                "column": column,
                "kind": SidecarKind::IvfRabitq.as_str(),
                "params": {},
            });
            let job = kyma_core::fabric::EnqueueJob {
                kind: kyma_core::fabric::JOB_INDEX_BUILD.to_string(),
                payload,
                priority: 0,
                affinity_worker_id: None,
                req_capabilities: Vec::new(),
                label_selector: serde_json::json!({}),
                max_attempts: 3,
            };
            fabric
                .enqueue_job(tenant, &job)
                .await
                .map_err(|e| Error::Internal(format!("enqueue index_build: {e}")))?;
            jobs += 1;
        }
        debug!(table = %table.name, column, jobs, extents = missing.len(),
               "enqueued ivf_rabitq builds");
        Ok(jobs)
    }

    /// Run the scheduler until `shutdown` fires.
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) {
        info!(
            poll_secs = self.poll_interval.as_secs(),
            "index scheduler started"
        );
        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("index scheduler shutting down");
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "index scheduler tick failed");
                    }
                }
            }
        }
    }
}
