//! Connector artifact capability.
//!
//! Lets a connector's `run_once` persist a full-file blob (e.g. a redacted CI
//! job log) to the object store AND register its catalog tracking row in one
//! call, without the connector touching the catalog directly. Handed to the
//! connector via [`crate::types::ConnectorCtx::artifacts`].
//!
//! Bytes are stored verbatim — the caller redacts first (see kyma-redact).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_catalog::PostgresCatalog;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use uuid::Uuid;

/// Capability for writing + tracking object-store artifacts from a connector.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Register the catalog tracking row for `record`, then write `bytes` to the
    /// object store at `record.object_path` (overwriting). Returns the artifact
    /// row id.
    ///
    /// Row-before-blob ordering is deliberate: if the blob write fails, the
    /// tracking row simply points at a missing object — retrieval returns
    /// `None` and the next connector tick re-writes the blob at the same
    /// deterministic key. The reverse order would leave an untracked orphan
    /// blob that no retention sweep would ever reclaim.
    async fn put_and_register(&self, record: ArtifactRecord, bytes: Bytes) -> anyhow::Result<Uuid>;
}

/// Object-store backed [`ArtifactStore`] (the production implementation).
pub struct ObjectArtifactStore {
    pub store: Arc<dyn ObjectStore>,
    pub catalog: Arc<PostgresCatalog>,
}

impl ObjectArtifactStore {
    pub fn new(store: Arc<dyn ObjectStore>, catalog: Arc<PostgresCatalog>) -> Self {
        Self { store, catalog }
    }
}

#[async_trait]
impl ArtifactStore for ObjectArtifactStore {
    async fn put_and_register(&self, record: ArtifactRecord, bytes: Bytes) -> anyhow::Result<Uuid> {
        let key = ObjPath::from(record.object_path.as_str());
        let id = self
            .catalog
            .register_artifact(&record)
            .await
            .map_err(|e| anyhow::anyhow!("artifact register: {e}"))?;
        kyma_storage::put_artifact(&self.store, &key, bytes)
            .await
            .map_err(|e| anyhow::anyhow!("artifact put: {e}"))?;
        Ok(id)
    }
}
