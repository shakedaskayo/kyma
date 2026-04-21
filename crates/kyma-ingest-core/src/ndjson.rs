//! Shared NDJSON → Arrow RecordBatch converter.
//!
//! Wraps arrow-json's `ReaderBuilder` for all columns that Arrow's JSON
//! reader already supports, and adds FixedSizeList<Float32> handling
//! (which upstream arrow-json rejects with NotYetImplemented). Used by
//! every ingest frontend so vector columns work the same way from REST,
//! Kafka, and file-drop.

use arrow_array::{ArrayRef, FixedSizeListArray, Float32Array, RecordBatch};
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
/// Vector columns (`FixedSizeList<Float32, N>`) are coerced from JSON arrays
/// of floats; any other column defers to arrow-json's built-in reader.
///
/// Behavior invariant: if `schema` contains no vector columns, this function's
/// output is identical to `arrow_json::ReaderBuilder::new(schema).build(bytes)`.
pub fn parse_ndjson(bytes: &[u8], schema: SchemaRef) -> Result<Vec<RecordBatch>, NdjsonError> {
    // Identify vector columns (by schema position, name, inner type, dimension).
    let mut vector_cols: Vec<(usize, String, i32)> = Vec::new();
    for (i, f) in schema.fields().iter().enumerate() {
        if let DataType::FixedSizeList(inner, dim) = f.data_type() {
            match inner.data_type() {
                DataType::Float32 => vector_cols.push((i, f.name().clone(), *dim)),
                other => {
                    return Err(NdjsonError::VectorUnsupportedInner {
                        column: f.name().clone(),
                        got: other.clone(),
                    })
                }
            }
        }
    }

    if vector_cols.is_empty() {
        // Fast path: pure arrow-json, preserves existing behavior.
        let reader =
            arrow_json::ReaderBuilder::new(schema).build(BufReader::new(bytes))?;
        return reader
            .collect::<Result<Vec<_>, _>>()
            .map_err(NdjsonError::from);
    }

    // Slow path: strip vector columns from schema, parse the rest via
    // arrow-json, parse the raw NDJSON lines independently to extract
    // vector values, then splice both back together.
    //
    // Important: arrow-json ignores unknown JSON fields by default, so
    // stripping vector fields from the schema and leaving their values in
    // the raw NDJSON is safe — arrow-json will simply skip them.
    let stripped_fields: Vec<Arc<Field>> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(i, _)| !vector_cols.iter().any(|(vi, _, _)| vi == i))
        .map(|(_, f)| f.clone())
        .collect();
    let stripped_schema = Arc::new(Schema::new(stripped_fields));
    let stripped_batches: Vec<RecordBatch> =
        arrow_json::ReaderBuilder::new(stripped_schema.clone())
            .build(BufReader::new(bytes))?
            .collect::<Result<Vec<_>, _>>()?;

    // Parse raw NDJSON lines for vector values.
    // One row per non-blank line. We build per-column Vec<Vec<f32>> then
    // assemble FixedSizeListArrays matching the row-count of each batch.
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(line)?;
        rows.push(v);
    }

    // For each vector column, build one Float32Array of length rows.len()*dim,
    // then wrap as a FixedSizeListArray. Error on any row where the field is
    // missing / wrong type / wrong dimension / contains non-numeric.
    let mut vector_arrays: Vec<(usize, String, ArrayRef)> = Vec::with_capacity(vector_cols.len());
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
        vector_arrays.push((*pos, name.clone(), Arc::new(arr) as ArrayRef));
    }

    // Splice back: build a full RecordBatch by walking the original schema,
    // taking columns from either the stripped batch or the vector arrays.
    // arrow-json may emit multiple batches if the NDJSON is large; our
    // vector columns cover ALL rows, so we need to slice them per-batch.
    //
    // Assumption: arrow-json's row order matches input order (true for
    // NDJSON), so batch N covers rows [offset, offset+batch.num_rows()).
    let mut out = Vec::with_capacity(stripped_batches.len());
    let mut offset: usize = 0;
    for stripped in stripped_batches {
        let nrows = stripped.num_rows();
        let mut full_cols: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
        let mut stripped_idx: usize = 0;
        for (i, _) in schema.fields().iter().enumerate() {
            if let Some((_, _, full_arr)) = vector_arrays.iter().find(|(vi, _, _)| *vi == i) {
                let sliced = full_arr.slice(offset, nrows);
                full_cols.push(sliced);
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
