use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_connectors::arrow_coerce::rows_to_batches;
use serde_json::json;
use std::sync::Arc;

fn metrics_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Float64, true),
        Field::new("labels", DataType::Utf8, true),
    ]))
}

#[test]
fn coerces_rows_into_one_batch() {
    let schema = metrics_schema();
    let rows = vec![
        json!({
            "timestamp": "2026-04-20T12:00:00Z",
            "name": "a",
            "value": 1.5,
            "labels": "{\"x\":\"y\"}",
        }),
        json!({
            "timestamp": "2026-04-20T12:00:00Z",
            "name": "b",
            "value": null,
            "labels": "{}",
        }),
    ];
    let batches = rows_to_batches(&schema, rows).expect("coerce");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);
    assert_eq!(batches[0].num_columns(), 4);
}

#[test]
fn empty_rows_yields_empty_vec() {
    let schema = metrics_schema();
    let batches = rows_to_batches(&schema, vec![]).expect("coerce");
    assert!(batches.is_empty());
}

#[test]
fn coerces_rows_with_binary_props_column() {
    // Reproduces the connector-sink bug surfaced by a live ingest:
    // `ensure_table` creates tables with the default schema, whose `props`
    // catch-all column is `Binary`. arrow-json's reader rejects Binary, so a
    // naive ReaderBuilder call fails with "Binary is not supported by JSON".
    // The github connector emits rows with a JSON-string `props` field, so the
    // sink must coerce them through the shared parse_ndjson Binary path.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, true),
        Field::new("labels", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("props", DataType::Binary, true),
    ]));
    let rows = vec![
        json!({"id": "repo:o/r", "labels": "Repository", "name": "r", "props": "{\"stars\":3}"}),
        json!({"id": "user:o", "labels": "User", "name": "o", "props": null}),
    ];
    let batches = rows_to_batches(&schema, rows).expect("coerce with binary props");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);
    assert_eq!(batches[0].num_columns(), 4);
}
