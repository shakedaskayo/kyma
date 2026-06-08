//! Artifact retention worker — the object-store-blob sibling of
//! [`crate::retention::PhysicalDeleteWorker`].
//!
//! Standalone artifacts (CI job logs, contributed files, fs-watch snapshots)
//! live outside the columnar extents, so the extent GC never sees them. This
//! worker sweeps them on the same soft-delete + grace pattern:
//!
//!   * Pass 1 — soft-delete artifacts whose `expires_at` is past now.
//!   * Pass 2 — after `grace_period`, delete the blob then the catalog row
//!     (blob first, so a row referencing a vanished object is the worse-but-safe
//!     failure rather than an orphaned object).
//!
//! Postgres-only (the artifact catalog is), mirroring the retention worker.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::errors::{Error, Result};
use object_store::path::Path;
use object_store::ObjectStore;
use tracing::{info, warn};

fn pg(catalog: &Arc<dyn Catalog>) -> Result<&PostgresCatalog> {
    catalog
        .as_ref_any()
        .downcast_ref::<PostgresCatalog>()
        .ok_or_else(|| Error::Internal("artifact retention requires PostgresCatalog".into()))
}

/// Counts from one sweep, returned for observability + tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactSweepStats {
    pub soft_deleted: usize,
    pub blobs_deleted: usize,
    pub rows_deleted: usize,
}

#[derive(Clone)]
pub struct ArtifactRetentionWorker {
    catalog: Arc<dyn Catalog>,
    store: Arc<dyn ObjectStore>,
    pub poll_interval: Duration,
    pub grace_period: chrono::Duration,
}

impl ArtifactRetentionWorker {
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
            "artifact-retention worker starting"
        );
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("artifact-retention worker shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "artifact-retention tick error");
                    }
                }
            }
        }
    }

    /// One sweep: soft-delete expired, then physically delete past-grace blobs +
    /// rows. Returns counts. Public for tests + one-shot invocation.
    pub async fn tick(&self) -> Result<ArtifactSweepStats> {
        let pg = pg(&self.catalog)?;
        let now = Utc::now();
        let mut stats = ArtifactSweepStats::default();

        // Pass 0: (re)stamp `expires_at` from each tenant's retention settings so
        // newly-written artifacts (expires_at = NULL) become sweepable. Idempotent
        // and best-effort — a stamping failure must not block the GC.
        match pg.retention_tenants().await {
            Ok(tenants) => {
                for t in tenants {
                    match pg.load_retention_settings(t).await {
                        Ok(settings) => {
                            if let Err(e) = pg.restamp_artifact_expiry(t, &settings).await {
                                warn!(tenant = %t.as_uuid(), error = %e, "artifact expiry restamp failed");
                            }
                        }
                        Err(e) => {
                            warn!(tenant = %t.as_uuid(), error = %e, "load retention settings failed")
                        }
                    }
                }
            }
            Err(e) => warn!(error = %e, "listing retention tenants failed"),
        }

        // Pass 1: soft-delete expired artifacts.
        let swept = pg.soft_delete_expired_artifacts(now).await?;
        stats.soft_deleted = swept.len();
        if !swept.is_empty() {
            info!(count = swept.len(), "artifact-retention: soft-deleted expired");
            ::metrics::counter!("kyma_artifact_retention_soft_deleted_total")
                .increment(swept.len() as u64);
        }

        // Pass 2: physically delete rows soft-deleted before the grace cutoff.
        let cutoff = now - self.grace_period;
        let candidates = pg.artifact_gc_candidates(cutoff).await?;
        if candidates.is_empty() {
            return Ok(stats);
        }

        let mut deleted_ids = Vec::with_capacity(candidates.len());
        let mut failed: u64 = 0;
        for (id, path) in &candidates {
            match self.store.delete(&Path::from(path.as_str())).await {
                Ok(()) => deleted_ids.push(*id),
                // Object already gone (or never written) — still drop the row.
                Err(object_store::Error::NotFound { .. }) => deleted_ids.push(*id),
                Err(e) => {
                    warn!(object_path = %path, error = %e, "artifact-gc: blob delete failed");
                    failed += 1;
                }
            }
        }
        stats.blobs_deleted = deleted_ids.len();

        pg.delete_artifact_rows(&deleted_ids).await?;
        stats.rows_deleted = deleted_ids.len();

        ::metrics::counter!("kyma_artifact_gc_blobs_deleted_total")
            .increment(deleted_ids.len() as u64);
        if failed > 0 {
            ::metrics::counter!("kyma_artifact_gc_blob_delete_failed_total").increment(failed);
        }
        info!(
            blobs_deleted = stats.blobs_deleted,
            rows_deleted = stats.rows_deleted,
            failed,
            "artifact-gc complete"
        );
        Ok(stats)
    }
}
