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
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
}

#[test]
fn rejects_unparseable_json() {
    let bytes = b"{not json";
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::PARSE_ERROR);
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
    assert_eq!(ErrorCode::PARSE_ERROR, -32700);
    assert_eq!(ErrorCode::INVALID_REQUEST, -32600);
    assert_eq!(ErrorCode::METHOD_NOT_FOUND, -32601);
    assert_eq!(ErrorCode::INVALID_PARAMS, -32602);
    assert_eq!(ErrorCode::INTERNAL_ERROR, -32603);
    let e = ErrorObject::new(ErrorCode::METHOD_NOT_FOUND, "no such method");
    assert_eq!(e.code, -32601);
    assert_eq!(e.message, "no such method");
}
