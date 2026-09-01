//! Parquet extent writer: buffer appended batches, write one ZSTD Parquet
//! object (one row group per batch), compute the shared `column_stats`.

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use async_trait::async_trait;
use pensieve_core::errors::{FormatError, Result, StorageError};
use pensieve_core::segment_format::{ExtentWriteResult, ExtentWriter};
use pensieve_core::types::{ExtentId, SchemaRef};
use pensieve_format_tlm::column_stats::ColumnStatsAccumulator;
use object_store::path::Path;
use object_store::{ObjectStore, PutPayload};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

pub struct ParquetExtentWriter {
    store: Arc<dyn ObjectStore>,
    path_prefix: String,
    tenant_segment: String,
    schema: SchemaRef,
    extent_id: ExtentId,
    /// Buffered batches; written as one row group each on finish (phase-A
    /// in-memory buffering, mirroring the TLM writer).
    batches: Vec<RecordBatch>,
    row_count: u64,
    block_count: u32,
    stats: ColumnStatsAccumulator,
}

impl ParquetExtentWriter {
    pub(crate) fn new(
        store: Arc<dyn ObjectStore>,
        path_prefix: String,
        tenant_segment: String,
        schema: SchemaRef,
    ) -> Self {
        let stats = ColumnStatsAccumulator::new(&schema);
        Self {
            store,
            path_prefix,
            tenant_segment,
            schema,
            extent_id: ExtentId::new(),
            batches: Vec::new(),
            row_count: 0,
            block_count: 0,
            stats,
        }
    }
}

#[async_trait]
impl ExtentWriter for ParquetExtentWriter {
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
        self.batches.push(batch);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<ExtentWriteResult> {
        let column_stats = self.stats.finish();
        let min_timestamp_nanos = self.stats.min_ts_nanos();
        let max_timestamp_nanos = self.stats.max_ts_nanos();

        // Write all buffered batches into one Parquet object — ZSTD, one row
        // group per batch (flush forces a row-group boundary) so the reader's
        // BlockId maps 1:1 to a row group.
        let mut buffer: Vec<u8> = Vec::with_capacity(4 * 1024 * 1024);
        {
            let props = WriterProperties::builder()
                .set_compression(Compression::ZSTD(ZstdLevel::try_new(3).map_err(|e| {
                    FormatError::Corrupt {
                        path: "<parquet props>".into(),
                        detail: format!("zstd level: {e}"),
                    }
                })?))
                .build();
            let mut writer = ArrowWriter::try_new(&mut buffer, self.schema.clone(), Some(props))
                .map_err(|e| FormatError::Corrupt {
                    path: "<parquet writer>".into(),
                    detail: format!("ArrowWriter::try_new: {e}"),
                })?;
            for batch in &self.batches {
                writer.write(batch).map_err(|e| FormatError::Corrupt {
                    path: "<parquet writer>".into(),
                    detail: format!("write batch: {e}"),
                })?;
                // Close the current row group so 1 batch == 1 row group == 1 block.
                writer.flush().map_err(|e| FormatError::Corrupt {
                    path: "<parquet writer>".into(),
                    detail: format!("flush row group: {e}"),
                })?;
            }
            writer.close().map_err(|e| FormatError::Corrupt {
                path: "<parquet writer>".into(),
                detail: format!("close: {e}"),
            })?;
        }

        let byte_size = buffer.len() as u64;
        let object_path =
            format_extent_path(&self.path_prefix, &self.tenant_segment, &self.extent_id);
        self.store
            .put(&Path::from(object_path.as_str()), PutPayload::from(buffer))
            .await
            .map_err(|e| StorageError::ObjectStore(e.to_string()))?;

        Ok(ExtentWriteResult {
            extent_id: self.extent_id,
            object_path,
            byte_size,
            row_count: self.row_count,
            block_count: self.block_count,
            min_timestamp_nanos,
            max_timestamp_nanos,
            present_paths: Vec::new(),
            column_stats,
        })
    }
}

/// `<prefix>/<tenant>/extents/<id>.parquet` (tenant segment omitted when empty).
fn format_extent_path(prefix: &str, tenant_segment: &str, extent_id: &ExtentId) -> String {
    let core = if tenant_segment.is_empty() {
        format!("extents/{extent_id}.parquet")
    } else {
        format!("{tenant_segment}/extents/{extent_id}.parquet")
    };
    if prefix.is_empty() {
        core
    } else {
        format!("{prefix}/{core}")
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
