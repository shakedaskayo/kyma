//! Per-block min/max stats, written into the extent footer for v2 extents.
//!
//! Goal: let `pruned_blocks` skip entire blocks before decode when the
//! query's predicate can't possibly match the block. Complements catalog
//! extent-level pruning; this is the next layer down the cascade.
//!
//! Serialized as JSON for ease of evolution. Size is bounded: one object
//! per block, string min/max capped, columns without a trackable type
//! omitted.

use arrow_array::{
    cast::AsArray, Array, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampNanosecondArray,
};
use arrow_schema::DataType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Hard cap on the byte length of stored string min/max. Strings in a
/// block that have any value longer than this disable string min/max for
/// that (block, column) — correctness preserved, pruning just skipped.
pub const STRING_MINMAX_CAP: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockStats {
    pub row_count: u32,
    /// Keyed by column name (not index) so reads survive small schema
    /// reorderings from ALTER TABLE ADD COLUMN.
    pub cols: BTreeMap<String, ColStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum ColStat {
    /// Timestamp nanoseconds since Unix epoch.
    #[serde(rename = "ts")]
    Ts { min: i64, max: i64, nulls: u32 },
    #[serde(rename = "i64")]
    Int64 { min: i64, max: i64, nulls: u32 },
    /// UTF-8 string, min/max bounded by `STRING_MINMAX_CAP`.
    #[serde(rename = "str")]
    Str { min: String, max: String, nulls: u32 },
}

/// Compute stats for one block (RecordBatch). Columns whose type we don't
/// track are omitted — the pruner conservatively keeps such blocks.
pub fn stats_for_batch(batch: &RecordBatch) -> BlockStats {
    let mut cols = BTreeMap::new();
    for (i, field) in batch.schema().fields().iter().enumerate() {
        let col = batch.column(i);
        let name = field.name().clone();
        match field.data_type() {
            DataType::Timestamp(_, _) => {
                if let Some(arr) = col.as_any().downcast_ref::<TimestampNanosecondArray>() {
                    if let Some(s) = ts_stats(arr) {
                        cols.insert(name, s);
                    }
                }
            }
            DataType::Int32 => {
                if let Some(arr) = col.as_any().downcast_ref::<Int32Array>() {
                    if let Some(s) = i32_stats(arr) {
                        cols.insert(name, s);
                    }
                }
            }
            DataType::Int64 => {
                if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
                    if let Some(s) = i64_stats(arr) {
                        cols.insert(name, s);
                    }
                }
            }
            DataType::Utf8 | DataType::LargeUtf8 => {
                let arr: &StringArray = col.as_string();
                if let Some(s) = str_stats(arr) {
                    cols.insert(name, s);
                }
            }
            _ => {}
        }
    }
    BlockStats {
        row_count: batch.num_rows() as u32,
        cols,
    }
}

fn ts_stats(arr: &TimestampNanosecondArray) -> Option<ColStat> {
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    let mut nulls = 0u32;
    let mut any = false;
    for i in 0..arr.len() {
        if arr.is_null(i) {
            nulls += 1;
            continue;
        }
        let v = arr.value(i);
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        any = true;
    }
    if !any {
        return None;
    }
    Some(ColStat::Ts { min, max, nulls })
}

fn i32_stats(arr: &Int32Array) -> Option<ColStat> {
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    let mut nulls = 0u32;
    let mut any = false;
    for i in 0..arr.len() {
        if arr.is_null(i) {
            nulls += 1;
            continue;
        }
        let v = arr.value(i) as i64;
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        any = true;
    }
    if !any {
        return None;
    }
    Some(ColStat::Int64 { min, max, nulls })
}

fn i64_stats(arr: &Int64Array) -> Option<ColStat> {
    let mut min = i64::MAX;
    let mut max = i64::MIN;
    let mut nulls = 0u32;
    let mut any = false;
    for i in 0..arr.len() {
        if arr.is_null(i) {
            nulls += 1;
            continue;
        }
        let v = arr.value(i);
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
        any = true;
    }
    if !any {
        return None;
    }
    Some(ColStat::Int64 { min, max, nulls })
}

fn str_stats(arr: &StringArray) -> Option<ColStat> {
    let mut min: Option<&str> = None;
    let mut max: Option<&str> = None;
    let mut nulls = 0u32;
    for i in 0..arr.len() {
        if arr.is_null(i) {
            nulls += 1;
            continue;
        }
        let v = arr.value(i);
        // Any single value longer than the cap disables string min/max
        // for this block-column — correctness preserved via conservative
        // "keep block" at prune time.
        if v.len() > STRING_MINMAX_CAP {
            return None;
        }
        min = Some(match min {
            Some(cur) => {
                if v < cur {
                    v
                } else {
                    cur
                }
            }
            None => v,
        });
        max = Some(match max {
            Some(cur) => {
                if v > cur {
                    v
                } else {
                    cur
                }
            }
            None => v,
        });
    }
    match (min, max) {
        (Some(mn), Some(mx)) => Some(ColStat::Str {
            min: mn.to_string(),
            max: mx.to_string(),
            nulls,
        }),
        _ => None,
    }
}
