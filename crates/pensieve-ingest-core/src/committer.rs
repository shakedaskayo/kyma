//! Committer — the async half of the S2.2 router/committer split.
//!
//! When ingest runs in staged mode (`PENSIEVE_INGEST_MODE=staged`), routers write
//! the extent object + record its manifest in `staged_extents` + ack ("S3 is the
//! WAL"). This worker drains those staged rows, groups them by table, and commits
//! each group as one snapshot via [`Catalog::commit_staged_group`] — which
//! commits the snapshot AND deletes the staged rows in a single transaction, so
//! the move from staged → committed is exactly-once. A racing committer (multi
//! node) loses the snapshot `(table, sequence)` unique/CAS and, on retry, finds
//! the rows already destaged; nothing is double-committed or lost.
//!
//! Server-mode only (the staged catalog methods are no-ops on SQLite). Spawned
//! only when staged ingest is enabled; otherwise the synchronous write path is
//! unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use pensieve_core::catalog::Catalog;
use pensieve_core::errors::{CatalogError, Error};
use pensieve_core::tenant::TenantId;
use pensieve_core::types::TableId;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Drains `staged_extents` and batch-commits per table.
pub struct Committer {
    catalog: Arc<dyn Catalog>,
    tenant: TenantId,
    /// How often to drain.
    pub poll_interval: Duration,
    /// Max staged rows pulled per tick (across all tables).
    pub max_per_tick: i64,
}

impl Committer {
    pub fn new(catalog: Arc<dyn Catalog>, tenant: TenantId) -> Self {
        let window_ms = std::env::var("PENSIEVE_COMMIT_WINDOW_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1000);
        Self {
            catalog,
            tenant,
            poll_interval: Duration::from_millis(window_ms.max(1)),
            max_per_tick: 1024,
        }
    }

    /// One drain+commit pass. Returns the number of extents committed.
    pub async fn tick(&self) -> pensieve_core::errors::Result<usize> {
        let staged = self
            .catalog
            .list_staged_extents(self.tenant, self.max_per_tick)
            .await?;
        if staged.is_empty() {
            return Ok(0);
        }

        // Group by table — each table commits as its own snapshot.
        let mut groups: HashMap<TableId, Vec<pensieve_core::catalog::StagedExtentRow>> = HashMap::new();
        for row in staged {
            groups.entry(row.table_id).or_default().push(row);
        }

        let mut committed = 0usize;
        for (table_id, rows) in groups {
            let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.id).collect();
            let manifests: Vec<_> = rows.iter().map(|r| r.manifest.clone()).collect();
            // Retry on Conflict (a racing committer / concurrent snapshot bump).
            let mut attempt = 0;
            loop {
                match self
                    .catalog
                    .commit_staged_group(self.tenant, table_id, &ids, &manifests)
                    .await
                {
                    Ok(_) => {
                        committed += manifests.len();
                        break;
                    }
                    Err(Error::Catalog(CatalogError::Conflict)) if attempt < 20 => {
                        attempt += 1;
                        // Tiny backoff; re-read happens inside commit_staged_group.
                        tokio::time::sleep(Duration::from_millis(2 * attempt)).await;
                        continue;
                    }
                    Err(e) => {
                        warn!(table = %table_id, error = %e, "commit_staged_group failed");
                        break;
                    }
                }
            }
        }
        Ok(committed)
    }

    /// Run until `shutdown` fires.
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) {
        info!(
            poll_ms = self.poll_interval.as_millis() as u64,
            "committer started"
        );
        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("committer shutting down — final drain");
                    // Best-effort final drain so a clean shutdown commits acked rows.
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "committer final drain failed");
                    }
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    match self.tick().await {
                        Ok(n) if n > 0 => debug!(committed = n, "committer drained staged extents"),
                        Ok(_) => {}
                        Err(e) => warn!(error = %e, "committer tick failed"),
                    }
                }
            }
        }
    }
}
