//! Phase-A extent writer.
//!
//! Buffers appended `RecordBatch`es in memory, emits Arrow IPC file bytes
//! on `finish()`, prepends the magic, and uploads a single object. The
//! per-extent `column_stats` contract (distinct/tokens/vec + timestamp bounds)
//! is computed by the shared [`crate::column_stats::ColumnStatsAccumulator`] so
//! every segment format emits byte-identical stats.

use crate::block_stats::{stats_for_batch, BlockStats};
use crate::column_stats::ColumnStatsAccumulator;
use crate::{TelemetryFormat, MAGIC_V2};
use arrow::ipc::writer::FileWriter;
use arrow_array::RecordBatch;
use arrow_schema::Schema;
use async_trait::async_trait;
use pensieve_core::errors::{FormatError, Result};
use pensieve_core::segment_format::{ExtentWriteResult, ExtentWriter};
use pensieve_core::types::{ExtentId, SchemaRef};
use object_store::path::Path;
use object_store::PutPayload;
use std::sync::Arc;

pub struct TelemetryExtentWriter {
    format: TelemetryFormat,
    extent_id: ExtentId,
    schema: SchemaRef,
    // Arrow IPC FileWriter over a Vec<u8> — phase A buffers in-memory.
    ipc: FileWriter<Vec<u8>>,
    row_count: u64,
    block_count: u32,
    /// Shared per-extent column-stats contract (distinct/tokens/vec + ts bounds).
    stats: ColumnStatsAccumulator,
    /// Per-block min/max stats, one entry per appended batch. Emitted as a
    /// JSON footer after the Arrow IPC body (v2 format).
    block_stats: Vec<BlockStats>,
}

impl TelemetryExtentWriter {
    pub(crate) fn new(format: TelemetryFormat, schema: SchemaRef) -> Self {
        // Reserve some capacity up front — ~16 MiB is a reasonable guess for
        // phase-A extents we generate from tests.
        let buffer: Vec<u8> = Vec::with_capacity(16 * 1024 * 1024);
        let ipc =
            FileWriter::try_new(buffer, &schema).expect("Arrow IPC FileWriter construction failed");
        let stats = ColumnStatsAccumulator::new(&schema);
        Self {
            format,
            extent_id: ExtentId::new(),
            schema,
            ipc,
            row_count: 0,
            block_count: 0,
            stats,
            block_stats: Vec::new(),
        }
    }
}

#[async_trait]
impl ExtentWriter for TelemetryExtentWriter {
    async fn append(&mut self, batch: RecordBatch) -> Result<()> {
        if batch.schema() != self.schema {
            return Err(FormatError::TypeMismatch {
                expected: format_schema(&self.schema),
                got: format_schema(&batch.schema()),
            }
            .into());
        }
        self.row_count += batch.num_rows() as u64;
        self.block_count += 1;
        self.stats.update(&batch);
        self.block_stats.push(stats_for_batch(&batch));
        self.ipc.write(&batch).map_err(|e| FormatError::Corrupt {
            path: "<in-memory ipc>".to_string(),
            detail: format!("FileWriter::write: {e}"),
        })?;
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<ExtentWriteResult> {
        // Capture stats *before* destructuring — `finish()` borrows the accumulator.
        let column_stats = self.stats.finish();
        let min_ts_nanos = self.stats.min_ts_nanos();
        let max_ts_nanos = self.stats.max_ts_nanos();
        let TelemetryExtentWriter {
            format,
            extent_id,
            row_count,
            block_count,
            mut ipc,
            block_stats,
            ..
        } = *self;

        ipc.finish().map_err(|e| FormatError::Corrupt {
            path: "<in-memory ipc>".to_string(),
            detail: format!("FileWriter::finish: {e}"),
        })?;

        let ipc_bytes = ipc.into_inner().map_err(|e| FormatError::Corrupt {
            path: "<in-memory ipc>".to_string(),
            detail: format!("FileWriter::into_inner: {e}"),
        })?;

        // v2 frame:
        //   MAGIC_V2 || ipc_bytes || block_stats_json || stats_len u32 LE || MAGIC_V2
        let stats_json = serde_json::to_vec(&block_stats).map_err(|e| FormatError::Corrupt {
            path: "<in-memory ipc>".to_string(),
            detail: format!("block-stats serialize: {e}"),
        })?;
        let stats_len = stats_json.len() as u32;

        let mut payload: Vec<u8> = Vec::with_capacity(
            MAGIC_V2.len() + ipc_bytes.len() + stats_json.len() + 4 + MAGIC_V2.len(),
        );
        payload.extend_from_slice(MAGIC_V2);
        payload.extend_from_slice(&ipc_bytes);
        payload.extend_from_slice(&stats_json);
        payload.extend_from_slice(&stats_len.to_le_bytes());
        payload.extend_from_slice(MAGIC_V2);
        let byte_size = payload.len() as u64;

        let object_path =
            format_extent_path(format.path_prefix(), format.tenant_segment(), &extent_id);
        let store: Arc<dyn object_store::ObjectStore> = format.store().clone();

        store
            .put(&Path::from(object_path.as_str()), PutPayload::from(payload))
            .await
            .map_err(|e| pensieve_core::errors::StorageError::ObjectStore(e.to_string()))?;

        Ok(ExtentWriteResult {
            extent_id,
            object_path,
            byte_size,
            row_count,
            block_count,
            min_timestamp_nanos: min_ts_nanos,
            max_timestamp_nanos: max_ts_nanos,
            // Phase A does not expose `dynamic`-column path tracking yet.
            present_paths: Vec::new(),
            column_stats,
        })
    }
}

fn format_extent_path(prefix: &str, tenant_segment: &str, extent_id: &ExtentId) -> String {
    let core = if tenant_segment.is_empty() {
        format!("extents/{extent_id}.pensieve")
    } else {
        format!("{tenant_segment}/extents/{extent_id}.pensieve")
    };
    if prefix.is_empty() {
        core
    } else {
        format!("{prefix}/{core}")
    }
}

#[cfg(test)]
mod path_tests {
    use super::format_extent_path;
    use pensieve_core::types::ExtentId;
    use uuid::Uuid;

    #[test]
    fn legacy_path_when_tenant_empty() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let path = format_extent_path("pensieve", "", &id);
        assert_eq!(
            path,
            "pensieve/extents/00000000-0000-0000-0000-000000000000.pensieve"
        );
    }

    #[test]
    fn tenant_segmented_path() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let tenant = "11111111-1111-1111-1111-111111111111";
        let path = format_extent_path("pensieve", tenant, &id);
        assert_eq!(
            path,
            "pensieve/11111111-1111-1111-1111-111111111111/extents/00000000-0000-0000-0000-000000000000.pensieve"
        );
    }
}

fn format_schema(schema: &Schema) -> String {
    let fields: Vec<String> = schema
        .fields()
        .iter()
        .map(|f| format!("{}: {:?}", f.name(), f.data_type()))
        .collect();
    format!("[{}]", fields.join(", "))
}
