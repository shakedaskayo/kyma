//! Graph-snapshot activation scheduler (S3.2).
//!
//! The deep-subgraph fast path ([`kyma_graph::StoredGraphProvider`]) can serve a
//! large-graph traversal from a persistent CSR snapshot instead of a per-hop SQL
//! loop — but only if a snapshot has been *built*. Building is a full edge scan,
//! so it can't run on the query path; something has to refresh snapshots out of
//! band. This scheduler is that something: it periodically scans every
//! registered graph and rebuilds the snapshot when the graph's edge table has
//! changed since the last build.
//!
//! Debounce signal: the edge table's `current_snapshot_id`. It advances on every
//! commit to that table and never moves backward, so it is a correct, cheap
//! staleness signal — if the live id still matches the marker stored beside the
//! snapshot ([`kyma_graph::snapshot::meta_path`]), the topology is unchanged and
//! the rebuild is skipped. A full edge scan therefore happens only after a real
//! write, not on every tick.
//!
//! Correctness never depends on a snapshot: a missing or stale one only makes a
//! deep query fall back to the per-hop path (slower, identical results). So a
//! failed rebuild is logged and retried next tick, never fatal.
//!
//! The same loop runs in the server (`kyma serve` / the distributed query node)
//! and in local mode (`kyma-local`), since both build their graph providers from
//! the same catalog + format pair.

use std::sync::Arc;
use std::time::Duration;

use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::graph_handler::stored_provider;

/// Periodically (re)builds persistent graph-topology snapshots for every
/// registered graph whose edge table has advanced since its last snapshot.
pub struct GraphSnapshotScheduler {
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    /// Sleep between sweeps.
    pub poll_interval: Duration,
}

/// Whether a snapshot must be rebuilt: there is no marker yet (never built, or
/// built before markers existed), or the live source version differs from the
/// recorded one. Pure so it can be unit-tested without a catalog.
fn needs_rebuild(last_marker: Option<&str>, current_version: &str) -> bool {
    last_marker != Some(current_version)
}

impl GraphSnapshotScheduler {
    fn default_poll_interval() -> Duration {
        Duration::from_secs(
            std::env::var("KYMA_GRAPH_SNAPSHOT_POLL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
        )
    }

    pub fn new(catalog: Arc<dyn Catalog>, format: Arc<dyn SegmentFormat>) -> Self {
        Self {
            catalog,
            format,
            poll_interval: Self::default_poll_interval(),
        }
    }

    /// Run one sweep across every registered graph. Returns the number of
    /// snapshots (re)built this sweep.
    pub async fn tick(&self) -> anyhow::Result<usize> {
        // No object store (e.g. a format that doesn't persist) → nothing to do.
        let Some(store) = self.format.object_store() else {
            debug!("no object store: graph snapshot scheduler idle");
            return Ok(0);
        };

        let mut built = 0usize;
        let databases = self
            .catalog
            .list_databases()
            .await
            .map_err(|e| anyhow::anyhow!("list_databases: {e}"))?;
        for db in databases {
            let graphs = match self.catalog.list_graphs(&db).await {
                Ok(g) => g,
                Err(e) => {
                    warn!(database = %db, error = %e, "list_graphs failed; skipping db");
                    continue;
                }
            };
            if graphs.is_empty() {
                continue;
            }
            // Resolve edge-table snapshot ids once per db.
            let tables = match self.catalog.list_tables_in_database(&db).await {
                Ok(t) => t,
                Err(e) => {
                    warn!(database = %db, error = %e, "list_tables failed; skipping db");
                    continue;
                }
            };
            for reg in graphs {
                let Some(edge_tbl) = tables.iter().find(|t| t.name == reg.edge_table) else {
                    // Edge table missing (mid-teardown / misconfigured graph).
                    debug!(database = %db, edge_table = %reg.edge_table,
                           "edge table not found; skipping graph");
                    continue;
                };
                let current_version = edge_tbl.current_snapshot_id.to_string();
                let marker = kyma_graph::snapshot::meta_path(&db, &reg.edge_table);
                let last = kyma_graph::snapshot::load_snapshot_meta(&store, &marker)
                    .await
                    .unwrap_or(None);
                if !needs_rebuild(last.as_deref(), &current_version) {
                    continue; // debounced: source unchanged since last snapshot
                }

                let provider = stored_provider(&self.catalog, &self.format, reg.clone());
                match provider.build_and_store_snapshot(&store).await {
                    Ok(()) => {
                        // Mark the source version *after* the snapshot lands, so a
                        // crash between the two just rebuilds next tick (never
                        // marks a snapshot that wasn't written).
                        if let Err(e) =
                            kyma_graph::snapshot::store_snapshot_meta(&store, &marker, &current_version)
                                .await
                        {
                            warn!(database = %db, graph = %reg.edge_table, error = %e,
                                  "snapshot built but marker write failed; will rebuild next tick");
                        }
                        built += 1;
                        debug!(database = %db, edge_table = %reg.edge_table,
                               version = %current_version, "graph snapshot rebuilt");
                    }
                    Err(e) => {
                        warn!(database = %db, edge_table = %reg.edge_table, error = %e,
                              "graph snapshot build failed; will retry next tick");
                    }
                }
            }
        }
        if built > 0 {
            info!(snapshots = built, "graph snapshot scheduler rebuilt snapshots");
        }
        Ok(built)
    }

    /// Run the scheduler until `shutdown` fires.
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) {
        info!(
            poll_secs = self.poll_interval.as_secs(),
            "graph snapshot scheduler started"
        );
        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    info!("graph snapshot scheduler shutting down");
                    break;
                }
                () = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "graph snapshot scheduler tick failed");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::needs_rebuild;

    #[test]
    fn rebuild_decision_debounces_on_unchanged_version() {
        // No marker yet → must build.
        assert!(needs_rebuild(None, "snap-1"));
        // Marker matches live version → skip.
        assert!(!needs_rebuild(Some("snap-1"), "snap-1"));
        // Source advanced → rebuild.
        assert!(needs_rebuild(Some("snap-1"), "snap-2"));
    }
}
