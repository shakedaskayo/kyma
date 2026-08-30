use crate::initialize::{handle_initialize, ServerInfo};
use serde_json::json;

#[test]
fn responds_with_protocol_and_capabilities() {
    let info = ServerInfo { name: "pensieve".into(), version: "0.0.1".into() };
    let resp = handle_initialize(
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name":"claude-code","version":"1.0"}
        }),
        &info,
    )
    .unwrap();
    assert_eq!(resp["protocolVersion"], "2025-03-26");
    assert!(resp["capabilities"]["tools"].is_object());
    assert_eq!(resp["serverInfo"]["name"], "pensieve");
    assert_eq!(resp["serverInfo"]["version"], "0.0.1");
}

#[test]
fn rejects_missing_protocol_version() {
    let info = ServerInfo { name: "pensieve".into(), version: "0.0.1".into() };
    let err = handle_initialize(json!({}), &info).unwrap_err();
    assert_eq!(err.code, crate::jsonrpc::ErrorCode::InvalidParams as i64);
}
