//! JSON rows → Arrow `RecordBatch` coercion, matching kyma-ingest-rest.
//!
//! Serializes to NDJSON bytes and feeds `arrow::json::ReaderBuilder`, so the
//! coercion rules (type promotion, missing-column null-fill, etc.) are
//! exactly identical to what REST ingest applies.

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use std::io::Cursor;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CoerceError {
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

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
    let reader = arrow::json::ReaderBuilder::new(schema.clone()).build(Cursor::new(buf))?;
    let mut out = Vec::new();
    for b in reader {
        out.push(b?);
    }
    Ok(out)
}
