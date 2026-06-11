//! Data source scheduler — enqueues connector_tick tasks for due data sources.

use crate::catalog_sql;
use kyma_catalog::PostgresCatalog;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct DataSourceScheduler {
    catalog: Arc<PostgresCatalog>,
    pub tick_interval: Duration,
}

impl DataSourceScheduler {
    pub fn new(catalog: Arc<PostgresCatalog>) -> Self {
        Self {
            catalog,
            tick_interval: Duration::from_millis(500),
        }
    }

    pub async fn tick_once(&self) -> Result<(), sqlx::Error> {
        let due = catalog_sql::list_due_periodic(self.catalog.pool()).await?;
        for c in due {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let bucketed = (now_ms / c.schedule_ms) * c.schedule_ms;
            // Each row carries its tenant_id; thread it through to the
            // jobs insert so the cluster-global scheduler stays
            // tenant-correct. DataSource syncs ride the worker fabric — the
            // embedded worker (or any remote worker advertising the
            // `data source` capability) claims and runs them.
            let inserted =
                catalog_sql::enqueue_connector_sync(self.catalog.pool(), c.tenant_id, c.id, bucketed)
                    .await?;
            if inserted > 0 {
                debug!(data_source = %c.name, bucketed, "enqueued connector_sync job");
            }
        }
        Ok(())
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!("connector scheduler starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("connector scheduler shutdown"); return; }
                _ = tokio::time::sleep(self.tick_interval) => {
                    if let Err(e) = self.tick_once().await {
                        error!(error = %e, "scheduler tick failed");
                    }
                }
            }
        }
    }
}
