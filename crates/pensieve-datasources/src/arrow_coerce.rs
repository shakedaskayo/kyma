//! JSON rows → Arrow `RecordBatch` coercion for the data source sink.
//!
//! Delegates to the shared [`kyma_ingest_core::parse_ndjson`] so data source
//! ingest applies *exactly* the same coercion rules as REST, Kafka, and
//! file-drop. Crucially that includes the two column kinds arrow-json's reader
//! cannot decode on its own:
//!
//! - dynamic `Binary` / `LargeBinary` columns (the default-schema `props`
//!   catch-all) — stored as raw JSON bytes,
//! - `FixedSizeList<Float32, N>` vector columns.
//!
//! Reimplementing this here previously caused data source ingest into any
//! default-schema table to fail with "Binary is not supported by JSON", since
//! `ensure_table` always provisions a Binary `props` column.

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CoerceError {
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("ndjson: {0}")]
    Ndjson(#[from] kyma_ingest_core::NdjsonError),
}

/// Coerce data source JSON rows into `RecordBatch`es against `schema`.
///
/// Serializes the rows to NDJSON and feeds the shared `parse_ndjson` helper,
/// so Binary/vector columns are handled identically to every other ingest
/// frontend. Empty input yields an empty batch list.
pub fn rows_to_batches(
    schema: &Arc<Schema>,
    rows: Vec<serde_json::Value>,
) -> Result<Vec<RecordBatch>, CoerceError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut buf = Vec::with_capacity(rows.len() * 64);
    for r in &rows {
        serde_json::to_writer(&mut buf, r)?;
        buf.push(b'\n');
    }
    Ok(kyma_ingest_core::parse_ndjson(&buf, schema.clone())?)
}
