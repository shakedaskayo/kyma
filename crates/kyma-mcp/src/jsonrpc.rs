//! JSON-RPC 2.0 frame types and codec.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Standard JSON-RPC 2.0 error codes plus our internal-error sentinel.
/// `non_exhaustive` reserves space for future codes without breaking callers.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[non_exhaustive]
#[repr(i64)]
pub enum ErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
}

impl From<ErrorCode> for i64 {
    fn from(c: ErrorCode) -> i64 { c as i64 }
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
        .map_err(|e| ErrorObject::new(ErrorCode::ParseError as i64, format!("parse: {e}")))?;
    match raw {
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ErrorObject::new(ErrorCode::InvalidRequest as i64, "batch must be non-empty"));
            }
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                let req: Request = serde_json::from_value(v).map_err(|e| {
                    ErrorObject::new(ErrorCode::InvalidRequest as i64, format!("batch item: {e}"))
                })?;
                if req.jsonrpc != "2.0" {
                    return Err(ErrorObject::new(ErrorCode::InvalidRequest as i64, "jsonrpc must be \"2.0\""));
                }
                out.push(req);
            }
            Ok(RequestEnvelope::Batch(out))
        }
        Value::Object(_) => {
            let req: Request = serde_json::from_value(raw)
                .map_err(|e| ErrorObject::new(ErrorCode::InvalidRequest as i64, format!("request: {e}")))?;
            if req.jsonrpc != "2.0" {
                return Err(ErrorObject::new(ErrorCode::InvalidRequest as i64, "jsonrpc must be \"2.0\""));
            }
            Ok(RequestEnvelope::Single(req))
        }
        _ => Err(ErrorObject::new(ErrorCode::InvalidRequest as i64, "request must be object or array")),
    }
}
