//! Retention sweeper + physical-delete worker.
//!
//! - [`RetentionSweeper`]: soft-deletes extents older than each table's
//!   configured `retention_days`. Commits one snapshot per table so the
//!   retention event is observable in the snapshot chain + audit log.
//!
//! - [`PhysicalDeleteWorker`]: actually removes the bytes. Periodically
//!   fetches extents soft-deleted longer than `grace_period` ago, deletes
//!   their object-store files, then deletes their catalog rows.
//!
//! Why two passes: the grace period protects in-flight queries that started
//! before the soft-delete and still hold the older snapshot id. 24 h grace
//! is the default (configurable per-table in a future slice).

use chrono::Utc;
use kyma_core::catalog::{Catalog, SnapshotSummary};
use kyma_core::errors::{CatalogError, Error, Result};
use kyma_core::types::{ExtentId, TableId};
use object_store::path::Path;
use object_store::ObjectStore;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

fn catalog_pool(catalog: &Arc<dyn Catalog>) -> Result<&sqlx::PgPool> {
    let any_ref: &dyn std::any::Any = catalog.as_ref_any();
    if let Some(pg) = any_ref.downcast_ref::<kyma_catalog::PostgresCatalog>() {
        Ok(pg.pool())
    } else {
        Err(Error::Internal(
            "retention requires PostgresCatalog (phase-C shortcut)".into(),
        ))
    }
}

// --------------------------------------------------------------------------
// RetentionSweeper
// --------------------------------------------------------------------------

#[derive(Clone)]
pub struct RetentionSweeper {
    catalog: Arc<dyn Catalog>,
    pub poll_interval: Duration,
}

impl RetentionSweeper {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self {
            catalog,
            poll_interval: Duration::from_secs(60),
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!("retention sweeper starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("retention sweeper shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "retention sweeper tick error");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> Result<()> {
        let pool = catalog_pool(&self.catalog)?;
        // Pull (table_id, retention_days, extent_id) for every live extent
        // past retention in a single query.
        let rows = sqlx::query(
            "SELECT e.table_id,
                    (t.config->>'retention_days')::int   AS retention_days,
                    e.id
             FROM extents e
             JOIN tables  t ON t.id = e.table_id
             WHERE e.deleted_at IS NULL
               AND (t.config->>'retention_days') IS NOT NULL
               AND e.max_timestamp IS NOT NULL
               AND e.max_timestamp < now() - ((t.config->>'retention_days')::int || ' days')::interval",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Catalog(CatalogError::Sql(e.to_string())))?;

        use sqlx::Row;
        let mut by_table: HashMap<TableId, Vec<ExtentId>> = HashMap::new();
        for row in rows {
            let tid: uuid::Uuid = row
                .try_get("table_id")
                .map_err(|e| Error::Catalog(CatalogError::Sql(e.to_string())))?;
            let eid: uuid::Uuid = row
                .try_get("id")
                .map_err(|e| Error::Catalog(CatalogError::Sql(e.to_string())))?;
            by_table
                .entry(TableId::from_uuid(tid))
                .or_default()
                .push(ExtentId::from_uuid(eid));
        }
        if by_table.is_empty() {
            return Ok(());
        }
        info!(
            tables_affected = by_table.len(),
            total_expired = by_table.values().map(|v| v.len()).sum::<usize>(),
            "retention: soft-deleting expired extents"
        );

        // One snapshot per table so the event is audit-visible.
        for (table_id, ids) in by_table {
            let mut attempts = 0;
            loop {
                attempts += 1;
                let mut txn = self.catalog.begin_snapshot(table_id).await?;
                txn.remove_extents(&ids).await?;
                match txn
                    .commit(SnapshotSummary {
                        rows_removed: 0,
                        bytes_removed: 0,
                        operation: "retention".into(),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(_) => {
                        debug!(table_id = %table_id, count = ids.len(), "retention commit ok");
                        ::metrics::counter!("kyma_retention_extents_soft_deleted_total")
                            .increment(ids.len() as u64);
                        break;
                    }
                    Err(Error::Catalog(CatalogError::Conflict)) if attempts < 5 => {
                        tokio::time::sleep(Duration::from_millis(25 * attempts as u64)).await;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------
// PhysicalDeleteWorker
// --------------------------------------------------------------------------

#[derive(Clone)]
pub struct PhysicalDeleteWorker {
    catalog: Arc<dyn Catalog>,
    store: Arc<dyn ObjectStore>,
    pub poll_interval: Duration,
    pub grace_period: chrono::Duration,
}

impl PhysicalDeleteWorker {
    pub fn new(catalog: Arc<dyn Catalog>, store: Arc<dyn ObjectStore>) -> Self {
        Self {
            catalog,
            store,
            poll_interval: Duration::from_secs(300),
            grace_period: chrono::Duration::hours(24),
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(
            grace_hours = self.grace_period.num_hours(),
            "physical-gc worker starting"
        );
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("physical-gc worker shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "physical-gc tick error");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> Result<()> {
        let cutoff = Utc::now() - self.grace_period;
        let ids = self.catalog.gc_candidates(cutoff).await?;
        if ids.is_empty() {
            return Ok(());
        }
        info!(count = ids.len(), "physical-gc: deleting extent objects");

        // Look up object paths in one SQL roundtrip so we can delete the
        // MinIO objects before dropping the catalog rows.
        let pool = catalog_pool(&self.catalog)?;
        let uuids: Vec<uuid::Uuid> = ids.iter().map(|e| *e.as_uuid()).collect();
        let paths: Vec<(uuid::Uuid, String)> =
            sqlx::query_as("SELECT id, object_path FROM extents WHERE id = ANY($1)")
                .bind(&uuids)
                .fetch_all(pool)
                .await
                .map_err(|e| Error::Catalog(CatalogError::Sql(e.to_string())))?;

        // Index sidecars of the doomed extents: collect their object paths
        // before `delete_extent_rows` cascades the `extent_indexes` rows
        // away. Same grace period as the extent data — sidecars stay usable
        // for in-grace readers and vanish with the extent.
        let sidecar_paths: Vec<(String,)> = sqlx::query_as(
            "SELECT object_path FROM extent_indexes WHERE extent_id = ANY($1)",
        )
        .bind(&uuids)
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Catalog(CatalogError::Sql(e.to_string())))?;

        let mut deleted_objects: u64 = 0;
        let mut failed_objects: u64 = 0;
        let all_paths = paths
            .iter()
            .map(|(_, p)| p.as_str())
            .chain(sidecar_paths.iter().map(|(p,)| p.as_str()));
        for path in all_paths {
            match self.store.delete(&Path::from(path)).await {
                Ok(()) => {
                    deleted_objects += 1;
                }
                Err(e) => {
                    // Object may already be gone — log but don't abort the batch.
                    warn!(object_path = %path, error = %e, "physical-gc: object-store delete failed");
                    failed_objects += 1;
                }
            }
        }

        // Now remove the catalog rows. Done after object deletion so that a
        // catalog row still referencing a dangling path is a better failure
        // mode than an object referencing a vanished row.
        self.catalog.delete_extent_rows(&ids).await?;

        ::metrics::counter!("kyma_physical_gc_objects_deleted_total").increment(deleted_objects);
        if failed_objects > 0 {
            ::metrics::counter!("kyma_physical_gc_objects_delete_failed_total")
                .increment(failed_objects);
        }
        info!(
            deleted_objects,
            failed_objects,
            catalog_rows_deleted = ids.len(),
            "physical-gc complete"
        );
        Ok(())
    }
}
