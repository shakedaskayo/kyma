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
//! Enqueue targets either the Postgres fabric queue (server) or the local
//! SQLite `jobs` table (embedded mode) through the [`kyma_core::fabric::JobEnqueuer`]
//! abstraction: [`IndexScheduler::new`] resolves the Postgres queue by downcast,
//! and [`IndexScheduler::with_enqueuer`] takes an explicit enqueuer for local
//! mode. A scheduler with no enqueuer is idle.

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

/// Periodically enqueues `index_build` / `embed_backfill` jobs for un-indexed
/// vector + text columns. The enqueue target is abstracted
/// ([`kyma_core::fabric::JobEnqueuer`]) so the same scan logic runs against the
/// Postgres fabric queue (server) or the local SQLite `jobs` table (embedded).
pub struct IndexScheduler {
    catalog: Arc<dyn Catalog>,
    /// Where build jobs are enqueued. `None` when the catalog has no fabric
    /// queue (`new` over a non-Postgres catalog) — `tick` is then idle.
    enqueuer: Option<Arc<dyn kyma_core::fabric::JobEnqueuer>>,
    /// Sleep between sweeps.
    pub poll_interval: Duration,
    /// Max extents bundled into a single `index_build` job.
    pub max_extents_per_job: usize,
}

impl IndexScheduler {
    fn default_poll_interval() -> Duration {
        Duration::from_secs(
            std::env::var("KYMA_INDEX_SCHED_POLL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
        )
    }

    /// Server constructor: resolves the Postgres fabric queue by downcast. Over
    /// a non-Postgres catalog the scheduler is idle (local mode uses
    /// [`IndexScheduler::with_enqueuer`] + a SQLite enqueuer instead).
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        let enqueuer: Option<Arc<dyn kyma_core::fabric::JobEnqueuer>> = catalog
            .as_ref_any()
            .downcast_ref::<kyma_catalog::PostgresCatalog>()
            .map(|pg| {
                Arc::new(kyma_catalog::PgFabricStore::new(pg.pool().clone()))
                    as Arc<dyn kyma_core::fabric::JobEnqueuer>
            });
        Self {
            catalog,
            enqueuer,
            poll_interval: Self::default_poll_interval(),
            max_extents_per_job: 32,
        }
    }

    /// Explicit-enqueuer constructor for catalogs whose fabric queue isn't
    /// reachable by downcast (local SQLite mode passes a `SqliteEnqueuer`).
    pub fn with_enqueuer(
        catalog: Arc<dyn Catalog>,
        enqueuer: Arc<dyn kyma_core::fabric::JobEnqueuer>,
    ) -> Self {
        Self {
            catalog,
            enqueuer: Some(enqueuer),
            poll_interval: Self::default_poll_interval(),
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
        // Resolve the enqueue target once. A catalog with no fabric queue
        // (`new` over non-Postgres, no explicit enqueuer) leaves the scheduler
        // idle — nothing to enqueue.
        let Some(enqueuer) = self.enqueuer.clone() else {
            debug!("no fabric enqueuer: index scheduler idle");
            return Ok(0);
        };
        let fabric: &dyn kyma_core::fabric::JobEnqueuer = enqueuer.as_ref();

        let mut enqueued = 0usize;
        let databases = self
            .catalog
            .list_databases()
            .await
            .map_err(|e| Error::Internal(format!("list_databases: {e}")))?;
        let mut embed_jobs = 0usize;
        let mut fts_jobs = 0usize;
        for db in databases {
            let tables = self.catalog.list_tables_in_database(&db).await?;
            for table in tables {
                // (1) embed_backfill: tables configured to auto-embed a text
                //     column get their un-embedded extents filled first, so the
                //     ANN pass below has vectors to index. (2b) the SAME text
                //     column also gets a BM25 FTS index on embedded extents —
                //     configuring a table for semantic search enables lexical
                //     search on the same text for free.
                for cfg in self
                    .catalog
                    .list_table_embed_configs(DEFAULT_TENANT, table.id)
                    .await
                    .unwrap_or_default()
                {
                    if !cfg.auto_embed {
                        continue;
                    }
                    embed_jobs += self
                        .enqueue_embed_missing(fabric, DEFAULT_TENANT, &table, &cfg)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(table = %table.name, error = %e, "embed enqueue failed");
                            0
                        });
                    fts_jobs += self
                        .enqueue_fts_missing(fabric, DEFAULT_TENANT, &table, &cfg)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(table = %table.name, error = %e, "fts enqueue failed");
                            0
                        });
                }

                // (2a) ivf_rabitq sidecars for vector columns (incl. ones just
                //     filled by a prior backfill — their extents now carry vec
                //     stats and no sidecar yet).
                for col in Self::vector_columns(&table) {
                    enqueued += self
                        .enqueue_missing(fabric, DEFAULT_TENANT, &table, &col)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(table = %table.name, column = %col, error = %e,
                                  "index enqueue failed");
                            0
                        });
                }
            }
        }
        if enqueued > 0 || embed_jobs > 0 || fts_jobs > 0 {
            info!(
                ivf_rabitq = enqueued,
                embed_backfill = embed_jobs,
                tantivy_fts = fts_jobs,
                "index scheduler enqueued build jobs"
            );
        }
        Ok(enqueued + embed_jobs + fts_jobs)
    }

    /// Enqueue `tantivy_fts` (BM25) build jobs for the embed config's text
    /// source column, on extents that are already embedded (vec stats present
    /// on the embedding column) and lack an FTS sidecar. Gating on
    /// embedded-and-stable avoids building an FTS index on an extent that
    /// embed_backfill is about to rewrite (which would discard it).
    async fn enqueue_fts_missing(
        &self,
        fabric: &dyn kyma_core::fabric::JobEnqueuer,
        tenant: TenantId,
        table: &TableRef,
        cfg: &kyma_core::catalog::TableEmbedConfig,
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
        // Only stable (already-embedded) extents.
        let ready: Vec<_> = extents
            .iter()
            .filter(|m| {
                m.column_stats
                    .get(&cfg.embedding_column)
                    .and_then(|c| c.get("vec"))
                    .is_some()
            })
            .map(|m| m.id)
            .collect();
        if ready.is_empty() {
            return Ok(0);
        }
        let have: std::collections::HashSet<_> = self
            .catalog
            .list_index_sidecars(tenant, table.id, &ready, Some(SidecarKind::TantivyFts))
            .await?
            .into_iter()
            .filter(|d| d.column == cfg.source_column)
            .map(|d| d.extent_id)
            .collect();
        let missing: Vec<_> = ready.into_iter().filter(|id| !have.contains(id)).collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let mut jobs = 0usize;
        for chunk in missing.chunks(self.max_extents_per_job) {
            let payload = serde_json::json!({
                "table_id": table.id.as_uuid(),
                "extent_ids": chunk.iter().map(|e| e.as_uuid()).collect::<Vec<_>>(),
                "column": cfg.source_column,
                "kind": SidecarKind::TantivyFts.as_str(),
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
                .enqueue(tenant, &job)
                .await
                .map_err(|e| Error::Internal(format!("enqueue tantivy_fts: {e}")))?;
            jobs += 1;
        }
        debug!(table = %table.name, column = %cfg.source_column, jobs,
               extents = missing.len(), "enqueued tantivy_fts");
        Ok(jobs)
    }

    /// Enqueue `embed_backfill` jobs for every live extent of the configured
    /// table whose embedding column is not yet populated. The cheap signal: the
    /// TLM writer records per-extent vector stats (`column_stats[col]["vec"]`)
    /// only when the embedding column has non-null values — so an extent lacking
    /// those stats still needs embedding. After backfill rewrites the extent the
    /// stats appear, so it is not re-enqueued.
    async fn enqueue_embed_missing(
        &self,
        fabric: &dyn kyma_core::fabric::JobEnqueuer,
        tenant: TenantId,
        table: &TableRef,
        cfg: &kyma_core::catalog::TableEmbedConfig,
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
        let missing: Vec<_> = extents
            .iter()
            .filter(|m| m.row_count > 0)
            .filter(|m| {
                // Needs embedding iff the embedding column has no vec stats yet.
                m.column_stats
                    .get(&cfg.embedding_column)
                    .and_then(|c| c.get("vec"))
                    .is_none()
            })
            .map(|m| m.id)
            .collect();
        if missing.is_empty() {
            return Ok(0);
        }
        let mut jobs = 0usize;
        for chunk in missing.chunks(self.max_extents_per_job) {
            let payload = serde_json::json!({
                "table_id": table.id.as_uuid(),
                "extent_ids": chunk.iter().map(|e| e.as_uuid()).collect::<Vec<_>>(),
                "source_column": cfg.source_column,
                "embedding_column": cfg.embedding_column,
                "model_id": cfg.model_id,
            });
            let job = kyma_core::fabric::EnqueueJob {
                kind: kyma_core::fabric::JOB_EMBED_BACKFILL.to_string(),
                payload,
                priority: 0,
                affinity_worker_id: None,
                req_capabilities: Vec::new(),
                label_selector: serde_json::json!({}),
                max_attempts: 3,
            };
            fabric
                .enqueue(tenant, &job)
                .await
                .map_err(|e| Error::Internal(format!("enqueue embed_backfill: {e}")))?;
            jobs += 1;
        }
        debug!(table = %table.name, column = %cfg.embedding_column, jobs,
               extents = missing.len(), "enqueued embed_backfill");
        Ok(jobs)
    }

    /// Enqueue `index_build` jobs for every live extent of `(table, column)`
    /// that lacks an `ivf_rabitq` sidecar. Returns the number of jobs enqueued.
    async fn enqueue_missing(
        &self,
        fabric: &dyn kyma_core::fabric::JobEnqueuer,
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
        // Only index extents that actually carry vectors: the TLM writer
        // records `column_stats[col]["vec"]` when the column has non-null
        // embeddings. An extent without those stats either has no vectors yet
        // (scalar/un-embedded) or is awaiting embed_backfill — indexing it would
        // build a degenerate sidecar and be redone once it's embedded.
        let embedded_ids: Vec<_> = extents
            .iter()
            .filter(|m| {
                m.column_stats
                    .get(column)
                    .and_then(|c| c.get("vec"))
                    .is_some()
            })
            .map(|m| m.id)
            .collect();
        if embedded_ids.is_empty() {
            return Ok(0);
        }
        let have: std::collections::HashSet<_> = self
            .catalog
            .list_index_sidecars(
                tenant,
                table.id,
                &embedded_ids,
                Some(SidecarKind::IvfRabitq),
            )
            .await?
            .into_iter()
            .filter(|d| d.column == column)
            .map(|d| d.extent_id)
            .collect();

        let missing: Vec<_> = embedded_ids
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
                .enqueue(tenant, &job)
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
