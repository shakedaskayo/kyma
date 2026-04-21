use arrow_array::{cast::AsArray, Array, FixedSizeListArray, Float32Array};
use arrow_schema::{DataType, Field, Schema};
use kyma_ingest_core::parse_ndjson;
use std::sync::Arc;

fn schema_with_vector(dim: i32) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), dim),
            false,
        ),
    ]))
}

#[test]
fn primitive_only_schema_delegates_to_arrow_json_unchanged() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("a", DataType::Int32, false),
        Field::new("b", DataType::Utf8, false),
    ]));
    let ndjson = b"{\"a\":1,\"b\":\"x\"}\n{\"a\":2,\"b\":\"y\"}\n";
    let batches = parse_ndjson(ndjson, schema.clone()).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);
    assert_eq!(batches[0].num_columns(), 2);
}

#[test]
fn coerces_json_float_array_to_fixed_size_list_vector() {
    let schema = schema_with_vector(3);
    let ndjson =
        b"{\"id\":\"a\",\"embedding\":[0.1,0.2,0.3]}\n{\"id\":\"b\",\"embedding\":[0.4,0.5,0.6]}\n";
    let batches = parse_ndjson(ndjson, schema).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);

    let fsl: &FixedSizeListArray = batches[0]
        .column(1)
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .unwrap();
    assert_eq!(fsl.value_length(), 3);
    assert_eq!(fsl.len(), 2);

    let row0 = fsl.value(0);
    let floats0: &Float32Array = row0.as_primitive();
    assert_eq!(floats0.values(), &[0.1_f32, 0.2, 0.3]);

    let row1 = fsl.value(1);
    let floats1: &Float32Array = row1.as_primitive();
    assert_eq!(floats1.values(), &[0.4_f32, 0.5, 0.6]);
}

#[test]
fn rejects_wrong_dimension_for_vector_column() {
    let schema = schema_with_vector(3);
    let ndjson = b"{\"id\":\"a\",\"embedding\":[0.1,0.2]}\n";
    let err = parse_ndjson(ndjson, schema).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("array of 3 floats"), "got: {msg}");
    assert!(msg.contains("got length 2"), "got: {msg}");
    assert!(msg.contains("embedding"), "got: {msg}");
}

#[test]
fn rejects_non_array_value_for_vector_column() {
    let schema = schema_with_vector(3);
    let ndjson = b"{\"id\":\"a\",\"embedding\":\"not-an-array\"}\n";
    let err = parse_ndjson(ndjson, schema).unwrap_err();
    assert!(
        err.to_string().contains("expected array of floats"),
        "got: {err}"
    );
}

#[test]
fn rejects_non_numeric_element_in_vector() {
    let schema = schema_with_vector(3);
    let ndjson = b"{\"id\":\"a\",\"embedding\":[0.1,\"bad\",0.3]}\n";
    let err = parse_ndjson(ndjson, schema).unwrap_err();
    assert!(err.to_string().contains("non-numeric"), "got: {err}");
}
