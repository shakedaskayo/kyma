//! The telemetry storage format — phase A.
//!
//! # Phase A (this code) — Arrow IPC wrapper
//!
//! For the E2E vertical slice, an extent is literally
//!
//! ```text
//! [MAGIC "KYMA\x01"] [arrow IPC file bytes]
//! ```
//!
//! We leverage Apache Arrow's own IPC "file" format for the body — it has
//! a schema header, stream of record batches, and footer with batch
//! offsets. Each arrow record batch is one "block" in our `SegmentFormat`
//! abstraction.
//!
//! This phase is intentionally not the performance story — it exists so
//! every other layer (catalog, ingest, exec, server) can be wired
//! end-to-end while we evolve the real format in M3.
//!
//! # Phase B (later) — custom format
//!
//! The real format will replace the Arrow IPC body with custom per-type
//! column encoders (delta-of-delta timestamps, Gorilla floats,
//! inverted-index string columns, CBOR dynamic columns, …), but will
//! preserve the same `MAGIC + body + magic footer` shape and the same
//! `SegmentFormat` trait contract. See plan §4.

#![forbid(unsafe_code)]

pub mod block_stats;
mod reader;
mod writer;

use async_trait::async_trait;
use kyma_core::errors::Result;
use kyma_core::segment_format::{ExtentReader, ExtentWriter, OpenExtentInput, SegmentFormat};
use kyma_core::types::SchemaRef;
use object_store::ObjectStore;
use std::sync::Arc;

pub use reader::TelemetryExtentReader;
pub use writer::TelemetryExtentWriter;

/// Magic bytes written at the start of every v1 telemetry-format extent.
/// Kept for backward-compat: v1 extents can still be read.
pub const MAGIC: &[u8] = b"KYMA\x01";

/// Magic bytes for v2 extents. Layout:
/// ```text
/// MAGIC_V2 || arrow_ipc_bytes || block_stats_json || stats_len u32 LE || MAGIC_V2
/// ```
/// The trailing MAGIC_V2 + u32 lets readers walk backward from the end
/// without mutating the Arrow IPC body.
pub const MAGIC_V2: &[u8] = b"KYMA\x02";

/// Current format version this implementation writes.
/// v1 — Arrow IPC only. v2 — adds per-block stats footer.
pub const CURRENT_VERSION: u32 = 2;

/// The telemetry segment format (phase A).
#[derive(Clone)]
pub struct TelemetryFormat {
    store: Arc<dyn ObjectStore>,
    /// Prefix prepended to every extent's object key, e.g. `"kyma"`.
    path_prefix: String,
    /// Tenant segment injected between prefix and `extents/` for cloud
    /// deployments. Empty string = self-hosted / legacy mode (no tenant
    /// segment in the path).
    tenant_segment: String,
}

impl TelemetryFormat {
    /// Build a format over the given object store + path prefix.
    pub fn new(store: Arc<dyn ObjectStore>, path_prefix: impl Into<String>) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: String::new(),
        }
    }

    /// Build a format that namespaces every new extent under
    /// `<prefix>/<tenant_id>/extents/<extent_id>.kyma`.
    pub fn with_tenant(
        store: Arc<dyn ObjectStore>,
        path_prefix: impl Into<String>,
        tenant: kyma_core::tenant::TenantId,
    ) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: tenant.to_string(),
        }
    }

    /// Borrow the underlying object store (needed by the writer to upload).
    pub(crate) fn store(&self) -> &Arc<dyn ObjectStore> {
        &self.store
    }

    /// Borrow the path prefix (e.g. "kyma") used when building extent paths.
    pub(crate) fn path_prefix(&self) -> &str {
        &self.path_prefix
    }

    /// Borrow the tenant segment (e.g. a UUID string) used when building
    /// tenant-namespaced extent paths. Empty string = legacy / self-hosted
    /// mode with no tenant segment.
    pub(crate) fn tenant_segment(&self) -> &str {
        &self.tenant_segment
    }
}

impl std::fmt::Debug for TelemetryFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryFormat")
            .field("path_prefix", &self.path_prefix)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SegmentFormat for TelemetryFormat {
    fn name(&self) -> &'static str {
        "kyma-format-tlm"
    }

    fn magic(&self) -> &'static [u8] {
        MAGIC
    }

    fn current_version(&self) -> u32 {
        CURRENT_VERSION
    }

    fn max_readable_version(&self) -> u32 {
        CURRENT_VERSION
    }

    async fn open_extent(&self, input: OpenExtentInput) -> Result<Arc<dyn ExtentReader>> {
        let r = TelemetryExtentReader::open(self.store.clone(), input).await?;
        Ok(Arc::new(r))
    }

    async fn start_extent(
        &self,
        schema: SchemaRef,
        _target_bytes: u64,
    ) -> Result<Box<dyn ExtentWriter>> {
        let w = TelemetryExtentWriter::new(self.clone(), schema);
        Ok(Box::new(w))
    }

    fn object_store(&self) -> Option<Arc<dyn ObjectStore>> {
        Some(self.store.clone())
    }
}
