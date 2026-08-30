//! Per-extent column statistics — the catalog-pruning contract.
//!
//! This is the single source of truth for the `column_stats` JSON every extent
//! carries, regardless of segment format (TLM, Parquet, …). The catalog's
//! [`PrunePredicate`](kyma_core::catalog::PrunePredicate) reads these keys
//! (`distinct`, `tokens`, `vec`) to skip extents, and the index scheduler reads
//! `column_stats[col]["vec"]` to decide embed/ANN work — so every writer MUST
//! emit byte-identical stats or pruning + activation diverge per format.
//!
//! A writer feeds each appended `RecordBatch` to [`ColumnStatsAccumulator::update`]
//! and calls [`ColumnStatsAccumulator::finish`] to get the JSON, plus
//! [`ColumnStatsAccumulator::min_ts_nanos`] / [`max_ts_nanos`] for the catalog's
//! time-range prune.

use std::collections::HashSet;

use arrow_array::{
    cast::AsArray, Array, FixedSizeListArray, Float32Array, Int32Array, Int64Array, RecordBatch,
    StringArray, TimestampNanosecondArray,
};
use arrow_schema::{DataType, TimeUnit};
use kyma_core::types::SchemaRef;
use serde_json::json;

/// Max distinct values tracked per column. Past this, the column's distinct set
/// becomes `null` and equality-pruning degrades to no pruning (correct, slower).
const DISTINCT_SET_CAP: usize = 1_000;

/// Max tokens tracked per string column. Past this, the token set becomes `null`
/// and text-search pruning is disabled for the extent (LIKE still filters above).
const TOKEN_SET_CAP: usize = 10_000;

/// Max rows retained per vector column to compute the ANN prune radius. Past
/// this, vector stats are dropped (recall falls back to exact scan of the extent).
const VEC_ROW_CAP: usize = 50_000;

/// Tokenize a string into lowercased word-level tokens (split on non-alphanumeric
/// ASCII, keep tokens ≥ 2 chars). Shared with the tantivy/columnar token rule.
pub fn tokenize(s: &str, out: &mut HashSet<String>) {
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

#[derive(Clone, Copy, Debug)]
enum IndexableKind {
    String,
    Int32,
    Int64,
    Vector { dim: usize },
}

/// Accumulates the per-extent `column_stats` contract across appended batches.
pub struct ColumnStatsAccumulator {
    schema: SchemaRef,
    ts_col_index: Option<usize>,
    min_ts_nanos: Option<i64>,
    max_ts_nanos: Option<i64>,
    indexable_columns: Vec<(usize, IndexableKind)>,
    distinct_string: Vec<Option<HashSet<String>>>,
    distinct_int: Vec<Option<HashSet<i64>>>,
    tokens: Vec<Option<HashSet<String>>>,
    vec_sum: Vec<Option<Vec<f64>>>,
    vec_rows: Vec<Option<Vec<Vec<f32>>>>,
    vec_count: Vec<u64>,
}

impl ColumnStatsAccumulator {
    pub fn new(schema: &SchemaRef) -> Self {
        let ts_col_index = schema
            .fields()
            .iter()
            .position(|f| matches!(f.data_type(), DataType::Timestamp(TimeUnit::Nanosecond, _)));

        let mut indexable_columns = Vec::new();
        for (i, field) in schema.fields().iter().enumerate() {
            match field.data_type() {
                DataType::Utf8 | DataType::LargeUtf8 => {
                    indexable_columns.push((i, IndexableKind::String));
                }
                DataType::Int32 => indexable_columns.push((i, IndexableKind::Int32)),
                DataType::Int64 => indexable_columns.push((i, IndexableKind::Int64)),
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
            schema: schema.clone(),
            ts_col_index,
            min_ts_nanos: None,
            max_ts_nanos: None,
            indexable_columns,
            distinct_string,
            distinct_int,
            tokens,
            vec_sum,
            vec_rows,
            vec_count,
        }
    }

    /// Fold one batch into the running stats.
    pub fn update(&mut self, batch: &RecordBatch) {
        self.update_ts_bounds(batch);
        self.update_distinct_sets(batch);
        self.update_vector_stats(batch);
    }

    pub fn min_ts_nanos(&self) -> Option<i64> {
        self.min_ts_nanos
    }

    pub fn max_ts_nanos(&self) -> Option<i64> {
        self.max_ts_nanos
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
            self.min_ts_nanos = Some(self.min_ts_nanos.map_or(v, |m| m.min(v)));
            self.max_ts_nanos = Some(self.max_ts_nanos.map_or(v, |m| m.max(v)));
        }
    }

    fn update_distinct_sets(&mut self, batch: &RecordBatch) {
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
                        if let Some(set) = self.distinct_string[pos].as_mut() {
                            if set.len() >= DISTINCT_SET_CAP {
                                self.distinct_string[pos] = None;
                            } else {
                                set.insert(value.to_owned());
                            }
                        }
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
                IndexableKind::Vector { .. } => {}
            }
        }
    }

    fn update_vector_stats(&mut self, batch: &RecordBatch) {
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let IndexableKind::Vector { dim } = kind else {
                continue;
            };
            let dim = *dim;
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
                let norm = v
                    .iter()
                    .map(|x| (*x as f64) * (*x as f64))
                    .sum::<f64>()
                    .sqrt();
                if norm <= 1e-12 {
                    continue;
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
                        self.vec_rows[pos] = None;
                        self.vec_sum[pos] = None;
                    }
                    None => {}
                }
            }
        }
    }

    /// Build the `column_stats` JSON object.
    pub fn finish(&self) -> serde_json::Value {
        let mut stats = serde_json::Map::new();
        for (pos, (col_idx, kind)) in self.indexable_columns.iter().enumerate() {
            let name = self.schema.field(*col_idx).name();
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
                json!({ "distinct": distinct, "tokens": tokens }),
            );
        }
        serde_json::Value::Object(stats)
    }

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
