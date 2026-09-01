//! Parquet `SegmentFormat` — ZSTD columnar extents at rest (S2.1).
//!
//! A drop-in [`SegmentFormat`](pensieve_core::segment_format::SegmentFormat) that
//! writes each extent as a single Parquet object (ZSTD-compressed, one row group
//! per appended batch) instead of the TLM Arrow-IPC frame. It emits the SAME
//! per-extent `column_stats` (via the shared
//! [`pensieve_format_tlm::column_stats::ColumnStatsAccumulator`]) so catalog pruning
//! and the index scheduler's vec-stats gate behave identically regardless of
//! format — the only difference is bytes-on-object (Parquet's columnar + ZSTD
//! encoding is far smaller than Arrow IPC).
//!
//! Per-extent format dispatch: a Parquet object begins with the standard
//! `PAR1` magic, so a [`FormatRegistry`] can pick the right reader by sniffing
//! the first bytes — old TLM (`PENSIEVE…`) extents stay readable forever.
//!
//! Block model: 1 appended `RecordBatch` → 1 Parquet row group → 1 [`BlockId`].
//! `read_block(n)` reads row group `n`; `pruned_blocks` is conservative for now
//! (returns all row groups — correct; native row-group/bloom pruning is a
//! follow-up). Extent-level pruning already happens via `column_stats`.

use std::sync::Arc;

use async_trait::async_trait;
use pensieve_core::errors::Result;
use pensieve_core::segment_format::{ExtentReader, ExtentWriter, OpenExtentInput, SegmentFormat};
use pensieve_core::types::SchemaRef;
use object_store::ObjectStore;

mod reader;
mod writer;

pub use reader::ParquetExtentReader;
pub use writer::ParquetExtentWriter;

/// Standard Parquet magic (file head + tail). Used for format sniffing.
pub const MAGIC: &[u8] = b"PAR1";
/// On-object format version this impl writes.
pub const CURRENT_VERSION: u32 = 1;

/// Parquet segment format over an object store.
#[derive(Clone)]
pub struct ParquetFormat {
    store: Arc<dyn ObjectStore>,
    path_prefix: String,
    tenant_segment: String,
}

impl ParquetFormat {
    /// Build a format over the given object store + path prefix (no tenant
    /// segment — self-hosted / legacy path layout).
    pub fn new(store: Arc<dyn ObjectStore>, path_prefix: impl Into<String>) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: String::new(),
        }
    }

    /// Namespace every new extent under `<prefix>/<tenant_id>/extents/<id>.parquet`.
    pub fn with_tenant(
        store: Arc<dyn ObjectStore>,
        path_prefix: impl Into<String>,
        tenant: pensieve_core::tenant::TenantId,
    ) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: tenant.to_string(),
        }
    }

    pub(crate) fn store(&self) -> &Arc<dyn ObjectStore> {
        &self.store
    }
    pub(crate) fn path_prefix(&self) -> &str {
        &self.path_prefix
    }
    pub(crate) fn tenant_segment(&self) -> &str {
        &self.tenant_segment
    }
}

#[async_trait]
impl SegmentFormat for ParquetFormat {
    fn name(&self) -> &'static str {
        "pensieve-format-parquet"
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
        let reader = ParquetExtentReader::open(self.store.clone(), input).await?;
        Ok(Arc::new(reader))
    }

    async fn start_extent(
        &self,
        schema: SchemaRef,
        _target_bytes: u64,
    ) -> Result<Box<dyn ExtentWriter>> {
        Ok(Box::new(ParquetExtentWriter::new(
            self.store.clone(),
            self.path_prefix.clone(),
            self.tenant_segment.clone(),
            schema,
        )))
    }

    fn object_store(&self) -> Option<Arc<dyn ObjectStore>> {
        Some(self.store.clone())
    }
}
