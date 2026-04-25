//! Shared NDJSON → Arrow RecordBatch converter.
//!
//! Wraps arrow-json's `ReaderBuilder` for all columns that Arrow's JSON
//! reader already supports, and adds:
//! - `FixedSizeList<Float32, N>` (vector columns) — upstream arrow-json rejects these.
//! - `Binary` (dynamic columns) — arrow-json's JSON reader cannot decode Binary.
//!   Binary/dynamic columns are read from the NDJSON as either a JSON string (stored
//!   as UTF-8 bytes) or any other value serialized to JSON bytes. This matches kyma's
//!   `dynamic` column semantic: the stored value is the raw JSON representation.
//!
//! Used by every ingest frontend so vector and dynamic columns work the same way
//! from REST, Kafka, and file-drop.

use arrow_array::{ArrayRef, BinaryArray, FixedSizeListArray, Float32Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use std::io::BufReader;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NdjsonError {
    #[error("arrow-json: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("column `{column}`: expected array of {dimension} floats, got length {got}")]
    VectorDimensionMismatch {
        column: String,
        dimension: i32,
        got: usize,
    },
    #[error("column `{column}`: expected array of floats, got {got}")]
    VectorWrongType { column: String, got: String },
    #[error("column `{column}`: non-numeric element at index {index}: {value}")]
    VectorNonNumeric {
        column: String,
        index: usize,
        value: String,
    },
    #[error("column `{column}`: FixedSizeList inner type must be Float32, got {got:?}")]
    VectorUnsupportedInner { column: String, got: DataType },
}

/// Parse NDJSON bytes into a batch of `RecordBatch`es against the given schema.
///
/// Columns handled via the manual slow path (stripped from arrow-json then spliced):
/// - `FixedSizeList<Float32, N>` — vector columns
/// - `Binary` / `LargeBinary` — dynamic columns (stored as raw JSON bytes)
///
/// All other columns delegate to arrow-json's built-in reader.
pub fn parse_ndjson(bytes: &[u8], schema: SchemaRef) -> Result<Vec<RecordBatch>, NdjsonError> {
    // Identify columns that arrow-json cannot handle natively.
    let mut vector_cols: Vec<(usize, String, i32)> = Vec::new();
    let mut binary_cols: Vec<(usize, String)> = Vec::new();

    for (i, f) in schema.fields().iter().enumerate() {
        match f.data_type() {
            DataType::FixedSizeList(inner, dim) => match inner.data_type() {
                DataType::Float32 => vector_cols.push((i, f.name().clone(), *dim)),
                other => {
                    return Err(NdjsonError::VectorUnsupportedInner {
                        column: f.name().clone(),
                        got: other.clone(),
                    })
                }
            },
            DataType::Binary | DataType::LargeBinary => {
                binary_cols.push((i, f.name().clone()));
            }
            _ => {}
        }
    }

    if vector_cols.is_empty() && binary_cols.is_empty() {
        // Fast path: pure arrow-json, preserves existing behavior.
        let reader = arrow_json::ReaderBuilder::new(schema).build(BufReader::new(bytes))?;
        return reader
            .collect::<Result<Vec<_>, _>>()
            .map_err(NdjsonError::from);
    }

    // Slow path: strip unsupported columns from schema, parse the rest via
    // arrow-json, build manual arrays for the stripped columns, then splice
    // everything back.
    //
    // Important: arrow-json ignores unknown JSON fields by default, so
    // leaving the stripped fields' values in the raw NDJSON is safe.
    let is_manual = |i: usize| {
        vector_cols.iter().any(|(vi, _, _)| *vi == i)
            || binary_cols.iter().any(|(bi, _)| *bi == i)
    };

    let stripped_fields: Vec<Arc<Field>> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(i, _)| !is_manual(*i))
        .map(|(_, f)| f.clone())
        .collect();
    let stripped_schema = Arc::new(Schema::new(stripped_fields));
    let stripped_batches: Vec<RecordBatch> =
        arrow_json::ReaderBuilder::new(stripped_schema.clone())
            .build(BufReader::new(bytes))?
            .collect::<Result<Vec<_>, _>>()?;

    // Parse raw NDJSON lines to extract values for manual columns.
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(line)?;
        rows.push(v);
    }

    // ---- Build vector arrays ----
    let mut manual_arrays: Vec<(usize, ArrayRef)> = Vec::new();

    for (pos, name, dim) in &vector_cols {
        let mut flat: Vec<f32> = Vec::with_capacity(rows.len() * *dim as usize);
        for row in &rows {
            let val = row.get(name).ok_or_else(|| NdjsonError::VectorWrongType {
                column: name.clone(),
                got: "missing".into(),
            })?;
            let arr = val.as_array().ok_or_else(|| NdjsonError::VectorWrongType {
                column: name.clone(),
                got: serde_json_type_name(val).into(),
            })?;
            if arr.len() != *dim as usize {
                return Err(NdjsonError::VectorDimensionMismatch {
                    column: name.clone(),
                    dimension: *dim,
                    got: arr.len(),
                });
            }
            for (idx, item) in arr.iter().enumerate() {
                let f = item.as_f64().ok_or_else(|| NdjsonError::VectorNonNumeric {
                    column: name.clone(),
                    index: idx,
                    value: item.to_string(),
                })? as f32;
                flat.push(f);
            }
        }
        let values = Float32Array::from(flat);
        let inner_field = Arc::new(Field::new("item", DataType::Float32, false));
        let arr = FixedSizeListArray::new(inner_field, *dim, Arc::new(values), None);
        manual_arrays.push((*pos, Arc::new(arr) as ArrayRef));
    }

    // ---- Build binary (dynamic) arrays ----
    // For each Binary column, the JSON value is encoded as its UTF-8 JSON
    // representation (or the raw string bytes if the value is a JSON string).
    // This matches kyma's `dynamic` semantic: the stored bytes are the JSON.
    for (pos, name) in &binary_cols {
        let mut bufs: Vec<Option<Vec<u8>>> = Vec::with_capacity(rows.len());
        for row in &rows {
            match row.get(name) {
                None | Some(serde_json::Value::Null) => bufs.push(None),
                Some(serde_json::Value::String(s)) => bufs.push(Some(s.as_bytes().to_vec())),
                Some(other) => {
                    // Serialize any non-string JSON value back to bytes.
                    let encoded = serde_json::to_vec(other)?;
                    bufs.push(Some(encoded));
                }
            }
        }
        let arr = BinaryArray::from(
            bufs.iter()
                .map(|opt| opt.as_deref())
                .collect::<Vec<Option<&[u8]>>>(),
        );
        manual_arrays.push((*pos, Arc::new(arr) as ArrayRef));
    }

    // Sort by position so the splice loop below is index-stable.
    manual_arrays.sort_by_key(|(pos, _)| *pos);

    // Splice back: build full RecordBatches by merging stripped batches with
    // manual arrays. arrow-json may emit multiple batches; slice manual arrays
    // accordingly.
    let mut out = Vec::with_capacity(stripped_batches.len());
    let mut offset: usize = 0;
    for stripped in stripped_batches {
        let nrows = stripped.num_rows();
        let mut full_cols: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
        let mut stripped_idx: usize = 0;
        for (i, _) in schema.fields().iter().enumerate() {
            if let Some((_, arr)) = manual_arrays.iter().find(|(vi, _)| *vi == i) {
                full_cols.push(arr.slice(offset, nrows));
            } else {
                full_cols.push(stripped.column(stripped_idx).clone());
                stripped_idx += 1;
            }
        }
        out.push(RecordBatch::try_new(schema.clone(), full_cols)?);
        offset += nrows;
    }
    Ok(out)
}

fn serde_json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod binary_column_tests {
    use super::*;
    use arrow_array::Array;
    use arrow_schema::{Field, Schema};
    use std::sync::Arc;

    fn make_schema_with_binary() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, true),
            Field::new("props", DataType::Binary, true),
        ]))
    }

    #[test]
    fn binary_col_from_string_value() {
        let schema = make_schema_with_binary();
        let ndjson = b"{\"id\":\"a\",\"props\":\"{\\\"x\\\":1}\"}\n";
        let batches = parse_ndjson(ndjson, schema).expect("parse ok");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);
        let arr = batches[0]
            .column_by_name("props")
            .expect("props column exists");
        let bin = arr
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("BinaryArray");
        assert_eq!(bin.value(0), b"{\"x\":1}");
    }

    #[test]
    fn binary_col_from_object_value() {
        let schema = make_schema_with_binary();
        let ndjson = b"{\"id\":\"b\",\"props\":{\"y\":2}}\n";
        let batches = parse_ndjson(ndjson, schema).expect("parse ok");
        let arr = batches[0]
            .column_by_name("props")
            .expect("props column exists");
        let bin = arr
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("BinaryArray");
        let stored: serde_json::Value =
            serde_json::from_slice(bin.value(0)).expect("stored bytes are valid JSON");
        assert_eq!(stored["y"], serde_json::json!(2));
    }

    #[test]
    fn binary_col_null_value() {
        let schema = make_schema_with_binary();
        let ndjson = b"{\"id\":\"c\",\"props\":null}\n";
        let batches = parse_ndjson(ndjson, schema).expect("parse ok");
        let arr = batches[0]
            .column_by_name("props")
            .expect("props column exists");
        let bin = arr
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("BinaryArray");
        assert!(bin.is_null(0));
    }
}
