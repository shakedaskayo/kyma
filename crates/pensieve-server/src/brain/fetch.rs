//! Memory-table reads for the exporter: latest node versions + edges for a
//! brain's realm selection, mapped to `kyma-brain`'s row types. The
//! `embedding` column is excluded at SQL level and never leaves the store.

use kyma_brain::registry::{BrainConfig, RealmSelector};
use kyma_brain::types::{EdgeRow, NoteRow};
use serde_json::Value;

use crate::agent::state::AgentState;
use crate::agent::tools::{execute_sql, SharedToolCtx};

const MAX_ROWS: usize = 1_000_000;

fn shared_ctx(agent: &AgentState) -> SharedToolCtx {
    SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: agent.catalog.clone(),
        format: agent.format.clone(),
        pool: agent.pool.clone(),
        memory: agent.memory.clone(),
        hitl: None,
        memory_settings_path: agent.memory_settings_path.clone(),
    }
}

fn realm_predicate(cfg: &BrainConfig) -> String {
    match &cfg.realms {
        RealmSelector::All => String::new(),
        RealmSelector::Realms(realms) => {
            let quoted: Vec<String> =
                realms.iter().map(|r| format!("'{}'", r.replace('\'', "''"))).collect();
            format!(" AND realm IN ({})", quoted.join(", "))
        }
    }
}

fn str_field(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn opt_field(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Fetch the latest node versions + all edges for a brain. Status /
/// importance / type filters are applied later by the pure planner
/// (`kyma_brain::vault::included`) — SQL only scopes realms and drops the
/// embedding column.
pub async fn fetch_rows(
    agent: &AgentState,
    cfg: &BrainConfig,
) -> Result<(Vec<NoteRow>, Vec<EdgeRow>), String> {
    let shared = shared_ctx(agent);
    let db = kyma_memory::DEFAULT_DATABASE;
    let realm_filter = realm_predicate(cfg);

    let nodes_sql = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM memory_nodes) \
         SELECT id, realm, memory_type, title, content, tags, importance, status, \
                created_at, updated_at, valid_at, invalid_at, topic_key \
         FROM latest WHERE __rn = 1{realm_filter}"
    );
    let nodes_val = execute_sql(&shared, db, &nodes_sql, MAX_ROWS).await;
    if let Some(e) = nodes_val.get("error") {
        // A store with no memory database yet is an empty brain, not an error.
        let msg = e.as_str().unwrap_or_default();
        if msg.contains("no tables") || msg.contains("does not exist") {
            return Ok((Vec::new(), Vec::new()));
        }
        return Err(format!("memory_nodes: {msg}"));
    }
    let nodes: Vec<NoteRow> = nodes_val
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|r| NoteRow {
                    // Stored node ids carry the `memory:` prefix; the brain
                    // crate (filenames, frontmatter kyma_memory_id) uses the
                    // bare uuid.
                    id: str_field(r, "id").strip_prefix("memory:").map_or_else(
                        || str_field(r, "id"),
                        str::to_string,
                    ),
                    realm: str_field(r, "realm"),
                    memory_type: str_field(r, "memory_type"),
                    title: str_field(r, "title"),
                    content: str_field(r, "content"),
                    tags: str_field(r, "tags"),
                    importance: r.get("importance").and_then(Value::as_f64).unwrap_or(0.5),
                    status: str_field(r, "status"),
                    created_at: str_field(r, "created_at"),
                    updated_at: str_field(r, "updated_at"),
                    valid_at: opt_field(r, "valid_at"),
                    invalid_at: opt_field(r, "invalid_at"),
                    topic_key: opt_field(r, "topic_key"),
                })
                .filter(|n| !n.id.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let edges_sql = "SELECT src, dst, type FROM memory_edges".to_string();
    let edges_val = execute_sql(&shared, db, &edges_sql, MAX_ROWS).await;
    let edges: Vec<EdgeRow> = edges_val
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|r| EdgeRow {
                    src: str_field(r, "src"),
                    dst: str_field(r, "dst"),
                    edge_type: str_field(r, "type"),
                })
                .filter(|e| !e.src.is_empty() && !e.dst.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ok((nodes, edges))
}
