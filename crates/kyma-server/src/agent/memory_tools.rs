//! Agentic Memory tools wired into the agent. These let the Ask Kyma agent
//! (and the future ingestion/dream agent) persist and recall durable memories.
//! Memory lives in the columnar `memory` graph; recall reuses the platform
//! `cosine_distance` UDF via [`super::tools::execute_sql`].

use std::sync::Arc;

use adk_rust::tool::FunctionTool;
use adk_rust::{Tool, ToolContext};
use kyma_memory::types::{MemoryStatus, MemoryType, RecallFilter};
use kyma_memory::{CreateMemory, MemoryWriter, DEFAULT_DATABASE, DEFAULT_REALM, NODE_TABLE};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::memory_retrieve::{retrieve, RetrieveRequest};
use super::tools::{execute_sql, SharedToolCtx};

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Normalize a memory reference (uuid or `memory:<uuid>`) to a node id.
fn node_id_of(s: &str) -> String {
    if s.starts_with("memory:") {
        s.to_string()
    } else {
        format!("memory:{s}")
    }
}

/// Build a [`MemoryWriter`] from the shared tool context + the process-wide
/// embedding backend. Returns a JSON error payload on failure.
async fn build_writer(shared: &SharedToolCtx) -> std::result::Result<MemoryWriter, Value> {
    let embed = kyma_memory::shared_embedding()
        .await
        .map_err(|e| json!({"error": format!("embedding backend: {e}")}))?;
    Ok(MemoryWriter::new(
        shared.catalog.clone(),
        shared.format.clone(),
        embed,
    ))
}

/// Find the latest memory node id for a `(realm, topic_key)`, if any.
/// Backs the topic-key upsert path in `save_memory`.
async fn find_by_topic_key(shared: &SharedToolCtx, realm: &str, topic_key: &str) -> Option<String> {
    let q = format!(
        "WITH latest AS (SELECT id, realm, topic_key, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT id FROM latest WHERE rn = 1 AND topic_key = {tk} AND realm = {r} LIMIT 1",
        nt = NODE_TABLE,
        tk = kyma_memory::sql::sql_str(topic_key),
        r = kyma_memory::sql::sql_str(realm),
    );
    let res = execute_sql(shared, DEFAULT_DATABASE, &q, 1).await;
    res.get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// save_memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SaveMemoryArgs {
    /// The memory content — a self-contained, durable fact/decision/preference/learning.
    content: String,
    /// Short descriptive title (optional).
    #[serde(default)]
    title: Option<String>,
    /// One of: fact, decision, preference, learning, summary. Defaults to fact.
    #[serde(default)]
    memory_type: Option<String>,
    /// Free-form tags.
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// Namespace/realm (e.g. a project name, or "global" for cross-project facts).
    #[serde(default)]
    realm: Option<String>,
    /// Importance 0.0–1.0 (higher = surfaced first). Defaults to 0.5.
    #[serde(default)]
    importance: Option<f32>,
    /// Graph node ids this memory is about — creates REFERENCES edges.
    #[serde(default)]
    references: Option<Vec<String>>,
    /// Stable upsert key (e.g. "architecture/auth-model"). When set, a later
    /// save with the same realm + topic_key UPDATES this memory in place
    /// (bumping its revision) instead of creating a duplicate.
    #[serde(default)]
    topic_key: Option<String>,
    /// Optional structured context, folded into the memory body so it stays
    /// searchable: why the decision/fact holds, where it applies, and what was
    /// learned. (engram's What/Why/Where/Learned shape.)
    #[serde(default)]
    why: Option<String>,
    #[serde(default, rename = "where")]
    where_: Option<String>,
    #[serde(default)]
    learned: Option<String>,
}

/// Append a `Label: value` line to `content` when `val` is non-empty.
fn append_field(content: &mut String, label: &str, val: &Option<String>) {
    if let Some(v) = val.as_deref() {
        let v = v.trim();
        if !v.is_empty() {
            content.push_str(&format!("\n{label}: {v}"));
        }
    }
}

const SAVE_MEMORY_DESC: &str = "Persist a durable memory (fact, decision, \
preference, learning, or summary) so it can be recalled in later sessions. \
Optionally link it to graph entities it's about via `references` (node ids). \
Pass a stable `topic_key` (e.g. \"architecture/auth-model\") to upsert — a \
later save with the same realm+topic_key updates the memory in place instead \
of duplicating. Use this when the user states something worth remembering.";

pub fn tool_save_memory(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "save_memory",
            SAVE_MEMORY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: SaveMemoryArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let mut content = parsed.content;
                    append_field(&mut content, "Why", &parsed.why);
                    append_field(&mut content, "Where", &parsed.where_);
                    append_field(&mut content, "Learned", &parsed.learned);
                    let mut cm = CreateMemory::new(content);
                    cm.title = parsed.title;
                    cm.memory_type = parsed
                        .memory_type
                        .as_deref()
                        .map(MemoryType::parse)
                        .unwrap_or_default();
                    cm.tags = parsed.tags.unwrap_or_default();
                    cm.realm = parsed.realm.unwrap_or_else(|| DEFAULT_REALM.to_string());
                    cm.importance = parsed.importance.unwrap_or(0.5).clamp(0.0, 1.0);
                    cm.references = parsed.references.unwrap_or_default();
                    cm.topic_key = parsed.topic_key.clone().filter(|s| !s.trim().is_empty());

                    // Topic-key upsert: if a memory with the same (realm, topic_key)
                    // exists, update it in place (latest-wins) instead of duplicating.
                    // Provision first so the topic_key column exists for the lookup.
                    if cm.topic_key.is_some() {
                        let _ = writer.ensure_provisioned().await;
                    }
                    if let Some(tk) = cm.topic_key.as_deref() {
                        if let Some(existing) = find_by_topic_key(&shared, &cm.realm, tk).await {
                            let uuid_part = existing.strip_prefix("memory:").unwrap_or(&existing);
                            if let Ok(u) = uuid::Uuid::parse_str(uuid_part) {
                                return Ok(match writer.save_as(u, &cm).await {
                                    Ok(()) => json!({
                                        "saved": true, "upserted": true,
                                        "id": u.to_string(), "node_id": existing,
                                        "topic_key": tk,
                                    }),
                                    Err(e) => json!({"error": format!("upsert: {e}")}),
                                });
                            }
                        }
                    }
                    match writer.save(&cm).await {
                        Ok(id) => Ok(json!({
                            "saved": true,
                            "id": id.to_string(),
                            "node_id": format!("memory:{id}"),
                        })),
                        Err(e) => Ok(json!({"error": format!("save_memory: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<SaveMemoryArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// recall_memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RecallMemoryArgs {
    /// Natural-language query to match memories against semantically.
    query: String,
    /// Max memories to return (default 8).
    #[serde(default)]
    limit: Option<usize>,
    /// Realms to search (e.g. the current project plus "global"). Empty = all.
    #[serde(default)]
    realms: Option<Vec<String>>,
    /// Restrict to one memory type (fact/decision/preference/learning/summary).
    #[serde(default)]
    memory_type: Option<String>,
    /// Minimum importance threshold.
    #[serde(default)]
    importance_min: Option<f32>,
    /// Require these tags (substring match).
    #[serde(default)]
    tags: Option<Vec<String>>,
}

const RECALL_MEMORY_DESC: &str = "Recall the most relevant stored memories for \
a query using graph-aware hybrid search (semantic + keyword), expanded over \
connected memories and resources. Call this before answering questions that \
may depend on prior context, preferences, or past decisions. Returns ranked \
memories, connected resources, and a ready-to-use context block.";

pub fn tool_recall_memory(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "recall_memory",
            RECALL_MEMORY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: RecallMemoryArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let req = RetrieveRequest {
                        query: parsed.query,
                        realms: parsed.realms.unwrap_or_default(),
                        memory_type: parsed.memory_type,
                        tags: parsed.tags.unwrap_or_default(),
                        importance_min: parsed.importance_min,
                        as_of: None,
                        include_invalidated: false,
                        limit: parsed.limit,
                        expand_hops: Some(1),
                    };
                    Ok(retrieve(&shared, &req).await.to_json())
                }
            },
        )
        .with_parameters_schema::<RecallMemoryArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// memory_search — the primary "find anything fast" tool for coding agents
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MemorySearchArgs {
    /// Natural-language query.
    query: String,
    /// Max memories to return (default 8).
    #[serde(default)]
    limit: Option<usize>,
    /// Realms to search (e.g. the current project plus "global"). Empty = all.
    #[serde(default)]
    realms: Option<Vec<String>>,
    /// Restrict to one memory type (fact/decision/preference/learning/procedure/summary/entity).
    #[serde(default)]
    memory_type: Option<String>,
    /// Require these tags (substring match).
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// Minimum importance threshold.
    #[serde(default)]
    importance_min: Option<f32>,
    /// Graph-expansion hops (0–2, default 1): pulls in connected memories and
    /// linked catalog resources/traces for contextual understanding.
    #[serde(default)]
    expand_hops: Option<u8>,
    /// Point-in-time recall (RFC3339): only memories valid at this instant.
    #[serde(default)]
    as_of: Option<String>,
}

const MEMORY_SEARCH_DESC: &str = "Find anything fast across the agent's memory: \
hybrid semantic + keyword search, graph-expanded over connected memories, \
catalog resources, and distributed traces. Returns ranked memories with \
validity intervals, the connecting graph paths, and a ready-to-use context \
block. Use this FIRST when a question may depend on prior context, decisions, \
preferences, or how entities/resources relate. Follow `linked` node ids with \
graph_traverse for a deeper subgraph.";

pub fn tool_memory_search(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "memory_search",
            MEMORY_SEARCH_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: MemorySearchArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let req = RetrieveRequest {
                        query: parsed.query,
                        realms: parsed.realms.unwrap_or_default(),
                        memory_type: parsed.memory_type,
                        tags: parsed.tags.unwrap_or_default(),
                        importance_min: parsed.importance_min,
                        as_of: parsed.as_of,
                        include_invalidated: false,
                        limit: parsed.limit,
                        expand_hops: parsed.expand_hops,
                    };
                    Ok(retrieve(&shared, &req).await.to_json())
                }
            },
        )
        .with_parameters_schema::<MemorySearchArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// list_memories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ListMemoriesArgs {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    memory_type: Option<String>,
    /// active | background | archived.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    realm: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

const LIST_MEMORIES_DESC: &str = "List stored memories with optional filters \
(type, status, realm, tags), newest first. Use for browsing/auditing memory \
rather than semantic search.";

pub fn tool_list_memories(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "list_memories",
            LIST_MEMORIES_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: ListMemoriesArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    if let Err(e) = writer.ensure_provisioned().await {
                        return Ok(json!({"error": format!("provision: {e}")}));
                    }
                    let limit = parsed.limit.unwrap_or(50).clamp(1, 500);
                    let offset = parsed.offset.unwrap_or(0);
                    let statuses = parsed
                        .status
                        .as_deref()
                        .and_then(MemoryStatus::parse)
                        .map(|s| vec![s])
                        .unwrap_or_default();
                    let filter = RecallFilter {
                        realms: parsed.realm.map(|r| vec![r]).unwrap_or_default(),
                        memory_type: parsed.memory_type.as_deref().map(MemoryType::parse),
                        statuses,
                        tags: parsed.tags.unwrap_or_default(),
                        importance_min: None,
                        since: None,
                        until: None,
                        ..Default::default()
                    };
                    let sql = kyma_memory::sql::list_sql(NODE_TABLE, &filter, limit, offset);
                    Ok(execute_sql(&shared, DEFAULT_DATABASE, &sql, limit).await)
                }
            },
        )
        .with_parameters_schema::<ListMemoriesArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// link_memory_to_entity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct LinkMemoryArgs {
    /// The memory (uuid or `memory:<uuid>`).
    memory_id: String,
    /// The target graph node id to link to (e.g. a repo/service/table node).
    target_node_id: String,
    /// Relationship type. Default "REFERENCES".
    #[serde(default)]
    relationship_type: Option<String>,
    /// The target endpoint's `database/graph` namespace (for cross-graph edges).
    #[serde(default)]
    target_namespace: Option<String>,
    /// Realm for the edge. Default "default".
    #[serde(default)]
    realm: Option<String>,
}

const LINK_MEMORY_DESC: &str = "Link a memory to an existing graph entity \
(repo, service, table, user, …) by node id, creating a REFERENCES edge. Use \
after recall/graph search to connect a memory to what it's about.";

pub fn tool_link_memory_to_entity(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "link_memory_to_entity",
            LINK_MEMORY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: LinkMemoryArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let src = node_id_of(&parsed.memory_id);
                    let rel = parsed
                        .relationship_type
                        .unwrap_or_else(|| "REFERENCES".to_string());
                    let realm = parsed.realm.unwrap_or_else(|| DEFAULT_REALM.to_string());
                    match writer
                        .link(
                            &src,
                            &parsed.target_node_id,
                            &rel,
                            &realm,
                            parsed.target_namespace.as_deref(),
                        )
                        .await
                    {
                        Ok(()) => Ok(json!({
                            "linked": true,
                            "src": src,
                            "dst": parsed.target_node_id,
                            "type": rel,
                        })),
                        Err(e) => Ok(json!({"error": format!("link: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<LinkMemoryArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// update_memory_status / update_memory_importance (read latest → append version)
// ---------------------------------------------------------------------------

/// Fetch the latest version of a memory node as a JSON row, or an error payload.
/// Shared with the conflict-resolution pipeline ([`super::memory_conflict`]).
pub(crate) async fn fetch_latest_node(
    shared: &SharedToolCtx,
    node_id: &str,
) -> std::result::Result<Value, Value> {
    let sql = kyma_memory::sql::latest_node_sql(NODE_TABLE, node_id);
    let res = execute_sql(shared, DEFAULT_DATABASE, &sql, 1).await;
    if let Some(err) = res.get("error") {
        return Err(json!({"error": format!("fetch: {err}")}));
    }
    let row = res
        .get("rows")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .cloned();
    match row {
        Some(r) => Ok(r),
        None => Err(json!({"error": "memory not found"})),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UpdateStatusArgs {
    memory_id: String,
    /// active | background | archived.
    status: String,
}

const UPDATE_STATUS_DESC: &str = "Change a memory's lifecycle status \
(active/background/archived). Archived memories are hidden from recall. Use \
during housekeeping to retire stale memories without deleting them.";

pub fn tool_update_memory_status(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "update_memory_status",
            UPDATE_STATUS_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: UpdateStatusArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let status = match MemoryStatus::parse(&parsed.status) {
                        Some(s) => s,
                        None => return Ok(json!({"error": "status must be active|background|archived"})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let node_id = node_id_of(&parsed.memory_id);
                    let mut row = match fetch_latest_node(&shared, &node_id).await {
                        Ok(r) => r,
                        Err(e) => return Ok(e),
                    };
                    row["status"] = json!(status.as_str());
                    row["updated_at"] = json!(now_rfc3339());
                    match writer.append_node_rows(vec![row]).await {
                        Ok(()) => Ok(json!({"ok": true, "memory_id": node_id, "status": status.as_str()})),
                        Err(e) => Ok(json!({"error": format!("update_status: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<UpdateStatusArgs>()
        .with_read_only(false),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct UpdateImportanceArgs {
    memory_id: String,
    /// New importance 0.0–1.0.
    importance: f32,
}

const UPDATE_IMPORTANCE_DESC: &str = "Set a memory's importance (0.0–1.0). \
Higher importance surfaces a memory earlier in recall. Use during housekeeping \
to re-weight memories.";

pub fn tool_update_memory_importance(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "update_memory_importance",
            UPDATE_IMPORTANCE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: UpdateImportanceArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let importance = parsed.importance.clamp(0.0, 1.0) as f64;
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let node_id = node_id_of(&parsed.memory_id);
                    let mut row = match fetch_latest_node(&shared, &node_id).await {
                        Ok(r) => r,
                        Err(e) => return Ok(e),
                    };
                    row["importance"] = json!(importance);
                    row["updated_at"] = json!(now_rfc3339());
                    match writer.append_node_rows(vec![row]).await {
                        Ok(()) => Ok(json!({"ok": true, "memory_id": node_id, "importance": importance})),
                        Err(e) => Ok(json!({"error": format!("update_importance: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<UpdateImportanceArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// merge_memories
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct MergeMemoriesArgs {
    /// The memory to keep (uuid or node id).
    into_id: String,
    /// Memories to archive and fold into `into_id`.
    from_ids: Vec<String>,
}

const MERGE_MEMORIES_DESC: &str = "Consolidate duplicate/overlapping memories: \
archive each `from` memory and record a MERGED_INTO edge to the kept memory. \
Use during housekeeping to deduplicate.";

pub fn tool_merge_memories(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "merge_memories",
            MERGE_MEMORIES_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: MergeMemoriesArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let into = node_id_of(&parsed.into_id);
                    let now = now_rfc3339();
                    let mut merged: Vec<String> = Vec::new();
                    for from in &parsed.from_ids {
                        let from_id = node_id_of(from);
                        if from_id == into {
                            continue;
                        }
                        // Archive the source (new version).
                        if let Ok(mut row) = fetch_latest_node(&shared, &from_id).await {
                            row["status"] = json!("archived");
                            row["updated_at"] = json!(now);
                            let _ = writer.append_node_rows(vec![row]).await;
                        }
                        // Record the merge edge.
                        let _ = writer
                            .link(&from_id, &into, "MERGED_INTO", DEFAULT_REALM, None)
                            .await;
                        merged.push(from_id);
                    }
                    Ok(json!({"ok": true, "into": into, "merged": merged}))
                }
            },
        )
        .with_parameters_schema::<MergeMemoriesArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// memory_compare / memory_judge — agent-driven conflict resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct CompareArgs {
    /// First memory (uuid or `memory:<uuid>`).
    memory_id: String,
    /// Second memory to compare against.
    other_id: String,
}

const COMPARE_DESC: &str = "Fetch two memories side by side so you can judge \
their relationship, then record it with `memory_judge`. Returns both full \
memory rows (content + metadata).";

pub fn tool_memory_compare(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "memory_compare",
            COMPARE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: CompareArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let a = fetch_latest_node(&shared, &node_id_of(&parsed.memory_id)).await;
                    let b = fetch_latest_node(&shared, &node_id_of(&parsed.other_id)).await;
                    Ok(json!({
                        "a": a.ok(),
                        "b": b.ok(),
                        "hint": "Decide the relationship, then call memory_judge with a verdict: \
                                 supersedes | conflicts | related | compatible | merged.",
                    }))
                }
            },
        )
        .with_parameters_schema::<CompareArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct JudgeArgs {
    /// The memory making the judgment (uuid or `memory:<uuid>`).
    memory_id: String,
    /// The other memory the verdict is about.
    target_id: String,
    /// One of: supersedes | conflicts | related | compatible | merged.
    verdict: String,
    /// Optional short rationale (stored on the edge).
    #[serde(default)]
    reason: Option<String>,
}

const JUDGE_DESC: &str = "Record a conflict/relationship verdict between two \
memories as a graph edge. `supersedes` invalidates the target (bi-temporal) + \
writes an INVALIDATES edge; `merged` archives the target + writes MERGED_INTO; \
`conflicts`/`related`/`compatible` write a RELATES_TO edge (verdict in props). \
Use after `memory_compare` or when recall surfaces a contradiction.";

pub fn tool_memory_judge(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "memory_judge",
            JUDGE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: JudgeArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let src = node_id_of(&parsed.memory_id);
                    let dst = node_id_of(&parsed.target_id);
                    let verdict = parsed.verdict.trim().to_ascii_lowercase();
                    let now = now_rfc3339();

                    // Realm follows the target memory when we can read it.
                    let target_row = fetch_latest_node(&shared, &dst).await.ok();
                    let realm = target_row
                        .as_ref()
                        .and_then(|r| r.get("realm"))
                        .and_then(Value::as_str)
                        .unwrap_or(DEFAULT_REALM)
                        .to_string();

                    match verdict.as_str() {
                        "supersedes" | "invalidates" => {
                            let Some(mut row) = target_row else {
                                return Ok(json!({"error": "target memory not found"}));
                            };
                            row["invalid_at"] = json!(now);
                            row["superseded_by"] = json!(src);
                            row["updated_at"] = json!(now);
                            if let Err(e) = writer.append_node_rows(vec![row]).await {
                                return Ok(json!({"error": format!("invalidate: {e}")}));
                            }
                            let _ = writer.link(&src, &dst, "INVALIDATES", &realm, None).await;
                            Ok(json!({"ok": true, "verdict": "supersedes", "src": src, "dst": dst}))
                        }
                        "merged" | "merged_into" => {
                            if let Some(mut row) = target_row {
                                row["status"] = json!("archived");
                                row["updated_at"] = json!(now);
                                let _ = writer.append_node_rows(vec![row]).await;
                            }
                            let _ = writer.link(&dst, &src, "MERGED_INTO", &realm, None).await;
                            Ok(json!({"ok": true, "verdict": "merged", "into": src, "from": dst}))
                        }
                        _ => {
                            let props = json!({ "verdict": verdict, "reason": parsed.reason });
                            let edge = kyma_memory::rows::edge_row(
                                &src, &dst, "RELATES_TO", &realm, None, Some(&props), &now,
                            );
                            if let Err(e) = writer.append_edge_rows(vec![edge]).await {
                                return Ok(json!({"error": format!("relate: {e}")}));
                            }
                            Ok(json!({"ok": true, "verdict": verdict, "src": src, "dst": dst}))
                        }
                    }
                }
            },
        )
        .with_parameters_schema::<JudgeArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// memory_session_summary — structured end-of-session capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SessionSummaryArgs {
    /// What the session worked on.
    #[serde(default)]
    goal: Option<String>,
    /// User preferences / instructions discovered.
    #[serde(default)]
    instructions: Option<String>,
    /// Technical findings / discoveries.
    #[serde(default)]
    discoveries: Option<String>,
    /// What was accomplished.
    #[serde(default)]
    accomplished: Option<String>,
    /// Remaining work / next steps.
    #[serde(default)]
    next_steps: Option<String>,
    /// Relevant files touched.
    #[serde(default)]
    files: Option<String>,
    /// Memory namespace (project). Defaults to "default".
    #[serde(default)]
    realm: Option<String>,
}

const SESSION_SUMMARY_DESC: &str = "Save a structured end-of-session summary \
(goal, instructions, discoveries, accomplished, next steps, files) as a durable \
`summary` memory so the next session resumes with context. Call this when \
wrapping up a work session.";

pub fn tool_memory_session_summary(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "memory_session_summary",
            SESSION_SUMMARY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: SessionSummaryArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let mut content = String::from("Session summary.");
                    append_field(&mut content, "Goal", &parsed.goal);
                    append_field(&mut content, "Instructions", &parsed.instructions);
                    append_field(&mut content, "Discoveries", &parsed.discoveries);
                    append_field(&mut content, "Accomplished", &parsed.accomplished);
                    append_field(&mut content, "Next steps", &parsed.next_steps);
                    append_field(&mut content, "Files", &parsed.files);
                    if content == "Session summary." {
                        return Ok(json!({"error": "nothing to summarize — provide at least one field"}));
                    }
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    let mut cm = CreateMemory::new(content);
                    cm.title = Some("Session summary".to_string());
                    cm.memory_type = MemoryType::Summary;
                    cm.realm = parsed.realm.unwrap_or_else(|| DEFAULT_REALM.to_string());
                    cm.importance = 0.6;
                    cm.tags = vec!["session-summary".to_string()];
                    match writer.save(&cm).await {
                        Ok(id) => Ok(json!({"saved": true, "id": id.to_string(), "node_id": format!("memory:{id}")})),
                        Err(e) => Ok(json!({"error": format!("session_summary: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<SessionSummaryArgs>()
        .with_read_only(false),
    )
}
