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

use super::memory_gate::{self, OpPayload};
use super::memory_policy::MemoryOp;
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

/// The realm a fetched node row belongs to (queue partition for mutations).
fn row_realm(row: &Value) -> String {
    row.get("realm")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_REALM)
        .to_string()
}

/// Build a [`MemoryWriter`] from the shared tool context + the process-wide
/// embedding backend. Returns a JSON error payload on failure.
pub(crate) async fn build_writer(shared: &SharedToolCtx) -> std::result::Result<MemoryWriter, Value> {
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
    /// Wait for the memory to be fully committed before returning. Default
    /// false: the save is acked immediately and batched in the background.
    #[serde(default)]
    sync: Option<bool>,
    /// Visibility scope (S3.3). Omit or `"public"` to share; `"private:<agent>"`
    /// restricts recall to that agent. Recall honors this via `space_agent`.
    #[serde(default)]
    space: Option<String>,
}

/// Fold a [`SaveMemoryArgs`] into the [`CreateMemory`] the writer/queue takes.
fn create_from_save_args(parsed: SaveMemoryArgs) -> CreateMemory {
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
    cm.topic_key = parsed.topic_key.filter(|s| !s.trim().is_empty());
    cm.space = parsed.space.filter(|s| !s.trim().is_empty());
    // Attribute the writing client (S3.3) for provenance + space scoping.
    cm.writer_agent_id = super::identity::client_name();
    // Stamp who is writing (host / transport / MCP client) into provenance.
    super::identity::stamp_provenance(&mut cm);
    cm
}

/// Resolve the topic-key upsert target for `cm`, if any. Issues a realm
/// barrier first when the async queue is active so the lookup observes
/// in-flight same-key saves (keeps latest-wins correct under batching).
async fn resolve_upsert_target(shared: &SharedToolCtx, cm: &CreateMemory) -> Option<uuid::Uuid> {
    let tk = cm.topic_key.as_deref()?;
    shared.memory_barrier(std::slice::from_ref(&cm.realm)).await;
    let existing = find_by_topic_key(shared, &cm.realm, tk).await?;
    let uuid_part = existing.strip_prefix("memory:").unwrap_or(&existing);
    uuid::Uuid::parse_str(uuid_part).ok()
}

/// Try the async enqueue path for a save. Returns the response payload on
/// success; `None` means "use the synchronous path" (sync requested, no queue,
/// or the queue refused — backpressure/shutdown).
async fn try_queue_save(
    shared: &SharedToolCtx,
    cm: &CreateMemory,
    upsert: Option<uuid::Uuid>,
) -> Option<Value> {
    let q = shared.memory.as_ref()?;
    let res = match upsert {
        Some(u) => q.submit_save_as(u, cm, true).await.map(|()| u),
        None => q.submit_create(cm, true).await,
    };
    match res {
        Ok(id) => {
            let mut out = json!({
                "saved": true,
                "queued": true,
                "id": id.to_string(),
                "node_id": format!("memory:{id}"),
            });
            if upsert.is_some() {
                out["upserted"] = json!(true);
                if let Some(tk) = cm.topic_key.as_deref() {
                    out["topic_key"] = json!(tk);
                }
            }
            Some(out)
        }
        Err(e) => {
            tracing::warn!(error = %e, "memory queue rejected save; falling back to synchronous path");
            None
        }
    }
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
                    let sync = parsed.sync.unwrap_or(false);
                    let cm = create_from_save_args(parsed);

                    // Async-by-default: resolve the upsert target (behind a
                    // realm barrier), enqueue, and return the final id
                    // immediately. Falls back to the synchronous path on
                    // `sync: true` or queue refusal.
                    if !sync && shared.memory.is_some() {
                        let upsert = resolve_upsert_target(&shared, &cm).await;
                        if let Some(out) = try_queue_save(&shared, &cm, upsert).await {
                            return Ok(out);
                        }
                    }

                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
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
// ingest_entity — dynamic virtual resources on the graph
// ---------------------------------------------------------------------------

/// One edge wiring an entity to another node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct EntityLink {
    /// Node id to wire to. For a catalog resource (e.g. a data-source-ingested
    /// repo/service/table) pass its graph node id (e.g. "repo:owner/name") plus
    /// `target_namespace`. For a memory/entity pass "memory:<uuid>".
    target_node_id: String,
    /// Relationship type. Default "RELATES_TO" (e.g. OWNS, DEPENDS_ON, PART_OF).
    #[serde(default)]
    relationship_type: Option<String>,
    /// The target's `database/graph` namespace for cross-graph edges to
    /// data source resources (e.g. "github"). Omit when the target is a memory.
    #[serde(default)]
    target_namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct IngestEntityArgs {
    /// Entity name / title (e.g. "payments service", "ledger table").
    name: String,
    /// Kind: service | repo | table | person | file | config | concept | … —
    /// folded into the body and the upsert key.
    #[serde(default)]
    kind: Option<String>,
    /// Arbitrary properties (object); folded into the body so the entity stays
    /// searchable (e.g. {"language":"rust","owner":"team-pay"}).
    #[serde(default)]
    properties: Option<Value>,
    /// Namespace/realm (usually the project). Defaults to "default".
    #[serde(default)]
    realm: Option<String>,
    /// Edges wiring this entity to existing graph nodes / memories / siblings.
    #[serde(default)]
    links: Option<Vec<EntityLink>>,
    /// Icon name from the gallery (e.g. "github", "datadog", "kubernetes",
    /// "service", "database", "person"). Optional — when omitted the server
    /// derives one from `type`/`kind`/vendor via the icon gallery so the graph
    /// shows a recognisable mark.
    #[serde(default)]
    icon: Option<String>,
    /// Classification type `provider::resource` (e.g. "kubernetes::pod",
    /// "aws::ec2::instance", "github::repository", "datadog::monitor"). Drives
    /// the brand mark + resource classification; prefer this over `kind` for
    /// vendor/cloud resources.
    #[serde(default, rename = "type")]
    entity_type: Option<String>,
    /// Wait for the entity + links to be fully committed before returning.
    /// Default false: queued and batched in the background.
    #[serde(default)]
    sync: Option<bool>,
}

/// Lowercase, hyphenated slug for the entity's stable upsert key.
fn slug(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

const INGEST_ENTITY_DESC: &str = "Create (or update) a virtual resource/entity on the \
knowledge graph and wire it to existing graph nodes and memories — enriching the context \
engine with agent-known entities (a service, repo, table, person, file, config, concept) and \
their relationships. `links` connect the entity to catalog resources (set `target_namespace`, \
e.g. \"github\", plus the node id like \"repo:owner/name\") or to memories (\"memory:<uuid>\"). \
Idempotent on (realm, kind, name): re-ingesting the same entity updates it in place. Discover \
real node ids to link to first via find_references_to / graph_traverse / recall_memory. Prefer \
`type` as a `provider::resource` classification (e.g. \"kubernetes::pod\", \"aws::ec2::instance\", \
\"github::repository\", \"datadog::monitor\") — it sets the brand mark + resource class. Or set \
`icon` directly to a gallery name — a brand when the entity is \
vendor-specific (github, gitlab, slack, datadog, kubernetes, docker, grafana, pagerduty, aws, \
gcp, postgresql, prometheus, sentry, redis, mongodb, snowflake, elastic, terraform) or a kind \
glyph otherwise (service, repo, database, table, person, file, config, secret, concept, infra, \
deployment, pod, api, function). If omitted, the server derives one from kind/vendor.";

pub fn tool_ingest_entity(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "ingest_entity",
            INGEST_ENTITY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: IngestEntityArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let name = parsed.name.trim().to_string();
                    if name.is_empty() {
                        return Ok(json!({"error": "name is required"}));
                    }
                    let realm = parsed
                        .realm
                        .clone()
                        .unwrap_or_else(|| DEFAULT_REALM.to_string());
                    let kind: Option<String> = parsed
                        .kind
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);

                    // Fold kind + properties into the content so the entity is
                    // recallable by its attributes.
                    let mut content = name.clone();
                    if let Some(k) = kind.as_deref() {
                        content.push_str(&format!("\nKind: {k}"));
                    }
                    // Vendor/source hint (drives brand-icon resolution).
                    let vendor: Option<String> = parsed
                        .properties
                        .as_ref()
                        .and_then(Value::as_object)
                        .and_then(|o| {
                            ["vendor", "source", "brand", "provider", "source_type"]
                                .iter()
                                .find_map(|k| o.get(*k).and_then(Value::as_str))
                                .map(str::to_string)
                        });
                    if let Some(obj) = parsed.properties.as_ref().and_then(Value::as_object) {
                        for (k, v) in obj {
                            let vs = match v {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            if !vs.trim().is_empty() {
                                content.push_str(&format!("\n{k}: {vs}"));
                            }
                        }
                    }
                    // Classification type `provider::resource` (folded in so
                    // clients can classify + brand the node).
                    let entity_type: Option<String> = parsed
                        .entity_type
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    if let Some(t) = entity_type.as_deref() {
                        content.push_str(&format!("\ntype: {t}"));
                    }

                    // Resolve the entity's icon: explicit `icon` wins, else from
                    // the classification type, else from kind/vendor — all via
                    // the gallery. Folded into the body so every client renders a
                    // recognisable mark.
                    let gallery = crate::icon_config::IconGallery::global();
                    let icon: Option<String> = parsed
                        .icon
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_lowercase)
                        .or_else(|| entity_type.as_deref().and_then(|t| gallery.resolve_type(t)))
                        .or_else(|| gallery.resolve(kind.as_deref(), vendor.as_deref()));
                    if let Some(ic) = icon.as_deref() {
                        content.push_str(&format!("\nicon: {ic}"));
                    }

                    let mut cm = CreateMemory::new(content);
                    cm.title = Some(name.clone());
                    cm.memory_type = MemoryType::Entity;
                    cm.realm = realm.clone();
                    cm.importance = 0.5;
                    // Categorize provenance so housekeeping can tell agent-minted
                    // ("synthetic") nodes from extracted/data-source-derived ones and
                    // score/relevance-check them accordingly.
                    cm.provenance = Some(json!({
                        "source": "synthetic",
                        "via": "ingest_entity",
                        "kind": kind,
                        "type": entity_type.clone(),
                        "icon": icon.clone(),
                    }));
                    super::identity::stamp_provenance(&mut cm);
                    let topic_key = format!(
                        "entity/{}/{}",
                        kind.as_deref().unwrap_or("entity"),
                        slug(&name)
                    );
                    cm.topic_key = Some(topic_key.clone());

                    // Async-by-default: enqueue the entity + its links. FIFO +
                    // node-before-edge ordering makes linking to the
                    // just-queued entity safe within the same flush.
                    let sync = parsed.sync.unwrap_or(false);
                    if !sync && shared.memory.is_some() {
                        let upsert = resolve_upsert_target(&shared, &cm).await;
                        if let Some(mut out) = try_queue_save(&shared, &cm, upsert).await {
                            let q = shared.memory.as_ref().expect("queue checked above");
                            let id = out["id"].as_str().unwrap_or_default().to_string();
                            let src = format!("memory:{id}");
                            let now = now_rfc3339();
                            let mut linked = 0usize;
                            let mut link_errors: Vec<String> = Vec::new();
                            for l in parsed.links.clone().unwrap_or_default() {
                                let dst = if l.target_namespace.is_some() {
                                    l.target_node_id.clone()
                                } else {
                                    node_id_of(&l.target_node_id)
                                };
                                let rel = l
                                    .relationship_type
                                    .unwrap_or_else(|| "RELATES_TO".to_string());
                                let edge = kyma_memory::rows::edge_row(
                                    &src,
                                    &dst,
                                    &rel,
                                    &realm,
                                    l.target_namespace.as_deref(),
                                    None,
                                    &now,
                                );
                                match q.submit_edge_row(&realm, edge, true).await {
                                    Ok(()) => linked += 1,
                                    Err(e) => link_errors.push(format!("{dst}: {e}")),
                                }
                            }
                            out["created"] = json!(true);
                            out["upserted"] = json!(upsert.is_some());
                            out["kind"] = json!(kind);
                            out["type"] = json!(entity_type);
                            out["icon"] = json!(icon);
                            out["topic_key"] = json!(topic_key);
                            out["links"] = json!(linked);
                            if !link_errors.is_empty() {
                                out["link_errors"] = json!(link_errors);
                            }
                            return Ok(out);
                        }
                    }

                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
                    // Upsert by topic key so re-ingesting the same entity updates
                    // it in place rather than duplicating. Provision first so the
                    // topic_key column exists for the lookup.
                    let _ = writer.ensure_provisioned().await;
                    let existing = find_by_topic_key(&shared, &realm, &topic_key).await;
                    let (id, upserted) = match existing.as_deref().and_then(|e| {
                        uuid::Uuid::parse_str(e.strip_prefix("memory:").unwrap_or(e)).ok()
                    }) {
                        Some(u) => match writer.save_as(u, &cm).await {
                            Ok(()) => (u, true),
                            Err(e) => return Ok(json!({"error": format!("upsert: {e}")})),
                        },
                        None => match writer.save(&cm).await {
                            Ok(u) => (u, false),
                            Err(e) => return Ok(json!({"error": format!("ingest_entity: {e}")})),
                        },
                    };

                    // Wire the requested edges. A target with a namespace is a
                    // cross-graph node id as-is; otherwise it's a memory/entity.
                    let src = format!("memory:{id}");
                    let mut linked = 0usize;
                    let mut link_errors: Vec<String> = Vec::new();
                    for l in parsed.links.unwrap_or_default() {
                        let dst = if l.target_namespace.is_some() {
                            l.target_node_id.clone()
                        } else {
                            node_id_of(&l.target_node_id)
                        };
                        let rel = l
                            .relationship_type
                            .unwrap_or_else(|| "RELATES_TO".to_string());
                        match writer
                            .link(&src, &dst, &rel, &realm, l.target_namespace.as_deref())
                            .await
                        {
                            Ok(()) => linked += 1,
                            Err(e) => link_errors.push(format!("{dst}: {e}")),
                        }
                    }

                    let mut out = json!({
                        "created": true,
                        "upserted": upserted,
                        "id": id.to_string(),
                        "node_id": src,
                        "kind": kind,
                        "type": entity_type,
                        "icon": icon,
                        "topic_key": topic_key,
                        "links": linked,
                    });
                    if !link_errors.is_empty() {
                        out["link_errors"] = json!(link_errors);
                    }
                    Ok(out)
                }
            },
        )
        .with_parameters_schema::<IngestEntityArgs>()
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
    /// Agent identity for memory-space visibility (S3.3): returns shared
    /// memories plus this agent's own `private:<agent>`. Omit for shared-only.
    #[serde(default)]
    space_agent: Option<String>,
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
                        space_agent: parsed.space_agent,
                    };
                    // Read-your-own-writes: land queued writes for the target
                    // realms first (bounded; no-op when nothing is pending).
                    shared.memory_barrier(&req.realms).await;
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
    /// Agent identity for memory-space visibility (S3.3): returns shared
    /// memories plus this agent's own `private:<agent>`. Omit for shared-only.
    #[serde(default)]
    space_agent: Option<String>,
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
                        space_agent: parsed.space_agent,
                    };
                    // Read-your-own-writes: land queued writes for the target
                    // realms first (bounded; no-op when nothing is pending).
                    shared.memory_barrier(&req.realms).await;
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
                    // Read-your-own-writes for the listed realm(s).
                    shared.memory_barrier(&filter.realms).await;
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
                    let src = node_id_of(&parsed.memory_id);
                    let rel = parsed
                        .relationship_type
                        .unwrap_or_else(|| "REFERENCES".to_string());
                    let realm = parsed.realm.unwrap_or_else(|| DEFAULT_REALM.to_string());

                    // HITL chokepoint: a cross-graph link is the riskier op
                    // (LinkEntityCrossRealm); a same-graph edge is RelationshipWrite.
                    if shared.hitl.is_some() {
                        let op = if parsed.target_namespace.is_some() {
                            MemoryOp::LinkEntityCrossRealm
                        } else {
                            MemoryOp::RelationshipWrite
                        };
                        let payload = OpPayload::Link {
                            src: src.clone(),
                            dst: parsed.target_node_id.clone(),
                            rel: rel.clone(),
                            realm: realm.clone(),
                            target_namespace: parsed.target_namespace.clone(),
                        };
                        if let Some(out) =
                            memory_gate::gate_tool_op(&shared, op, &realm, None, payload).await
                        {
                            return Ok(out);
                        }
                    }

                    // Async-by-default: queue the edge. FIFO + node-before-edge
                    // makes linking a just-queued memory safe.
                    if let Some(q) = shared.memory.as_ref() {
                        let edge = kyma_memory::rows::edge_row(
                            &src,
                            &parsed.target_node_id,
                            &rel,
                            &realm,
                            parsed.target_namespace.as_deref(),
                            None,
                            &now_rfc3339(),
                        );
                        match q.submit_edge_row(&realm, edge, true).await {
                            Ok(()) => {
                                return Ok(json!({
                                    "linked": true,
                                    "queued": true,
                                    "src": src,
                                    "dst": parsed.target_node_id,
                                    "type": rel,
                                }))
                            }
                            Err(e) => tracing::warn!(error = %e, "memory queue rejected link; falling back to synchronous path"),
                        }
                    }

                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
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
    // Read-modify-write correctness: a mutation must observe the queued
    // writes that precede it (the realm is unknown until the row is read, so
    // barrier across all realms — these are rare curation paths).
    shared.memory_barrier(&[]).await;
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
                    let node_id = node_id_of(&parsed.memory_id);
                    // HITL chokepoint: only archival is a gateable op (retiring a
                    // memory); active/background transitions stay ungated.
                    if shared.hitl.is_some() && parsed.status.eq_ignore_ascii_case("archived") {
                        let payload = OpPayload::Archive {
                            memory_id: node_id.clone(),
                        };
                        if let Some(out) = memory_gate::gate_tool_op(
                            &shared,
                            MemoryOp::Archive,
                            DEFAULT_REALM,
                            None,
                            payload,
                        )
                        .await
                        {
                            return Ok(out);
                        }
                    }
                    // fetch_latest_node barriers internally, so this
                    // read-modify-write sees queued versions of the memory.
                    let mut row = match fetch_latest_node(&shared, &node_id).await {
                        Ok(r) => r,
                        Err(e) => return Ok(e),
                    };
                    row["status"] = json!(status.as_str());
                    row["updated_at"] = json!(now_rfc3339());
                    let realm = row_realm(&row);

                    if let Some(q) = shared.memory.as_ref() {
                        match q.submit_node_row(&realm, row.clone(), true).await {
                            Ok(()) => {
                                return Ok(json!({"ok": true, "queued": true, "memory_id": node_id, "status": status.as_str()}))
                            }
                            Err(e) => tracing::warn!(error = %e, "memory queue rejected update; falling back to synchronous path"),
                        }
                    }
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
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
                    let node_id = node_id_of(&parsed.memory_id);
                    // fetch_latest_node barriers internally (read-modify-write).
                    let mut row = match fetch_latest_node(&shared, &node_id).await {
                        Ok(r) => r,
                        Err(e) => return Ok(e),
                    };
                    row["importance"] = json!(importance);
                    row["updated_at"] = json!(now_rfc3339());
                    let realm = row_realm(&row);

                    if let Some(q) = shared.memory.as_ref() {
                        match q.submit_node_row(&realm, row.clone(), true).await {
                            Ok(()) => {
                                return Ok(json!({"ok": true, "queued": true, "memory_id": node_id, "importance": importance}))
                            }
                            Err(e) => tracing::warn!(error = %e, "memory queue rejected update; falling back to synchronous path"),
                        }
                    }
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
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
                    // HITL chokepoint (autonomous runs only). When a gate is
                    // present the merge routes through it; the ungated path below
                    // is unchanged for the interactive agent.
                    if shared.hitl.is_some() {
                        let into = node_id_of(&parsed.into_id);
                        let from_ids: Vec<String> =
                            parsed.from_ids.iter().map(|f| node_id_of(f)).collect();
                        let payload = OpPayload::Merge {
                            into_id: into,
                            from_ids,
                        };
                        if let Some(out) = memory_gate::gate_tool_op(
                            &shared,
                            MemoryOp::Merge,
                            DEFAULT_REALM,
                            None,
                            payload,
                        )
                        .await
                        {
                            return Ok(out);
                        }
                    }
                    // Async path needs no writer (and no embedding backend);
                    // build one lazily only when the queue is absent.
                    let q = shared.memory.clone();
                    let writer = if q.is_none() {
                        match build_writer(&shared).await {
                            Ok(w) => Some(w),
                            Err(e) => return Ok(e),
                        }
                    } else {
                        None
                    };
                    let into = node_id_of(&parsed.into_id);
                    let now = now_rfc3339();
                    let mut merged: Vec<String> = Vec::new();
                    for from in &parsed.from_ids {
                        let from_id = node_id_of(from);
                        if from_id == into {
                            continue;
                        }
                        // Archive the source (new version). fetch_latest_node
                        // barriers internally so queued versions are observed.
                        if let Ok(mut row) = fetch_latest_node(&shared, &from_id).await {
                            row["status"] = json!("archived");
                            row["updated_at"] = json!(now);
                            let realm = row_realm(&row);
                            match (&q, &writer) {
                                (Some(q), _) => {
                                    let _ = q.submit_node_row(&realm, row, true).await;
                                }
                                (None, Some(w)) => {
                                    let _ = w.append_node_rows(vec![row]).await;
                                }
                                (None, None) => unreachable!("writer built when queue is absent"),
                            }
                        }
                        // Record the merge edge.
                        let edge = kyma_memory::rows::edge_row(
                            &from_id,
                            &into,
                            "MERGED_INTO",
                            DEFAULT_REALM,
                            None,
                            None,
                            &now,
                        );
                        match (&q, &writer) {
                            (Some(q), _) => {
                                let _ = q.submit_edge_row(DEFAULT_REALM, edge, true).await;
                            }
                            (None, Some(w)) => {
                                let _ = w.append_edge_rows(vec![edge]).await;
                            }
                            (None, None) => unreachable!("writer built when queue is absent"),
                        }
                        merged.push(from_id);
                    }
                    let mut out = json!({"ok": true, "into": into, "merged": merged});
                    if q.is_some() {
                        out["queued"] = json!(true);
                    }
                    Ok(out)
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
                    let q = shared.memory.clone();
                    let writer = if q.is_none() {
                        match build_writer(&shared).await {
                            Ok(w) => Some(w),
                            Err(e) => return Ok(e),
                        }
                    } else {
                        None
                    };
                    // Land a node row through the queue when present, else the writer.
                    let put_node = |row: Value| {
                        let q = q.clone();
                        let writer = writer.clone();
                        async move {
                            let realm = row_realm(&row);
                            match (&q, &writer) {
                                (Some(q), _) => q
                                    .submit_node_row(&realm, row, true)
                                    .await
                                    .map_err(|e| e.to_string()),
                                (None, Some(w)) => {
                                    w.append_node_rows(vec![row]).await.map_err(|e| e.to_string())
                                }
                                (None, None) => unreachable!("writer built when queue is absent"),
                            }
                        }
                    };
                    let put_edge = |edge: Value, realm: String| {
                        let q = q.clone();
                        let writer = writer.clone();
                        async move {
                            match (&q, &writer) {
                                (Some(q), _) => q
                                    .submit_edge_row(&realm, edge, true)
                                    .await
                                    .map_err(|e| e.to_string()),
                                (None, Some(w)) => {
                                    w.append_edge_rows(vec![edge]).await.map_err(|e| e.to_string())
                                }
                                (None, None) => unreachable!("writer built when queue is absent"),
                            }
                        }
                    };
                    let src = node_id_of(&parsed.memory_id);
                    let dst = node_id_of(&parsed.target_id);
                    let verdict = parsed.verdict.trim().to_ascii_lowercase();
                    let now = now_rfc3339();

                    // Realm follows the target memory when we can read it.
                    // (fetch_latest_node barriers internally.)
                    let target_row = fetch_latest_node(&shared, &dst).await.ok();
                    let realm = target_row
                        .as_ref()
                        .and_then(|r| r.get("realm"))
                        .and_then(Value::as_str)
                        .unwrap_or(DEFAULT_REALM)
                        .to_string();
                    let queued = q.is_some();

                    // HITL chokepoint: supersede invalidates the target (by an
                    // existing memory), merged folds it away, other verdicts write
                    // a relationship. Route through the gate when present.
                    if shared.hitl.is_some() {
                        let (op, payload) = match verdict.as_str() {
                            "supersedes" | "invalidates" => (
                                MemoryOp::Invalidate,
                                OpPayload::Supersede {
                                    target_id: dst.clone(),
                                    by_id: src.clone(),
                                },
                            ),
                            "merged" | "merged_into" => (
                                MemoryOp::Merge,
                                OpPayload::Merge {
                                    into_id: src.clone(),
                                    from_ids: vec![dst.clone()],
                                },
                            ),
                            _ => (
                                MemoryOp::RelationshipWrite,
                                OpPayload::Link {
                                    src: src.clone(),
                                    dst: dst.clone(),
                                    rel: "RELATES_TO".to_string(),
                                    realm: realm.clone(),
                                    target_namespace: None,
                                },
                            ),
                        };
                        if let Some(out) =
                            memory_gate::gate_tool_op(&shared, op, &realm, parsed.reason.clone(), payload)
                                .await
                        {
                            return Ok(out);
                        }
                    }

                    match verdict.as_str() {
                        "supersedes" | "invalidates" => {
                            let Some(mut row) = target_row else {
                                return Ok(json!({"error": "target memory not found"}));
                            };
                            row["invalid_at"] = json!(now);
                            row["superseded_by"] = json!(src);
                            row["updated_at"] = json!(now);
                            if let Err(e) = put_node(row).await {
                                return Ok(json!({"error": format!("invalidate: {e}")}));
                            }
                            let edge = kyma_memory::rows::edge_row(
                                &src, &dst, "INVALIDATES", &realm, None, None, &now,
                            );
                            let _ = put_edge(edge, realm.clone()).await;
                            Ok(json!({"ok": true, "queued": queued, "verdict": "supersedes", "src": src, "dst": dst}))
                        }
                        "merged" | "merged_into" => {
                            if let Some(mut row) = target_row {
                                row["status"] = json!("archived");
                                row["updated_at"] = json!(now);
                                let _ = put_node(row).await;
                            }
                            let edge = kyma_memory::rows::edge_row(
                                &dst, &src, "MERGED_INTO", &realm, None, None, &now,
                            );
                            let _ = put_edge(edge, realm.clone()).await;
                            Ok(json!({"ok": true, "queued": queued, "verdict": "merged", "into": src, "from": dst}))
                        }
                        _ => {
                            let props = json!({ "verdict": verdict, "reason": parsed.reason });
                            let edge = kyma_memory::rows::edge_row(
                                &src, &dst, "RELATES_TO", &realm, None, Some(&props), &now,
                            );
                            if let Err(e) = put_edge(edge, realm.clone()).await {
                                return Ok(json!({"error": format!("relate: {e}")}));
                            }
                            Ok(json!({"ok": true, "queued": queued, "verdict": verdict, "src": src, "dst": dst}))
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
                    let mut cm = CreateMemory::new(content);
                    cm.title = Some("Session summary".to_string());
                    cm.memory_type = MemoryType::Summary;
                    cm.realm = parsed.realm.unwrap_or_else(|| DEFAULT_REALM.to_string());
                    cm.importance = 0.6;
                    cm.tags = vec!["session-summary".to_string()];
                    super::identity::stamp_provenance(&mut cm);

                    // Async-by-default — a session summary never has a
                    // topic_key, so no upsert lookup is needed.
                    if let Some(out) = try_queue_save(&shared, &cm, None).await {
                        return Ok(out);
                    }
                    let writer = match build_writer(&shared).await {
                        Ok(w) => w,
                        Err(e) => return Ok(e),
                    };
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

// ---------------------------------------------------------------------------
// save_memories — explicit bulk save (one batch, one response)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SaveMemoriesArgs {
    /// The memories to save. Each entry takes the same shape as `save_memory`.
    memories: Vec<SaveMemoryArgs>,
    /// Wait for all memories to be fully committed before returning. Default
    /// false: the batch is queued (one embedding call + one commit per table).
    #[serde(default)]
    sync: Option<bool>,
}

const SAVE_MEMORIES_DESC: &str = "Persist MANY durable memories in one call — \
much faster than repeated save_memory: the batch shares one embedding \
round-trip and one storage commit. Use whenever you have two or more \
facts/decisions/learnings to remember at once. Entries take the same shape as \
save_memory (content, title, memory_type, tags, realm, importance, \
references, topic_key). Returns the ids in input order.";

pub fn tool_save_memories(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "save_memories",
            SAVE_MEMORIES_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: SaveMemoriesArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    if parsed.memories.is_empty() {
                        return Ok(json!({"error": "memories is empty"}));
                    }
                    let sync = parsed.sync.unwrap_or(false);
                    let items: Vec<CreateMemory> = parsed
                        .memories
                        .into_iter()
                        .map(create_from_save_args)
                        .collect();

                    let mut ids: Vec<Value> = Vec::with_capacity(items.len());
                    let mut errors: Vec<String> = Vec::new();
                    let use_queue = !sync && shared.memory.is_some();
                    // The whole loop enqueues within one linger window, so the
                    // worker lands it as one batch: one embed call, one node
                    // commit, one edge commit.
                    let writer = if use_queue {
                        None
                    } else {
                        match build_writer(&shared).await {
                            Ok(w) => Some(w),
                            Err(e) => return Ok(e),
                        }
                    };
                    for (i, cm) in items.iter().enumerate() {
                        if use_queue {
                            let upsert = resolve_upsert_target(&shared, cm).await;
                            match try_queue_save(&shared, cm, upsert).await {
                                Some(out) => ids.push(out["id"].clone()),
                                None => {
                                    errors.push(format!("#{i}: queue rejected"));
                                    ids.push(Value::Null);
                                }
                            }
                            continue;
                        }
                        let w = writer.as_ref().expect("writer built for sync path");
                        if cm.topic_key.is_some() {
                            let _ = w.ensure_provisioned().await;
                        }
                        let upsert =
                            match cm.topic_key.as_deref() {
                                Some(tk) => find_by_topic_key(&shared, &cm.realm, tk)
                                    .await
                                    .and_then(|e| {
                                        uuid::Uuid::parse_str(
                                            e.strip_prefix("memory:").unwrap_or(&e),
                                        )
                                        .ok()
                                    }),
                                None => None,
                            };
                        let res = match upsert {
                            Some(u) => w.save_as(u, cm).await.map(|()| u),
                            None => w.save(cm).await,
                        };
                        match res {
                            Ok(id) => ids.push(json!(id.to_string())),
                            Err(e) => {
                                errors.push(format!("#{i}: {e}"));
                                ids.push(Value::Null);
                            }
                        }
                    }
                    let mut out = json!({
                        "saved": errors.is_empty(),
                        "count": ids.iter().filter(|v| !v.is_null()).count(),
                        "ids": ids,
                    });
                    if use_queue {
                        out["queued"] = json!(true);
                    }
                    if !errors.is_empty() {
                        out["errors"] = json!(errors);
                    }
                    Ok(out)
                }
            },
        )
        .with_parameters_schema::<SaveMemoriesArgs>()
        .with_read_only(false),
    )
}

// ---------------------------------------------------------------------------
// flush_memory — explicit barrier for callers that need a commit checkpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct FlushMemoryArgs {
    /// Realms to flush. Empty/omitted = everything pending.
    #[serde(default)]
    realms: Option<Vec<String>>,
}

const FLUSH_MEMORY_DESC: &str = "Wait until queued memory writes are fully \
committed (bounded). Saves are queued + batched in the background by default; \
recall/search already flush their target realms automatically — call this only \
when you need an explicit durability checkpoint (e.g. right before the session \
ends).";

pub fn tool_flush_memory(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "flush_memory",
            FLUSH_MEMORY_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: FlushMemoryArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let realms = parsed.realms.unwrap_or_default();
                    let flushed = match shared.memory.as_ref() {
                        Some(q) => q.barrier(&realms).await,
                        None => true, // synchronous mode: writes are always committed
                    };
                    Ok(json!({"flushed": flushed}))
                }
            },
        )
        .with_parameters_schema::<FlushMemoryArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

#[cfg(test)]
mod space_param_tests {
    use super::*;

    #[test]
    fn save_args_map_space_and_writer() {
        // Explicit space flows to CreateMemory; blank space is dropped.
        let args: SaveMemoryArgs = serde_json::from_value(json!({
            "content": "secret note",
            "space": "private:agentA"
        }))
        .unwrap();
        let cm = create_from_save_args(args);
        assert_eq!(cm.space.as_deref(), Some("private:agentA"));
        // No MCP client recorded in this unit test → writer_agent_id is None.
        assert!(cm.writer_agent_id.is_none());

        let blank: SaveMemoryArgs =
            serde_json::from_value(json!({ "content": "x", "space": "  " })).unwrap();
        assert!(create_from_save_args(blank).space.is_none(), "blank space dropped");

        // Omitted space stays public (None).
        let none: SaveMemoryArgs = serde_json::from_value(json!({ "content": "x" })).unwrap();
        assert!(create_from_save_args(none).space.is_none());
    }

    #[test]
    fn recall_args_deserialize_space_agent() {
        let r: RecallMemoryArgs =
            serde_json::from_value(json!({ "query": "q", "space_agent": "agentB" })).unwrap();
        assert_eq!(r.space_agent.as_deref(), Some("agentB"));
        let s: MemorySearchArgs =
            serde_json::from_value(json!({ "query": "q", "space_agent": "agentB" })).unwrap();
        assert_eq!(s.space_agent.as_deref(), Some("agentB"));
        // Omitted → None (shared-only), back-compatible with existing callers.
        let r2: RecallMemoryArgs = serde_json::from_value(json!({ "query": "q" })).unwrap();
        assert!(r2.space_agent.is_none());
    }
}
