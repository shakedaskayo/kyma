//! Canonical node/edge row shape for graph data sources.
//!
//! The data source framework's sink expects each row to land with a fixed top-
//! level shape: nodes as `{ id, labels (string), name, props (json string) }`
//! and edges as `{ id, src, dst, type, props }`. Extra fields are packed into
//! `props` as a JSON-serialized object so the storage table stays narrow (the
//! evolve_schema seam caps at 32 columns) and the `<g>_nodes`/`<g>_edges`
//! tables remain queryable.
//!
//! Data sources are free to build rich rows (any extras) and call `normalize_*`
//! at emit time — this keeps the per-vendor code legible while satisfying the
//! framework contract.

use serde_json::{json, Map, Value};

/// `PullRequest` → `pull_request` — for deriving a `type` resource segment from
/// a node label.
pub(crate) fn snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Reshape a rich node JSON object into the canonical row shape. Accepts
/// `labels` as either a string or an array (uses the first element).
pub fn normalize_node(v: Value) -> Value {
    let mut obj = match v {
        Value::Object(m) => m,
        _ => return Value::Null,
    };
    let id = obj.remove("id").unwrap_or(Value::String(String::new()));
    let labels = match obj.remove("labels") {
        Some(Value::Array(arr)) => arr
            .into_iter()
            .next()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        Some(Value::String(s)) => s,
        _ => String::new(),
    };
    let name = match obj.remove("name") {
        Some(Value::String(s)) => s,
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let mut extras: Map<String, Value> = obj;
    // If the data source tagged a `vendor`, derive a classification
    // `type` ("vendor::resource") from the label so the graph can brand +
    // classify the node (e.g. vendor "github" + label "PullRequest" →
    // "github::pull_request").
    if let Some(Value::String(vendor)) = extras.get("vendor").cloned() {
        if !extras.contains_key("type") && !labels.is_empty() && !vendor.is_empty() {
            extras.insert(
                "type".to_string(),
                Value::String(format!("{vendor}::{}", snake_case(&labels))),
            );
        }
    }
    json!({
        "id": id,
        "labels": labels,
        "name": name,
        "props": serde_json::to_string(&Value::Object(extras)).unwrap_or_default(),
    })
}

/// Reshape a rich edge JSON object into the canonical row shape.
pub fn normalize_edge(v: Value) -> Value {
    let mut obj = match v {
        Value::Object(m) => m,
        _ => return Value::Null,
    };
    let id = obj.remove("id").unwrap_or(Value::String(String::new()));
    let src = obj.remove("src").unwrap_or(Value::String(String::new()));
    let dst = obj.remove("dst").unwrap_or(Value::String(String::new()));
    let rel = match obj.remove("type") {
        Some(Value::String(s)) => s,
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let extras: Map<String, Value> = obj;
    json!({
        "id": id,
        "src": src,
        "dst": dst,
        "type": rel,
        "props": serde_json::to_string(&Value::Object(extras)).unwrap_or_default(),
    })
}
