//! Drives the catch-all `artifacts` graph: reads the Postgres artifact catalog
//! and materializes nodes for non-producer-attached artifacts. No-op when the
//! catalog is not Postgres (local mode has no artifacts table).

use std::sync::Arc;

use kyma_artifact_graph::ArtifactGraphWriter;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;

/// Materialize artifact nodes for one tenant. Returns the number of nodes
/// written (0 when the catalog is not Postgres).
///
/// Uses the same `as_ref_any().downcast_ref::<PostgresCatalog>()` accessor that
/// the artifact-retention worker (`kyma_compaction::ArtifactRetentionWorker`)
/// relies on — the artifacts catalog is Postgres-only, so this is a safe no-op
/// under `kyma local` (sqlite).
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
    writer.sync(&records).await
}

/// Materialize artifact nodes for EVERY tenant that has live artifacts. No-op
/// when the catalog is not Postgres (local mode has no artifacts table).
/// Best-effort per tenant: a failure for one tenant is logged and the rest
/// proceed. Returns the total nodes written.
pub async fn sync_artifact_nodes_all_tenants(
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
) -> anyhow::Result<usize> {
    let Some(pg) = catalog.as_ref_any().downcast_ref::<PostgresCatalog>() else {
        return Ok(0);
    };
    let tenants = pg.list_artifact_tenants().await?;
    let mut total = 0;
    for tenant in tenants {
        match sync_artifact_nodes(catalog.clone(), format.clone(), tenant).await {
            Ok(n) => total += n,
            Err(e) => tracing::warn!(error = %e, ?tenant, "artifact-graph sync failed for tenant"),
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use kyma_format_tlm::TelemetryFormat;
    use object_store::memory::InMemory;

    /// The non-Postgres arm is a safe no-op: an in-memory sqlite catalog has no
    /// `artifacts` table, so the driver returns `Ok(0)` without touching the
    /// writer. (The Postgres arm is covered by the artifact-graph e2e.)
    #[tokio::test]
    async fn non_postgres_catalog_is_noop() {
        let catalog: Arc<dyn Catalog> =
            Arc::new(kyma_catalog_sqlite::SqliteCatalog::connect_in_memory().await.unwrap());
        let store = Arc::new(InMemory::new());
        let format: Arc<dyn SegmentFormat> =
            Arc::new(TelemetryFormat::new(store, "kyma-test"));
        let n = sync_artifact_nodes(catalog, format, kyma_core::tenant::DEFAULT_TENANT)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    /// `sync_artifact_nodes_all_tenants` over a non-Postgres catalog returns
    /// `Ok(0)` — it short-circuits at the downcast, never reaching the tenant
    /// list query.
    #[tokio::test]
    async fn all_tenants_non_postgres_is_noop() {
        let catalog: Arc<dyn Catalog> =
            Arc::new(kyma_catalog_sqlite::SqliteCatalog::connect_in_memory().await.unwrap());
        let store = Arc::new(InMemory::new());
        let format: Arc<dyn SegmentFormat> =
            Arc::new(TelemetryFormat::new(store, "kyma-test"));
        let n = sync_artifact_nodes_all_tenants(catalog, format)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}
