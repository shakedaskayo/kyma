//! Activities — raw-input capture for worked-example ("precedent") retrieval
//! (M8.2).
//!
//! Mirrors [`crate::file_candidates`]: an isolated database/graph reusing the
//! memory machinery (embedding, recall, graph registration) via
//! [`MemoryWriter::with_database`], rather than overloading [`crate::types::MemoryType`]
//! on the live `memory_nodes` table — which would need a new default-recall
//! exclusion and would break every exhaustive match on that enum. Activities
//! are immutable, write-once raw captures: no bi-temporal columns to manage,
//! no status/importance semantics, no memory-type taxonomy.
//!
//! A dreaming/extraction pass captures the raw input window as one Activity,
//! then links each memory it produces back to it with a `DERIVED_FROM` edge
//! (in the *memory* graph, via `MemoryWriter::link` with `target_namespace =
//! `[`ACTIVITIES_NAMESPACE`]`). Recall follows the same edge in reverse to
//! attach "we saw this input before" as a precedent block.

use serde_json::{json, Value};
use uuid::Uuid;

use crate::rows::preview;
use crate::{MemoryWriter, Result};

/// Database/namespace Activities live in (isolated from the live `memory`
/// graph until/unless something links into it).
pub const ACTIVITIES_DB: &str = "activities";

/// `MemoryWriter::with_database` names the graph after the database, so the
/// cross-graph `target_namespace` for edges pointing at an Activity is always
/// this fixed `"{db}/{graph}"` string.
pub const ACTIVITIES_NAMESPACE: &str = "activities/activities";

/// One raw capture — the extraction window that produced zero or more memories.
#[derive(Debug, Clone)]
pub struct RawActivity {
    pub text: String,
    pub realm: String,
    /// Free-form source label, e.g. `"claude-code"`.
    pub source: String,
}

fn activity_row(id: &Uuid, a: &RawActivity, embedding: &[f32], now: &str) -> Value {
    json!({
        "id": activity_node_id(id),
        "labels": "Activity",
        "realm": a.realm,
        "memory_type": "entity",
        "title": Value::Null,
        "content": a.text,
        "content_preview": preview(&a.text),
        "tags": format!("activity,source:{}", a.source),
        "importance": 0.0,
        "status": "active",
        "source_session_id": Value::Null,
        "source_run_id": Value::Null,
        "embedding": embedding,
        "created_at": now,
        "updated_at": now,
        "valid_at": now,
        "invalid_at": Value::Null,
        "superseded_by": Value::Null,
        "provenance": json!({"source": a.source}).to_string(),
        "topic_key": Value::Null,
    })
}

/// The node id string for an activity uuid.
pub fn activity_node_id(id: &Uuid) -> String {
    format!("activity:{id}")
}

/// Embed + persist one raw activity. `writer` must already be scoped to
/// [`ACTIVITIES_DB`] via [`MemoryWriter::with_database`]. Returns the new
/// activity's bare uuid (the node id is [`activity_node_id`] of it).
pub async fn capture(writer: &MemoryWriter, a: &RawActivity) -> Result<Uuid> {
    debug_assert_eq!(
        writer.database(),
        ACTIVITIES_DB,
        "writer must be scoped to the activities database"
    );
    writer.ensure_provisioned().await?;
    let emb = writer.embed_one(&a.text).await?;
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let row = activity_row(&id, a, &emb, &now);
    writer.append_node_rows(vec![row]).await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_node_id_is_prefixed() {
        let id = Uuid::nil();
        assert_eq!(
            activity_node_id(&id),
            "activity:00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn activity_row_carries_embedding_and_preview() {
        let id = Uuid::nil();
        let a = RawActivity {
            text: "x".repeat(1000),
            realm: "proj".into(),
            source: "claude-code".into(),
        };
        let row = activity_row(&id, &a, &[0.5, 0.25], "2026-07-01T00:00:00Z");
        assert_eq!(row["id"], json!(activity_node_id(&id)));
        assert_eq!(row["realm"], json!("proj"));
        assert_eq!(row["embedding"], json!([0.5, 0.25]));
        assert_eq!(row["tags"], json!("activity,source:claude-code"));
        assert!(row["content_preview"].as_str().unwrap().ends_with('…'));
    }
}
