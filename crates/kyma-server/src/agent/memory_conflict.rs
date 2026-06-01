//! Conflict resolution + bi-temporal apply (Mem0 A.U.D.N. + Zep/Graphiti).
//!
//! Each extracted candidate memory is reconciled against the most
//! semantically-similar existing memories: the model picks
//! ADD / UPDATE / NOOP / INVALIDATE and we apply it append-only. Contradicted
//! memories are **invalidated** (their `invalid_at` set + an `INVALIDATES`
//! edge to the replacement), never deleted — so history and audit survive and
//! point-in-time recall stays correct.

use serde_json::{json, Value};

use kyma_memory::types::{CreateMemory, MemoryType, RecallFilter};
use kyma_memory::{rows, MemoryWriter, DEFAULT_DATABASE, EDGE_INVALIDATES, NODE_TABLE};

use super::memory_extract::{decide_conflict, ConflictOp, ExtractedMemory};
use super::memory_tools::fetch_latest_node;
use super::state::AgentState;
use super::tools::{execute_sql, SharedToolCtx};

/// Per-run counts of the A.U.D.N. decisions applied.
#[derive(Debug, Default, Clone)]
pub struct ConflictTally {
    pub added: i64,
    pub updated: i64,
    pub noop: i64,
    pub invalidated: i64,
}

impl ConflictTally {
    pub fn written(&self) -> i64 {
        self.added + self.updated
    }
    pub fn merge(&mut self, other: &ConflictTally) {
        self.added += other.added;
        self.updated += other.updated;
        self.noop += other.noop;
        self.invalidated += other.invalidated;
    }
    pub fn to_json(&self) -> Value {
        json!({
            "added": self.added,
            "updated": self.updated,
            "noop": self.noop,
            "invalidated": self.invalidated,
        })
    }
}

/// Number of nearest neighbours shown to the conflict-decision model.
const NEIGHBOURS: usize = 5;

/// Reconcile one candidate memory into the store. `references` are entity node
/// ids the memory is about (become `REFERENCES` edges); `provenance` records
/// how it was formed. Never errors out the pipeline — failures degrade to a
/// no-op for that candidate and are logged.
pub async fn consolidate_memory(
    state: &AgentState,
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    m: &ExtractedMemory,
    references: Vec<String>,
    provenance: Value,
) -> ConflictTally {
    let mut tally = ConflictTally::default();
    let content = m.content.trim();
    if content.is_empty() {
        return tally;
    }
    let kind = MemoryType::parse(&m.kind);

    // 1. Find nearest existing neighbours (same realm + kind) for the decision.
    let similar = nearest(shared, writer, realm, kind, content).await;

    // 2. Decide. On any model/parse failure, default to ADD so we never drop
    //    genuinely new information.
    let decision = match decide_conflict(state, content, &similar).await {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, "conflict decision failed; defaulting to ADD");
            super::memory_extract::ConflictDecision {
                op: ConflictOp::Add,
                target_id: None,
                merged_content: None,
                reason: Some("decision error".into()),
            }
        }
    };

    match decision.op {
        ConflictOp::Noop => {
            tally.noop += 1;
        }
        ConflictOp::Add => {
            if add_memory(writer, realm, m, kind, content, references, provenance)
                .await
                .is_some()
            {
                tally.added += 1;
            }
        }
        ConflictOp::Update => match decision.target_id.as_deref() {
            Some(target) => {
                let new_content = decision.merged_content.as_deref().unwrap_or(content);
                if update_memory(shared, writer, target, new_content).await {
                    tally.updated += 1;
                } else if add_memory(writer, realm, m, kind, content, references, provenance)
                    .await
                    .is_some()
                {
                    tally.added += 1; // target vanished → fall back to ADD
                }
            }
            None => {
                if add_memory(writer, realm, m, kind, content, references, provenance)
                    .await
                    .is_some()
                {
                    tally.added += 1;
                }
            }
        },
        ConflictOp::Invalidate => {
            // Add the replacement first so we can record `superseded_by`.
            match add_memory(writer, realm, m, kind, content, references, provenance).await {
                Some(new_node_id) => {
                    tally.added += 1;
                    if let Some(target) = decision.target_id.as_deref() {
                        if invalidate_memory(shared, writer, realm, target, &new_node_id).await {
                            tally.invalidated += 1;
                        }
                    }
                }
                None => {}
            }
        }
    }
    tally
}

/// Embed the candidate and recall the nearest same-kind memories in realm.
async fn nearest(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    kind: MemoryType,
    content: &str,
) -> Vec<(String, String)> {
    let qvec = match writer.embed_one(content).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let filter = RecallFilter {
        realms: vec![realm.to_string()],
        memory_type: Some(kind),
        ..Default::default()
    };
    let sql = kyma_memory::sql::recall_sql(NODE_TABLE, &qvec, &filter, NEIGHBOURS, None);
    let res = execute_sql(shared, DEFAULT_DATABASE, &sql, NEIGHBOURS).await;
    res.get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    let id = r.get("id").and_then(Value::as_str)?;
                    let c = r
                        .get("content")
                        .or_else(|| r.get("content_preview"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    Some((id.to_string(), c.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Persist a brand-new memory + its `REFERENCES` edges. Returns the node id.
async fn add_memory(
    writer: &MemoryWriter,
    realm: &str,
    m: &ExtractedMemory,
    kind: MemoryType,
    content: &str,
    references: Vec<String>,
    provenance: Value,
) -> Option<String> {
    let mut cm = CreateMemory::new(content);
    cm.title = m.title.clone();
    cm.memory_type = kind;
    cm.realm = realm.to_string();
    cm.importance = m.importance.clamp(0.0, 1.0);
    cm.references = references;
    cm.valid_at = m.valid_at.clone();
    cm.provenance = Some(provenance);
    cm.tags = vec!["source:extraction".to_string()];
    match writer.save(&cm).await {
        Ok(id) => Some(rows::node_id(&id)),
        Err(e) => {
            tracing::debug!(error = %e, "add_memory failed");
            None
        }
    }
}

/// Rewrite an existing memory in place (new version, same id): replace content,
/// re-embed, refresh `updated_at`/`valid_at`. Latest-wins makes it authoritative.
async fn update_memory(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    target_node_id: &str,
    new_content: &str,
) -> bool {
    let mut row = match fetch_latest_node(shared, target_node_id).await {
        Ok(r) => r,
        Err(_) => return false,
    };
    let emb = match writer.embed_one(new_content).await {
        Ok(v) => v,
        Err(_) => return false,
    };
    let now = now_rfc3339();
    row["content"] = json!(new_content);
    row["content_preview"] = json!(rows::preview(new_content));
    row["embedding"] = json!(emb);
    row["updated_at"] = json!(now);
    row["valid_at"] = json!(now);
    writer.append_node_rows(vec![row]).await.is_ok()
}

/// Mark an existing memory invalidated (new version with `invalid_at` +
/// `superseded_by` set) and record an `INVALIDATES` edge from its replacement.
async fn invalidate_memory(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    target_node_id: &str,
    replacement_node_id: &str,
) -> bool {
    let mut row = match fetch_latest_node(shared, target_node_id).await {
        Ok(r) => r,
        Err(_) => return false,
    };
    let now = now_rfc3339();
    row["invalid_at"] = json!(now);
    row["superseded_by"] = json!(replacement_node_id);
    row["updated_at"] = json!(now);
    if writer.append_node_rows(vec![row]).await.is_err() {
        return false;
    }
    let _ = writer
        .link(replacement_node_id, target_node_id, EDGE_INVALIDATES, realm, None)
        .await;
    true
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
