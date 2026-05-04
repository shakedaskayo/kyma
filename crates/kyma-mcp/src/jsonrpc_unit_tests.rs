use crate::jsonrpc::{
    parse_envelope, ErrorCode, ErrorObject, Id, RequestEnvelope, Response,
};
use serde_json::json;

#[test]
fn parses_single_request_with_numeric_id() {
    let bytes = br#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Single(r) => {
            assert_eq!(r.method, "initialize");
            assert_eq!(r.id, Some(Id::Number(7)));
            assert!(r.params.is_some());
        }
        RequestEnvelope::Batch(_) => panic!("expected single"),
    }
}

#[test]
fn parses_notification_without_id() {
    let bytes = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Single(r) => {
            assert_eq!(r.method, "notifications/initialized");
            assert!(r.id.is_none());
        }
        _ => panic!("expected single"),
    }
}

#[test]
fn parses_batch_of_two() {
    let bytes = br#"[
        {"jsonrpc":"2.0","id":1,"method":"a"},
        {"jsonrpc":"2.0","id":2,"method":"b"}
    ]"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Batch(v) => assert_eq!(v.len(), 2),
        _ => panic!("expected batch"),
    }
}

#[test]
fn rejects_invalid_jsonrpc_version() {
    let bytes = br#"{"jsonrpc":"1.0","id":1,"method":"a"}"#;
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest as i64);
}

#[test]
fn rejects_unparseable_json() {
    let bytes = b"{not json";
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::ParseError as i64);
}

#[test]
fn response_serializes_with_jsonrpc_field() {
    let resp = Response::success(Id::Number(1), json!({"ok": true}));
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains(r#""jsonrpc":"2.0""#));
    assert!(s.contains(r#""result":{"ok":true}"#));
}

#[test]
fn error_object_carries_standard_codes() {
    assert_eq!(ErrorCode::ParseError as i64, -32700);
    assert_eq!(ErrorCode::InvalidRequest as i64, -32600);
    assert_eq!(ErrorCode::MethodNotFound as i64, -32601);
    assert_eq!(ErrorCode::InvalidParams as i64, -32602);
    assert_eq!(ErrorCode::InternalError as i64, -32603);
    let e = ErrorObject::new(ErrorCode::MethodNotFound as i64, "no such method");
    assert_eq!(e.code, -32601);
    assert_eq!(e.message, "no such method");
}

#[test]
fn parses_request_with_string_id() {
    let bytes = br#"{"jsonrpc":"2.0","id":"abc-123","method":"tools/list"}"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Single(r) => {
            assert_eq!(r.id, Some(Id::String("abc-123".into())));
        }
        _ => panic!("expected single"),
    }
}

#[test]
fn rejects_malformed_id_type() {
    // Float id is not allowed by JSON-RPC 2.0 (integer or string only).
    let bytes = br#"{"jsonrpc":"2.0","id":3.14,"method":"x"}"#;
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest as i64);
}

#[test]
fn rejects_top_level_scalar() {
    let bytes = br#""hello""#;
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest as i64);

    let bytes2 = b"42";
    let err2 = parse_envelope(bytes2).unwrap_err();
    assert_eq!(err2.code, ErrorCode::InvalidRequest as i64);
}

#[test]
fn rejects_batch_with_one_bad_item() {
    // Batch where the second entry is missing `method` (required field).
    let bytes = br#"[
        {"jsonrpc":"2.0","id":1,"method":"a"},
        {"jsonrpc":"2.0","id":2}
    ]"#;
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest as i64);
}
