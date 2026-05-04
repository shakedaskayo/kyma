//! JSON-RPC 2.0 frame types and codec.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct ErrorCode;

impl ErrorCode {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Id>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn success(id: Id, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn error(id: Id, error: ErrorObject) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(error) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

#[derive(Debug)]
pub enum RequestEnvelope {
    Single(Request),
    Batch(Vec<Request>),
}

pub fn parse_envelope(body: &[u8]) -> Result<RequestEnvelope, ErrorObject> {
    let raw: Value = serde_json::from_slice(body)
        .map_err(|e| ErrorObject::new(ErrorCode::PARSE_ERROR, format!("parse: {e}")))?;
    match raw {
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "batch must be non-empty"));
            }
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                let req: Request = serde_json::from_value(v).map_err(|e| {
                    ErrorObject::new(ErrorCode::INVALID_REQUEST, format!("batch item: {e}"))
                })?;
                if req.jsonrpc != "2.0" {
                    return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "jsonrpc must be \"2.0\""));
                }
                out.push(req);
            }
            Ok(RequestEnvelope::Batch(out))
        }
        Value::Object(_) => {
            let req: Request = serde_json::from_value(raw)
                .map_err(|e| ErrorObject::new(ErrorCode::INVALID_REQUEST, format!("request: {e}")))?;
            if req.jsonrpc != "2.0" {
                return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "jsonrpc must be \"2.0\""));
            }
            Ok(RequestEnvelope::Single(req))
        }
        _ => Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "request must be object or array")),
    }
}
