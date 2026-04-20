//! Phase-A extent writer.
//!
//! Buffers appended `RecordBatch`es in memory, emits Arrow IPC file bytes
//! on `finish()`, prepends the magic, and uploads a single object. Tracks
//! min/max timestamps for catalog pruning.

use crate::{TelemetryFormat, MAGIC};
use arrow::ipc::writer::FileWriter;
use arrow_array::{
    cast::AsArray, Array, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampNanosecondArray,
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
}

#[derive(Clone, Copy, Debug)]
enum IndexableKind {
    String,
    Int32,
    Int64,
}

impl TelemetryExtentWriter {
    pub(crate) fn new(format: TelemetryFormat, schema: SchemaRef) -> Self {
        // Reserve some capacity up front — ~16 MiB is a reasonable guess for
        // phase-A extents we generate from tests.
        let buffer: Vec<u8> = Vec::with_capacity(16 * 1024 * 1024);
        let ipc = FileWriter::try_new(buffer, &schema)
            .expect("Arrow IPC FileWriter construction failed");
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
                _ => {}
            }
        }
        let distinct_string = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::String).then(HashSet::new))
            .collect();
        let distinct_int = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::Int32 | IndexableKind::Int64).then(HashSet::new))
            .collect();
        let tokens = indexable_columns
            .iter()
            .map(|(_, k)| matches!(k, IndexableKind::String).then(HashSet::new))
            .collect();

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
            }
        }
    }

    fn build_column_stats(&self) -> serde_json::Value {
        let mut stats = serde_json::Map::new();
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let name = self.schema.field(*col_idx).name();
            let distinct: serde_json::Value = match kind {
                IndexableKind::String => match &self.distinct_string[pos] {
                    Some(set) => {
                        let mut vals: Vec<&String> = set.iter().collect();
                        vals.sort();
                        serde_json::Value::Array(
                            vals.into_iter().map(|s| json!(s)).collect(),
                        )
                    }
                    None => serde_json::Value::Null,
                },
                IndexableKind::Int32 | IndexableKind::Int64 => match &self.distinct_int[pos] {
                    Some(set) => {
                        let mut vals: Vec<i64> = set.iter().copied().collect();
                        vals.sort();
                        serde_json::Value::Array(
                            vals.into_iter().map(|v| json!(v)).collect(),
                        )
                    }
                    None => serde_json::Value::Null,
                },
            };
            // Token set: only for string columns (None for ints).
            let tokens: serde_json::Value = match kind {
                IndexableKind::String => match &self.tokens[pos] {
                    Some(set) => {
                        let mut toks: Vec<&String> = set.iter().collect();
                        toks.sort();
                        serde_json::Value::Array(
                            toks.into_iter().map(|t| json!(t)).collect(),
                        )
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
        self.ipc
            .write(&batch)
            .map_err(|e| FormatError::Corrupt {
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

        // Frame: MAGIC || ipc_bytes
        let mut payload: Vec<u8> = Vec::with_capacity(MAGIC.len() + ipc_bytes.len());
        payload.extend_from_slice(MAGIC);
        payload.extend_from_slice(&ipc_bytes);
        let byte_size = payload.len() as u64;

        let object_path = format_extent_path(format.path_prefix(), &extent_id);
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

fn format_extent_path(prefix: &str, extent_id: &ExtentId) -> String {
    // Flat prefix for phase A. Table-level sharding lands with partitioning.
    if prefix.is_empty() {
        format!("extents/{extent_id}.kyma")
    } else {
        format!("{prefix}/extents/{extent_id}.kyma")
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
