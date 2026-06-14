//! [`SidecarBuilder`] implementation for the IVF + 1-bit RaBitQ ANN index.
//!
//! The `index_build` job executor (`kyma-jobs::index_build`) drives this: it
//! streams the extent's blocks (full schema, in block order) to [`build`]. We
//! project the `column` (a `FixedSizeList<Float32>` embedding column),
//! L2-normalize each row exactly as the extent writer does
//! (`kyma-format-tlm`'s `update_vector_stats`), derive each row's
//! [`RowAddress`] from the batch ordinal + in-batch row index, and hand the
//! `(addr, vec)` pairs to [`crate::build::build_ivf_rabitq`].
//!
//! ## RowAddress.block convention
//!
//! Batches arrive in block order (`BatchStream` is documented as block-ordered;
//! the executor lists `pruned_blocks` which returns `BlockId(i)` for the i-th
//! appended batch, then reads them in that order). The TLM reader assigns
//! `BlockId(i)` to the i-th `RecordBatch` it decodes from the Arrow IPC stream,
//! which is exactly append order. So **`RowAddress.block` = the 0-based ordinal
//! of the batch within the stream**, and `row` = the row's index within that
//! batch. Part B resolves a hit back to extent data via
//! `ExtentReader::read_block(BlockId(block))` then row `row`.
//!
//! ## embedding_model_id
//!
//! [`ExtentManifest`] carries no per-column embedding-model metadata today
//! (only `column_stats`: min/max/null/distinct). There is therefore no build
//! context from which to recover the model id, so we report `None`. When the
//! manifest gains a column-metadata channel (or the job payload carries the
//! model), thread it through here.

use crate::build::build_ivf_rabitq;
use arrow_array::cast::AsArray;
use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch};
use arrow_schema::DataType;
use async_trait::async_trait;
use futures::StreamExt;
use kyma_core::catalog::ExtentManifest;
use kyma_core::errors::{Error, Result};
use kyma_core::index_sidecar::{
    BatchStream, BuiltSidecar, RowAddress, SidecarBuilder, SidecarKind,
};
use kyma_core::segment_format::BlockId;

/// Builds [`SidecarKind::IvfRabitq`] sidecars.
#[derive(Debug, Default, Clone)]
pub struct IvfRabitqBuilder;

impl IvfRabitqBuilder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SidecarBuilder for IvfRabitqBuilder {
    fn kind(&self) -> SidecarKind {
        SidecarKind::IvfRabitq
    }

    async fn build(
        &self,
        _extent: &ExtentManifest,
        column: &str,
        mut batches: BatchStream<'_>,
    ) -> Result<BuiltSidecar> {
        let mut rows: Vec<(RowAddress, Vec<f32>)> = Vec::new();
        let mut dim: Option<usize> = None;
        let mut block_ordinal: u32 = 0;

        while let Some(batch) = batches.next().await {
            let batch = batch?;
            let d = extract_rows(&batch, column, block_ordinal, &mut rows)?;
            if let Some(d) = d {
                match dim {
                    Some(prev) if prev != d => {
                        return Err(Error::Config(format!(
                            "vector column '{column}' has inconsistent dim: {prev} then {d}"
                        )));
                    }
                    _ => dim = Some(d),
                }
            }
            block_ordinal += 1;
        }

        let dim = dim.ok_or_else(|| {
            Error::Config(format!(
                "column '{column}' not found or not a FixedSizeList<Float32> vector column"
            ))
        })?;

        if rows.is_empty() {
            return Err(Error::Config(format!(
                "vector column '{column}' yielded no non-null rows to index"
            )));
        }

        let (bytes, params) = build_ivf_rabitq(&rows, dim, None)
            .map_err(|e| Error::Internal(format!("ivf_rabitq build failed: {e}")))?;

        Ok(BuiltSidecar {
            bytes,
            params,
            // No per-column model metadata available from the build context.
            embedding_model_id: None,
        })
    }
}

/// Project the `column` from `batch`, L2-normalize each non-null row, and push
/// `(RowAddress{block, row}, vec)` pairs into `out`. Returns the column's `dim`
/// if the column is a present `FixedSizeList<Float32>`, else `Ok(None)` when the
/// column is absent (so the caller can error after the whole stream is empty),
/// or an error when the column exists but is the wrong type.
///
/// Normalization matches `kyma-format-tlm`'s writer exactly: accumulate the
/// squared-norm in f64, skip rows whose norm ≤ 1e-12 (zero vectors can't be
/// placed on the unit sphere), and divide each coordinate by the norm.
fn extract_rows(
    batch: &RecordBatch,
    column: &str,
    block_ordinal: u32,
    out: &mut Vec<(RowAddress, Vec<f32>)>,
) -> Result<Option<usize>> {
    let Some(col_idx) = batch.schema().index_of(column).ok() else {
        return Ok(None);
    };
    let field = batch.schema().field(col_idx).clone();
    let dim = match field.data_type() {
        DataType::FixedSizeList(inner, dim) if matches!(inner.data_type(), DataType::Float32) => {
            *dim as usize
        }
        other => {
            return Err(Error::Config(format!(
                "column '{column}' is {other:?}, not FixedSizeList<Float32> — cannot build ANN index"
            )));
        }
    };

    let col = batch.column(col_idx);
    let arr: &FixedSizeListArray = col.as_fixed_size_list_opt().ok_or_else(|| {
        Error::Internal(format!("column '{column}' failed FixedSizeList downcast"))
    })?;
    let floats: &Float32Array = arr
        .values()
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| Error::Internal(format!("column '{column}' values not Float32")))?;

    for i in 0..arr.len() {
        if arr.is_null(i) {
            continue;
        }
        let start = i * dim;
        let mut v: Vec<f32> = (0..dim).map(|j| floats.value(start + j)).collect();
        // L2-normalize in f64 (identical to the writer's accumulation path).
        let norm = v
            .iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>()
            .sqrt();
        if norm <= 1e-12 {
            continue; // zero vector — can't normalize; skip (keeps unit-sphere invariant)
        }
        for x in v.iter_mut() {
            *x = (*x as f64 / norm) as f32;
        }
        out.push((
            RowAddress {
                block: BlockId(block_ordinal),
                row: i as u32,
            },
            v,
        ));
    }

    Ok(Some(dim))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::builder::{FixedSizeListBuilder, Float32Builder};
    use arrow_array::{Int64Array, RecordBatch};
    use arrow_schema::{Field, Schema};
    use futures::stream;
    use kyma_core::catalog::ExtentManifest;
    use kyma_core::types::{ExtentId, SchemaSnapshotId, TableId};
    use std::sync::Arc;

    fn vec_batch(vectors: &[Vec<f32>], dim: usize) -> RecordBatch {
        let values = Float32Builder::new();
        let mut b = FixedSizeListBuilder::new(values, dim as i32);
        for v in vectors {
            b.values().append_slice(v);
            b.append(true);
        }
        let arr = b.finish();
        let field = Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            true,
        );
        let schema = Arc::new(Schema::new(vec![field]));
        RecordBatch::try_new(schema, vec![Arc::new(arr)]).unwrap()
    }

    fn dummy_manifest() -> ExtentManifest {
        ExtentManifest {
            id: ExtentId::new(),
            table_id: TableId::new(),
            schema_snapshot_id: SchemaSnapshotId::new(),
            object_path: "t/extents/x.kyma".into(),
            byte_size: 0,
            row_count: 0,
            min_timestamp: None,
            max_timestamp: None,
            column_stats: serde_json::json!({}),
            present_paths: vec![],
            compaction_gen: 0,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn builds_over_two_blocks_with_correct_addresses() {
        let dim = 8;
        // Two batches; block ordinals should be 0 then 1.
        let b0 = vec_batch(&[vec![1.0; dim], vec![2.0; dim]], dim);
        let b1 = vec_batch(&[vec![3.0; dim]], dim);
        let s = stream::iter(vec![Ok(b0), Ok(b1)]).boxed();

        let builder = IvfRabitqBuilder::new();
        let built = builder
            .build(&dummy_manifest(), "embedding", s)
            .await
            .unwrap();
        assert_eq!(built.params["dim"], dim);
        assert_eq!(built.embedding_model_id, None);

        // Confirm we can read it back and addresses span both blocks.
        let h = crate::file::read_header(&built.bytes).unwrap();
        let mut all = Vec::new();
        for p in 0..h.nlist as usize {
            all.extend(crate::file::read_partition(&built.bytes, &h, p).unwrap());
        }
        let blocks: std::collections::HashSet<u32> = all.iter().map(|e| e.addr.block.0).collect();
        assert!(blocks.contains(&0));
        assert!(blocks.contains(&1));
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn skips_zero_vectors() {
        let dim = 8;
        let b0 = vec_batch(&[vec![0.0; dim], vec![1.0; dim]], dim);
        let s = stream::iter(vec![Ok(b0)]).boxed();
        let builder = IvfRabitqBuilder::new();
        let built = builder
            .build(&dummy_manifest(), "embedding", s)
            .await
            .unwrap();
        let h = crate::file::read_header(&built.bytes).unwrap();
        let mut all = Vec::new();
        for p in 0..h.nlist as usize {
            all.extend(crate::file::read_partition(&built.bytes, &h, p).unwrap());
        }
        // Only the non-zero row survives.
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].addr.row, 1);
    }

    #[tokio::test]
    async fn missing_column_errors() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let s = stream::iter(vec![Ok(batch)]).boxed();
        let builder = IvfRabitqBuilder::new();
        let r = builder.build(&dummy_manifest(), "embedding", s).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn wrong_type_column_errors() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "embedding",
                DataType::Int64,
                false,
            )])),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        let s = stream::iter(vec![Ok(batch)]).boxed();
        let builder = IvfRabitqBuilder::new();
        let r = builder.build(&dummy_manifest(), "embedding", s).await;
        assert!(r.is_err());
    }
}
