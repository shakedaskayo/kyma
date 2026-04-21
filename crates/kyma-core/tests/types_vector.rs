use kyma_core::types::DataType;

#[test]
fn vector_datatype_roundtrips_through_serde() {
    let dt = DataType::Vector {
        dimension: 384,
        model_id: Some("fastembed/bge-small-en-v1.5".into()),
    };
    let json = serde_json::to_string(&dt).unwrap();
    let back: DataType = serde_json::from_str(&json).unwrap();
    assert_eq!(dt, back);
}

#[test]
fn vector_datatype_display_matches_ddl_form() {
    let no_model = DataType::Vector {
        dimension: 384,
        model_id: None,
    };
    let with_model = DataType::Vector {
        dimension: 1536,
        model_id: Some("openai/text-embedding-3-small".into()),
    };
    assert_eq!(no_model.to_string(), "vector(384)");
    assert_eq!(
        with_model.to_string(),
        "vector(1536) MODEL 'openai/text-embedding-3-small'"
    );
}

#[test]
fn vector_datatype_json_tag_is_kind() {
    // The existing DataType enum uses serde tag="kind". Verify Vector follows suit.
    let dt = DataType::Vector {
        dimension: 4,
        model_id: None,
    };
    let v: serde_json::Value = serde_json::to_value(&dt).unwrap();
    assert_eq!(v["kind"], "vector");
    assert_eq!(v["dimension"], 4);
}
