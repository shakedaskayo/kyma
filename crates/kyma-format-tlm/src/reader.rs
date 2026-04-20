//! Phase-A extent reader.
//!
//! Fetches the entire object via object_store, validates the magic header,
//! hands the body to Arrow IPC `FileReader`. Each Arrow record batch is one
//! block. `pruned_blocks` returns all block ids — real block-level pruning
//! (via per-block stats + indices) lands in M3 alongside the custom format.

use crate::MAGIC;
use arrow::ipc::reader::FileReader;
use arrow_array::RecordBatch;
use async_trait::async_trait;
use kyma_core::errors::{FormatError, Result, StorageError};
use kyma_core::segment_format::{
    BlockId, BlockPredicate, ColumnId, ExtentMetadata, ExtentReader, OpenExtentInput,
};
use object_store::path::Path;
use object_store::ObjectStore;
use std::io::Cursor;
use std::sync::Arc;

pub struct TelemetryExtentReader {
    metadata: ExtentMetadata,
    // All decoded batches held in memory for phase A. Later we'll lazy-load
    // per block by seeking within the IPC file.
    batches: Vec<RecordBatch>,
    object_path: String,
}

impl TelemetryExtentReader {
    pub(crate) async fn open(
        store: Arc<dyn ObjectStore>,
        input: OpenExtentInput,
    ) -> Result<Self> {
        let path = Path::from(input.object_path.clone());
        let get = store
            .get(&path)
            .await
            .map_err(|e| StorageError::ObjectStore(e.to_string()))?;
        let bytes = get
            .bytes()
            .await
            .map_err(|e| StorageError::ObjectStore(e.to_string()))?;

        if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
            return Err(FormatError::InvalidMagic.into());
        }
        let body = &bytes[MAGIC.len()..];

        let cursor = Cursor::new(body.to_vec());
        let reader = FileReader::try_new(cursor, None).map_err(|e| FormatError::Corrupt {
            path: input.object_path.clone(),
            detail: format!("FileReader::try_new: {e}"),
        })?;

        let mut batches = Vec::new();
        let mut row_count: u64 = 0;
        for res in reader {
            let batch = res.map_err(|e| FormatError::Corrupt {
                path: input.object_path.clone(),
                detail: format!("FileReader batch decode: {e}"),
            })?;
            row_count += batch.num_rows() as u64;
            batches.push(batch);
        }

        let block_count = batches.len() as u32;
        Ok(Self {
            metadata: ExtentMetadata {
                row_count,
                block_count,
                byte_size: input.byte_size,
                schema: input.schema,
                format_version: crate::CURRENT_VERSION,
            },
            batches,
            object_path: input.object_path,
        })
    }
}

#[async_trait]
impl ExtentReader for TelemetryExtentReader {
    fn metadata(&self) -> &ExtentMetadata {
        &self.metadata
    }

    async fn pruned_blocks(&self, _predicate: &BlockPredicate) -> Result<Vec<BlockId>> {
        // Phase A: no per-block stats yet — return all blocks. Predicates
        // are still applied post-decode by DataFusion, which is correct but
        // suboptimal. M3 attaches per-block min/max + inverted indices for
        // real pruning at this level.
        Ok((0..self.metadata.block_count).map(BlockId).collect())
    }

    async fn read_block(
        &self,
        block: BlockId,
        projection: &[ColumnId],
    ) -> Result<RecordBatch> {
        let idx = block.0 as usize;
        let batch = self.batches.get(idx).ok_or_else(|| FormatError::Corrupt {
            path: self.object_path.clone(),
            detail: format!("block id {} out of range (block_count={})", block.0, self.metadata.block_count),
        })?;
        if projection.is_empty() {
            return Ok(batch.clone());
        }
        let col_indices: Vec<usize> = projection.iter().map(|c| c.0 as usize).collect();
        let projected = batch.project(&col_indices).map_err(|e| FormatError::Corrupt {
            path: self.object_path.clone(),
            detail: format!("projection failed: {e}"),
        })?;
        Ok(projected)
    }
}
