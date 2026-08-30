//! [`TantivyFtsBuilder`] — the [`SidecarBuilder`] that produces `tantivy_fts`
//! sidecars for the async index-build job.

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch};
use arrow_schema::DataType;
use async_trait::async_trait;
use futures::StreamExt;

use kyma_core::catalog::ExtentManifest;
use kyma_core::errors::{Error, Result};
use kyma_core::index_sidecar::{
    BatchStream, BuiltSidecar, RowAddress, SidecarBuilder, SidecarKind,
};
use kyma_core::segment_format::BlockId;

use crate::build::build_fts;

/// Builds a per-extent tantivy BM25 index over one Utf8 text column.
#[derive(Debug, Default, Clone)]
pub struct TantivyFtsBuilder;

impl TantivyFtsBuilder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SidecarBuilder for TantivyFtsBuilder {
    fn kind(&self) -> SidecarKind {
        SidecarKind::TantivyFts
    }

    async fn build(
        &self,
        _extent: &ExtentManifest,
        column: &str,
        mut batches: BatchStream<'_>,
    ) -> Result<BuiltSidecar> {
        let mut rows: Vec<(RowAddress, String)> = Vec::new();
        let mut block_ordinal: u32 = 0;
        let mut saw_column = false;

        while let Some(batch) = batches.next().await {
            let batch = batch?;
            saw_column |= extract_text_rows(&batch, column, block_ordinal, &mut rows)?;
            block_ordinal += 1;
        }

        if !saw_column {
            return Err(Error::Config(format!(
                "column '{column}' not found or not a Utf8 text column"
            )));
        }

        let (bytes, params) =
            build_fts(rows).map_err(|e| Error::Config(format!("fts build: {e}")))?;
        Ok(BuiltSidecar {
            bytes,
            params,
            embedding_model_id: None, // FTS is model-free.
        })
    }
}

/// Append `(RowAddress, text)` for every non-null row of the Utf8 `column` in
/// `batch`. Returns whether the column was present and of Utf8 type. Null cells
/// are skipped (an absent body indexes nothing for that row).
fn extract_text_rows(
    batch: &RecordBatch,
    column: &str,
    block: u32,
    out: &mut Vec<(RowAddress, String)>,
) -> Result<bool> {
    let Some(col) = batch.column_by_name(column) else {
        return Ok(false);
    };
    if !matches!(col.data_type(), DataType::Utf8) {
        return Ok(false);
    }
    let arr = col.as_string::<i32>();
    for row in 0..arr.len() {
        if arr.is_null(row) {
            continue;
        }
        out.push((
            RowAddress {
                block: BlockId(block),
                row: row as u32,
            },
            arr.value(row).to_string(),
        ));
    }
    Ok(true)
}
