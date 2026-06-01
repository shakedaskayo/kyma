//! Phase-A extent writer.
//!
//! Buffers appended `RecordBatch`es in memory, emits Arrow IPC file bytes
//! on `finish()`, prepends the magic, and uploads a single object. Tracks
//! min/max timestamps for catalog pruning.

use crate::block_stats::{stats_for_batch, BlockStats};
use crate::{TelemetryFormat, MAGIC_V2};
use arrow::ipc::writer::FileWriter;
use arrow_array::{
    cast::AsArray, Array, FixedSizeListArray, Float32Array, Int32Array, Int64Array, RecordBatch,
    StringArray, TimestampNanosecondArray,
};
use arrow_schema::{DataType, Schema, TimeUnit};
use async_trait::async_trait;
use kyma_core::errors::{FormatError, Result};
use kyma_core::segment_format::{ExtentWriteResult, ExtentWriter};
use kyma_core::types::{ExtentId, SchemaRef};
use object_store::path::Path;
use object_store::PutPayload;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

/// Max distinct values tracked per column. Past this threshold, we give
/// up and the column's distinct set becomes `null` — query-time pruning
/// then degrades to no pruning for that column (correct, just slower).
const DISTINCT_SET_CAP: usize = 1_000;

/// Max tokens tracked per string column. Tokens are whole words (split on
/// whitespace + ASCII punctuation, lowercased). Past this threshold, we
/// fall back to `null` and text-search pruning is disabled for this
/// extent — DataFusion still applies the LIKE filter above the scan, so
/// correctness is preserved.
const TOKEN_SET_CAP: usize = 10_000;

/// Max rows retained per vector column to compute the ANN prune radius. Past
/// this, vector stats for the extent are dropped (recall falls back to an
/// exact scan of that extent — correct, just unpruned).
const VEC_ROW_CAP: usize = 50_000;

/// Tokenize a string into lowercased word-level tokens. Splits on any
/// non-alphanumeric character (ASCII). Keeps tokens ≥ 2 chars — single
/// letters are usually noise (the, a, o, …).
pub(crate) fn tokenize(s: &str, out: &mut HashSet<String>) {
    let mut cur = String::with_capacity(16);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            if cur.len() >= 2 {
                out.insert(cur.clone());
            }
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.insert(cur);
    }
}

pub struct TelemetryExtentWriter {
    format: TelemetryFormat,
    extent_id: ExtentId,
    schema: SchemaRef,
    // Arrow IPC FileWriter over a Vec<u8> — phase A buffers in-memory.
    ipc: FileWriter<Vec<u8>>,
    row_count: u64,
    block_count: u32,
    min_ts_nanos: Option<i64>,
    max_ts_nanos: Option<i64>,
    ts_col_index: Option<usize>,
    /// Per-indexable-column distinct-value tracking. Key is column index
    /// in the schema. `None` means "cardinality exceeded — give up."
    distinct_string: Vec<Option<HashSet<String>>>,
    distinct_int: Vec<Option<HashSet<i64>>>,
    /// Per-string-column word-level token set, for text-search pruning.
    /// `None` means "too many tokens — give up" (query-time pruning
    /// degrades to no pruning).
    tokens: Vec<Option<HashSet<String>>>,
    /// Which schema columns get indexed (only stringish and integer types).
    indexable_columns: Vec<(usize, IndexableKind)>,
    /// Per-vector-column ANN accumulation (aligned by `indexable_columns`
    /// position). `vec_sum` is the running sum of L2-normalized rows
    /// (centroid = sum/count); `vec_rows` retains the normalized rows for the
    /// radius (max distance to centroid). Both `None` for non-vector columns
    /// or once the row cap is hit (vector stats then dropped → exact fallback).
    vec_sum: Vec<Option<Vec<f64>>>,
    vec_rows: Vec<Option<Vec<Vec<f32>>>>,
    vec_count: Vec<u64>,
    /// Per-block min/max stats, one entry per appended batch. Emitted as a
    /// JSON footer after the Arrow IPC body (v2 format).
    block_stats: Vec<BlockStats>,
}

#[derive(Clone, Copy, Debug)]
enum IndexableKind {
    String,
    Int32,
    Int64,
    /// A `FixedSizeList<Float32>` embedding column — indexed for ANN extent
    /// pruning via a per-extent centroid + radius on the unit sphere.
    Vector {
        dim: usize,
    },
}

impl TelemetryExtentWriter {
    pub(crate) fn new(format: TelemetryFormat, schema: SchemaRef) -> Self {
        // Reserve some capacity up front — ~16 MiB is a reasonable guess for
        // phase-A extents we generate from tests.
        let buffer: Vec<u8> = Vec::with_capacity(16 * 1024 * 1024);
        let ipc =
            FileWriter::try_new(buffer, &schema).expect("Arrow IPC FileWriter construction failed");
        let ts_col_index = schema
            .fields()
            .iter()
            .position(|f| matches!(f.data_type(), DataType::Timestamp(TimeUnit::Nanosecond, _)));

        // Pre-compute which columns we'll build equality indexes on.
        let mut indexable_columns = Vec::new();
        for (i, field) in schema.fields().iter().enumerate() {
            match field.data_type() {
                DataType::Utf8 | DataType::LargeUtf8 => {
                    indexable_columns.push((i, IndexableKind::String));
                }
                DataType::Int32 => {
                    indexable_columns.push((i, IndexableKind::Int32));
                }
                DataType::Int64 => {
                    indexable_columns.push((i, IndexableKind::Int64));
                }
                DataType::FixedSizeList(inner, dim)
                    if matches!(inner.data_type(), DataType::Float32) =>
                {
                    indexable_columns.push((i, IndexableKind::Vector { dim: *dim as usize }));
                }
                _ => {}
            }
        }
        let distinct_string = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::String).then(HashSet::new))
            .collect();
        let distinct_int = indexable_columns
            .iter()
            .map(|(_, k)| {
                matches!(k, IndexableKind::Int32 | IndexableKind::Int64).then(HashSet::new)
            })
            .collect();
        let tokens = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::String).then(HashSet::new))
            .collect();
        let vec_sum = indexable_columns
            .iter()
            .map(|(_, k)| match k {
                IndexableKind::Vector { dim } => Some(vec![0.0f64; *dim]),
                _ => None,
            })
            .collect();
        let vec_rows = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::Vector { .. }).then(Vec::new))
            .collect();
        let vec_count = vec![0u64; indexable_columns.len()];

        Self {
            format,
            extent_id: ExtentId::new(),
            schema,
            ipc,
            row_count: 0,
            block_count: 0,
            min_ts_nanos: None,
            max_ts_nanos: None,
            ts_col_index,
            distinct_string,
            distinct_int,
            tokens,
            indexable_columns,
            vec_sum,
            vec_rows,
            vec_count,
            block_stats: Vec::new(),
        }
    }

    fn update_ts_bounds(&mut self, batch: &RecordBatch) {
        let Some(idx) = self.ts_col_index else {
            return;
        };
        let col = batch.column(idx);
        let Some(arr) = col.as_any().downcast_ref::<TimestampNanosecondArray>() else {
            return;
        };
        for i in 0..arr.len() {
            if arr.is_null(i) {
                continue;
            }
            let v = arr.value(i);
            self.min_ts_nanos = Some(match self.min_ts_nanos {
                Some(m) => m.min(v),
                None => v,
            });
            self.max_ts_nanos = Some(match self.max_ts_nanos {
                Some(m) => m.max(v),
                None => v,
            });
        }
    }

    /// Fold the batch's values into the per-column distinct sets. If a set
    /// exceeds the cardinality cap, it's dropped (`None`) and the column
    /// effectively disables equality-pruning for this extent.
    fn update_distinct_sets(&mut self, batch: &RecordBatch) {
        // Iterate by position in `indexable_columns` so the distinct vecs
        // stay aligned by index.
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let col = batch.column(*col_idx);
            match kind {
                IndexableKind::String => {
                    let arr: &StringArray = col.as_string();
                    for i in 0..arr.len() {
                        if arr.is_null(i) {
                            continue;
                        }
                        let value = arr.value(i);

                        // Whole-value distinct set (for equality pruning).
                        if let Some(set) = self.distinct_string[pos].as_mut() {
                            if set.len() >= DISTINCT_SET_CAP {
                                self.distinct_string[pos] = None;
                            } else {
                                set.insert(value.to_owned());
                            }
                        }

                        // Word-level token set (for text-search pruning).
                        if let Some(tokens) = self.tokens[pos].as_mut() {
                            tokenize(value, tokens);
                            if tokens.len() > TOKEN_SET_CAP {
                                self.tokens[pos] = None;
                            }
                        }
                    }
                }
                IndexableKind::Int32 => {
                    let Some(set) = self.distinct_int[pos].as_mut() else {
                        continue;
                    };
                    let Some(arr) = col.as_any().downcast_ref::<Int32Array>() else {
                        continue;
                    };
                    for i in 0..arr.len() {
                        if arr.is_null(i) {
                            continue;
                        }
                        if set.len() >= DISTINCT_SET_CAP {
                            self.distinct_int[pos] = None;
                            break;
                        }
                        set.insert(arr.value(i) as i64);
                    }
                }
                IndexableKind::Int64 => {
                    let Some(set) = self.distinct_int[pos].as_mut() else {
                        continue;
                    };
                    let Some(arr) = col.as_any().downcast_ref::<Int64Array>() else {
                        continue;
                    };
                    for i in 0..arr.len() {
                        if arr.is_null(i) {
                            continue;
                        }
                        if set.len() >= DISTINCT_SET_CAP {
                            self.distinct_int[pos] = None;
                            break;
                        }
                        set.insert(arr.value(i));
                    }
                }
                // Vector columns are handled by `update_vector_stats`.
                IndexableKind::Vector { .. } => {}
            }
        }
    }

    /// Accumulate L2-normalized embedding rows into each vector column's
    /// running centroid sum + retained-row set. Normalizing puts everything on
    /// the unit sphere, where cosine distance = ½·‖·‖² and the centroid+radius
    /// triangle-inequality bound is valid (cosine itself isn't a metric).
    fn update_vector_stats(&mut self, batch: &RecordBatch) {
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let IndexableKind::Vector { dim } = kind else {
                continue;
            };
            let dim = *dim;
            // Disabled (cap hit on a prior batch) → nothing to do.
            if self.vec_sum[pos].is_none() {
                continue;
            }
            let col = batch.column(*col_idx);
            let Some(arr) = col.as_any().downcast_ref::<FixedSizeListArray>() else {
                continue;
            };
            let Some(floats) = arr.values().as_any().downcast_ref::<Float32Array>() else {
                continue;
            };
            for i in 0..arr.len() {
                if arr.is_null(i) {
                    continue;
                }
                let start = i * dim;
                let mut v: Vec<f32> = (0..dim).map(|j| floats.value(start + j)).collect();
                let norm = v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
                if norm <= 1e-12 {
                    continue; // zero vector — can't normalize; skip (keeps bound safe)
                }
                for x in v.iter_mut() {
                    *x = (*x as f64 / norm) as f32;
                }
                if let Some(sum) = self.vec_sum[pos].as_mut() {
                    for (s, x) in sum.iter_mut().zip(v.iter()) {
                        *s += *x as f64;
                    }
                }
                self.vec_count[pos] += 1;
                match self.vec_rows[pos].as_mut() {
                    Some(rows) if rows.len() < VEC_ROW_CAP => rows.push(v),
                    Some(_) => {
                        // Cap hit — can't compute an exact radius; drop the
                        // stat so recall falls back to an exact scan here.
                        self.vec_rows[pos] = None;
                        self.vec_sum[pos] = None;
                    }
                    None => {}
                }
            }
        }
    }

    fn build_column_stats(&self) -> serde_json::Value {
        let mut stats = serde_json::Map::new();
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let name = self.schema.field(*col_idx).name();
            // Vector columns carry an ANN centroid+radius instead of distinct/tokens.
            if let IndexableKind::Vector { .. } = kind {
                if let Some(vec_stat) = self.vector_stat(pos) {
                    stats.insert(name.clone(), json!({ "vec": vec_stat }));
                }
                continue;
            }
            let distinct: serde_json::Value = match kind {
                IndexableKind::String => match &self.distinct_string[pos] {
                    Some(set) => {
                        let mut vals: Vec<&String> = set.iter().collect();
                        vals.sort();
                        serde_json::Value::Array(vals.into_iter().map(|s| json!(s)).collect())
                    }
                    None => serde_json::Value::Null,
                },
                IndexableKind::Int32 | IndexableKind::Int64 => match &self.distinct_int[pos] {
                    Some(set) => {
                        let mut vals: Vec<i64> = set.iter().copied().collect();
                        vals.sort();
                        serde_json::Value::Array(vals.into_iter().map(|v| json!(v)).collect())
                    }
                    None => serde_json::Value::Null,
                },
                IndexableKind::Vector { .. } => unreachable!("handled above"),
            };
            // Token set: only for string columns (None for ints).
            let tokens: serde_json::Value = match kind {
                IndexableKind::String => match &self.tokens[pos] {
                    Some(set) => {
                        let mut toks: Vec<&String> = set.iter().collect();
                        toks.sort();
                        serde_json::Value::Array(toks.into_iter().map(|t| json!(t)).collect())
                    }
                    None => serde_json::Value::Null,
                },
                _ => serde_json::Value::Null,
            };
            stats.insert(
                name.clone(),
                json!({
                    "distinct": distinct,
                    "tokens": tokens,
                }),
            );
        }
        serde_json::Value::Object(stats)
    }

    /// Compute the per-extent vector stat for column `pos`: the centroid (mean
    /// of L2-normalized rows) and radius (max distance to centroid). `None`
    /// when no rows were accumulated or the row cap disabled the stat.
    fn vector_stat(&self, pos: usize) -> Option<serde_json::Value> {
        let sum = self.vec_sum[pos].as_ref()?;
        let rows = self.vec_rows[pos].as_ref()?;
        let n = self.vec_count[pos];
        if n == 0 {
            return None;
        }
        let nf = n as f64;
        let centroid: Vec<f64> = sum.iter().map(|s| s / nf).collect();
        let mut radius = 0.0f64;
        for r in rows {
            let mut d = 0.0f64;
            for (j, x) in r.iter().enumerate() {
                let diff = *x as f64 - centroid[j];
                d += diff * diff;
            }
            let d = d.sqrt();
            if d > radius {
                radius = d;
            }
        }
        Some(json!({ "centroid": centroid, "radius": radius, "count": n, "metric": "cosine" }))
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
        self.update_ts_bounds(&batch);
        self.update_distinct_sets(&batch);
        self.update_vector_stats(&batch);
        self.block_stats.push(stats_for_batch(&batch));
        self.ipc.write(&batch).map_err(|e| FormatError::Corrupt {
            path: "<in-memory ipc>".to_string(),
            detail: format!("FileWriter::write: {e}"),
        })?;
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<ExtentWriteResult> {
        // Capture column stats *before* destructuring — the writer owns the
        // distinct sets and `build_column_stats` borrows it.
        let column_stats = self.build_column_stats();
        let TelemetryExtentWriter {
            format,
            extent_id,
            row_count,
            block_count,
            min_ts_nanos,
            max_ts_nanos,
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

        let object_path = format_extent_path(format.path_prefix(), format.tenant_segment(), &extent_id);
        let store: Arc<dyn object_store::ObjectStore> = format.store().clone();

        store
            .put(&Path::from(object_path.as_str()), PutPayload::from(payload))
            .await
            .map_err(|e| kyma_core::errors::StorageError::ObjectStore(e.to_string()))?;

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
        format!("extents/{extent_id}.kyma")
    } else {
        format!("{tenant_segment}/extents/{extent_id}.kyma")
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
    use kyma_core::types::ExtentId;
    use uuid::Uuid;

    #[test]
    fn legacy_path_when_tenant_empty() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let path = format_extent_path("kyma", "", &id);
        assert_eq!(path, "kyma/extents/00000000-0000-0000-0000-000000000000.kyma");
    }

    #[test]
    fn tenant_segmented_path() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let tenant = "11111111-1111-1111-1111-111111111111";
        let path = format_extent_path("kyma", tenant, &id);
        assert_eq!(
            path,
            "kyma/11111111-1111-1111-1111-111111111111/extents/00000000-0000-0000-0000-000000000000.kyma"
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
