//! Phase-A extent reader.
//!
//! Handles both v1 (Arrow IPC only) and v2 (Arrow IPC + per-block stats
//! footer) layouts. Block-level pruning kicks in for v2 extents; v1
//! extents degrade to "return all blocks" (same as before).

use crate::block_stats::{BlockStats, ColStat};
use crate::{MAGIC, MAGIC_V2};
use arrow::ipc::reader::FileReader;
use arrow_array::RecordBatch;
use async_trait::async_trait;
use kyma_core::errors::{FormatError, Result, StorageError};
use kyma_core::segment_format::{
    BlockId, BlockPredicate, ColumnId, ExtentMetadata, ExtentReader, OpenExtentInput, ScalarValue,
};
use object_store::path::Path;
use object_store::ObjectStore;
use std::io::Cursor;
use std::sync::Arc;

pub struct TelemetryExtentReader {
    metadata: ExtentMetadata,
    batches: Vec<RecordBatch>,
    object_path: String,
    /// Per-block stats parsed from the v2 footer. `None` for v1 extents
    /// (pruning falls back to "return all blocks").
    block_stats: Option<Vec<BlockStats>>,
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

        if bytes.len() < MAGIC.len() {
            return Err(FormatError::InvalidMagic.into());
        }
        let head = &bytes[..MAGIC.len()];

        let (ipc_body, block_stats, format_version) = if head == MAGIC_V2 {
            if bytes.len() < MAGIC_V2.len() + 4 + MAGIC_V2.len() {
                return Err(FormatError::Corrupt {
                    path: input.object_path.clone(),
                    detail: "v2 extent too short for footer".into(),
                }
                .into());
            }
            let end = bytes.len();
            let tail_magic = &bytes[end - MAGIC_V2.len()..];
            if tail_magic != MAGIC_V2 {
                return Err(FormatError::Corrupt {
                    path: input.object_path.clone(),
                    detail: "v2 trailing magic mismatch".into(),
                }
                .into());
            }
            let stats_len_off = end - MAGIC_V2.len() - 4;
            let stats_len = u32::from_le_bytes(
                bytes[stats_len_off..stats_len_off + 4]
                    .try_into()
                    .map_err(|_| FormatError::Corrupt {
                        path: input.object_path.clone(),
                        detail: "bad stats len".into(),
                    })?,
            ) as usize;
            let stats_off = stats_len_off
                .checked_sub(stats_len)
                .ok_or_else(|| FormatError::Corrupt {
                    path: input.object_path.clone(),
                    detail: "stats_len underflow".into(),
                })?;
            let stats_bytes = &bytes[stats_off..stats_off + stats_len];
            let stats: Vec<BlockStats> = serde_json::from_slice(stats_bytes)
                .map_err(|e| FormatError::Corrupt {
                    path: input.object_path.clone(),
                    detail: format!("block-stats decode: {e}"),
                })?;
            let ipc_body = bytes.slice(MAGIC_V2.len()..stats_off);
            (ipc_body, Some(stats), 2u32)
        } else if head == MAGIC {
            let ipc_body = bytes.slice(MAGIC.len()..);
            (ipc_body, None, 1u32)
        } else {
            return Err(FormatError::InvalidMagic.into());
        };

        let cursor = Cursor::new(ipc_body.to_vec());
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
                format_version,
            },
            batches,
            object_path: input.object_path,
            block_stats,
        })
    }
}

#[async_trait]
impl ExtentReader for TelemetryExtentReader {
    fn metadata(&self) -> &ExtentMetadata {
        &self.metadata
    }

    async fn pruned_blocks(&self, predicate: &BlockPredicate) -> Result<Vec<BlockId>> {
        let total = self.metadata.block_count;
        let Some(stats) = self.block_stats.as_ref() else {
            return Ok((0..total).map(BlockId).collect());
        };
        let names: Vec<&str> = self
            .metadata
            .schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        let mut kept = Vec::with_capacity(total as usize);
        for (i, bs) in stats.iter().enumerate() {
            if evaluate(predicate, bs, &names) {
                kept.push(BlockId(i as u32));
            }
        }
        Ok(kept)
    }

    async fn read_block(
        &self,
        block: BlockId,
        projection: &[ColumnId],
    ) -> Result<RecordBatch> {
        let idx = block.0 as usize;
        let batch = self.batches.get(idx).ok_or_else(|| FormatError::Corrupt {
            path: self.object_path.clone(),
            detail: format!(
                "block id {} out of range (block_count={})",
                block.0, self.metadata.block_count
            ),
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

/// Evaluate a predicate against a block's stats. Returns `true` if the
/// block MAY contain matching rows — conservative: when stats are missing
/// or the predicate isn't expressible at this level, we include.
fn evaluate(pred: &BlockPredicate, bs: &BlockStats, names: &[&str]) -> bool {
    match pred {
        BlockPredicate::All => true,
        BlockPredicate::And(subs) => subs.iter().all(|s| evaluate(s, bs, names)),
        BlockPredicate::Or(subs) => {
            if subs.is_empty() {
                true
            } else {
                subs.iter().any(|s| evaluate(s, bs, names))
            }
        }
        BlockPredicate::Not(_) => true,
        BlockPredicate::TimeRange {
            col,
            start_inclusive,
            end_exclusive,
        } => {
            let Some(name) = names.get(col.0 as usize) else {
                return true;
            };
            let Some(ColStat::Ts { min, max, .. }) = bs.cols.get(*name) else {
                return true;
            };
            !(*max < *start_inclusive || *min >= *end_exclusive)
        }
        BlockPredicate::Equals { col, value } => {
            let Some(name) = names.get(col.0 as usize) else {
                return true;
            };
            let Some(cs) = bs.cols.get(*name) else {
                return true;
            };
            equals_ok(cs, value)
        }
        BlockPredicate::InSet { col, values } => {
            let Some(name) = names.get(col.0 as usize) else {
                return true;
            };
            let Some(cs) = bs.cols.get(*name) else {
                return true;
            };
            values.iter().any(|v| equals_ok(cs, v))
        }
        BlockPredicate::StringHas { .. }
        | BlockPredicate::DynamicPathExists { .. }
        | BlockPredicate::DynamicPathEquals { .. } => true,
    }
}

fn equals_ok(cs: &ColStat, v: &ScalarValue) -> bool {
    match (cs, v) {
        (ColStat::Ts { min, max, .. }, ScalarValue::Timestamp(t)) => t >= min && t <= max,
        (ColStat::Int64 { min, max, .. }, ScalarValue::Int64(t)) => t >= min && t <= max,
        (ColStat::Str { min, max, .. }, ScalarValue::String(t)) => {
            t.as_str() >= min.as_str() && t.as_str() <= max.as_str()
        }
        _ => true,
    }
}
