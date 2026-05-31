//! Build JSON rows for the columnar memory tables. Rows are serialized to
//! NDJSON and coerced to Arrow by `kyma_ingest_core::parse_ndjson`, so the
//! `embedding` value is a plain JSON array of floats.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::types::CreateMemory;

/// Max length of the stored content preview.
const PREVIEW_CHARS: usize = 280;

/// Canonical node id for a memory uuid.
pub fn node_id(id: &Uuid) -> String {
    format!("memory:{id}")
}

/// Deterministic edge id, matching the `{src}->{dst}:{type}` convention used
/// elsewhere so dedup keys stay stable.
pub fn edge_id(src: &str, dst: &str, rel_type: &str) -> String {
    format!("{src}->{dst}:{rel_type}")
}

pub fn preview(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.chars().count() <= PREVIEW_CHARS {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}…")
}

/// Build a `memory_nodes` row. `now` is an RFC3339 timestamp.
pub fn node_row(id: &Uuid, m: &CreateMemory, embedding: &[f32], now: &str) -> Value {
    json!({
        "id": node_id(id),
        "labels": "Memory",
        "realm": m.realm,
        "memory_type": m.memory_type.as_str(),
        "title": m.title,
        "content": m.content,
        "content_preview": preview(&m.content),
        "tags": m.tags.join(","),
        "importance": m.importance as f64,
        "status": "active",
        "source_session_id": m.source_session_id.map(|u| u.to_string()),
        "source_run_id": m.source_run_id.map(|u| u.to_string()),
        "embedding": embedding,
        "created_at": now,
        "updated_at": now,
    })
}

/// Build a `memory_edges` row.
pub fn edge_row(
    src: &str,
    dst: &str,
    rel_type: &str,
    realm: &str,
    target_namespace: Option<&str>,
    props: Option<&Value>,
    now: &str,
) -> Value {
    json!({
        "id": edge_id(src, dst, rel_type),
        "src": src,
        "dst": dst,
        "type": rel_type,
        "realm": realm,
        "target_namespace": target_namespace,
        "props": props.map(|p| p.to_string()),
        "created_at": now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CreateMemory, MemoryType};

    #[test]
    fn preview_truncates_long_content() {
        let long = "x".repeat(1000);
        let p = preview(&long);
        assert!(p.chars().count() <= PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn node_row_carries_embedding_array_and_ids() {
        let id = Uuid::nil();
        let mut m = CreateMemory::new("hello world");
        m.memory_type = MemoryType::Decision;
        m.tags = vec!["a".into(), "b".into()];
        // Use f32-exact values so the JSON round-trip compares cleanly.
        let row = node_row(&id, &m, &[0.5, 0.25, 0.125], "2026-05-31T00:00:00Z");
        assert_eq!(row["id"], json!("memory:00000000-0000-0000-0000-000000000000"));
        assert_eq!(row["memory_type"], json!("decision"));
        assert_eq!(row["tags"], json!("a,b"));
        assert_eq!(row["embedding"], json!([0.5, 0.25, 0.125]));
        assert_eq!(row["status"], json!("active"));
    }

    #[test]
    fn edge_id_is_stable() {
        assert_eq!(
            edge_id("memory:1", "default::repo:x", "REFERENCES"),
            "memory:1->default::repo:x:REFERENCES"
        );
    }
}
