//! Parquet extent reader: open the object, expose row groups as blocks.

use arrow_array::{RecordBatch, RecordBatchReader};
use bytes::Bytes;
use pensieve_core::errors::{FormatError, Result, StorageError};
use pensieve_core::segment_format::{
    BlockId, BlockPredicate, ColumnId, ExtentMetadata, ExtentReader, OpenExtentInput,
};
use object_store::path::Path;
use object_store::ObjectStore;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::sync::Arc;

use crate::CURRENT_VERSION;

pub struct ParquetExtentReader {
    /// The whole Parquet object, kept in memory (phase-A). `Bytes` is cheap to
    /// clone (refcounted) so each `read_block` re-builds a reader over it.
    bytes: Bytes,
    meta: ExtentMetadata,
}

impl ParquetExtentReader {
    pub(crate) async fn open(store: Arc<dyn ObjectStore>, input: OpenExtentInput) -> Result<Self> {
        let bytes = store
            .get(&Path::from(input.object_path.as_str()))
            .await
            .map_err(|e| StorageError::ObjectStore(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| StorageError::ObjectStore(e.to_string()))?;

        let builder = ParquetRecordBatchReaderBuilder::try_new(bytes.clone()).map_err(|e| {
            FormatError::Corrupt {
                path: input.object_path.clone(),
                detail: format!("parquet open: {e}"),
            }
        })?;
        let pq_meta = builder.metadata();
        let block_count = pq_meta.num_row_groups() as u32;
        let row_count = pq_meta.file_metadata().num_rows().max(0) as u64;
        let schema = builder.schema().clone();

        let meta = ExtentMetadata {
            row_count,
            block_count,
            byte_size: bytes.len() as u64,
            schema,
            format_version: CURRENT_VERSION,
        };
        Ok(Self { bytes, meta })
    }
}

#[async_trait::async_trait]
impl ExtentReader for ParquetExtentReader {
    fn metadata(&self) -> &ExtentMetadata {
        &self.meta
    }

    async fn pruned_blocks(&self, _predicate: &BlockPredicate) -> Result<Vec<BlockId>> {
        // Conservative: every row group survives. Extent-level pruning already
        // happens via `column_stats`; native row-group / bloom-filter pruning is
        // a follow-up (S2.1b) — returning all blocks is always correct.
        Ok((0..self.meta.block_count).map(BlockId).collect())
    }

    async fn read_block(&self, block: BlockId, projection: &[ColumnId]) -> Result<RecordBatch> {
        let builder =
            ParquetRecordBatchReaderBuilder::try_new(self.bytes.clone()).map_err(|e| {
                FormatError::Corrupt {
                    path: "<parquet block>".into(),
                    detail: format!("reopen: {e}"),
                }
            })?;
        let rg = block.0 as usize;
        if rg >= builder.metadata().num_row_groups() {
            return Err(FormatError::Corrupt {
                path: "<parquet block>".into(),
                detail: format!("row group {rg} out of range"),
            }
            .into());
        }
        let rg_rows = builder.metadata().row_group(rg).num_rows().max(0) as usize;

        let mask = if projection.is_empty() {
            ProjectionMask::all()
        } else {
            let descr = builder.metadata().file_metadata().schema_descr();
            ProjectionMask::roots(descr, projection.iter().map(|c| c.0 as usize))
        };

        let reader = builder
            .with_row_groups(vec![rg])
            .with_projection(mask)
            // One batch covers the whole row group so the block is a single
            // RecordBatch (matches the TLM block contract).
            .with_batch_size(rg_rows.max(1))
            .build()
            .map_err(|e| FormatError::Corrupt {
                path: "<parquet block>".into(),
                detail: format!("build reader: {e}"),
            })?;

        let schema = reader.schema();
        let mut batches: Vec<RecordBatch> = Vec::new();
        for b in reader {
            let b = b.map_err(|e| FormatError::Corrupt {
                path: "<parquet block>".into(),
                detail: format!("decode batch: {e}"),
            })?;
            batches.push(b);
        }
        if batches.is_empty() {
            return Ok(RecordBatch::new_empty(schema));
        }
        if batches.len() == 1 {
            return Ok(batches.pop().unwrap());
        }
        arrow::compute::concat_batches(&schema, &batches).map_err(|e| {
            FormatError::Corrupt {
                path: "<parquet block>".into(),
                detail: format!("concat batches: {e}"),
            }
            .into()
        })
    }
}
