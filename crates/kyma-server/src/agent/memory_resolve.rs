//! Entity resolution + graph linking — turns the extractor's loose entity and
//! relationship mentions into first-class memory-graph nodes and edges, and
//! best-effort links them to the real catalog resources/traces they denote.
//!
//! For each extracted entity we mint (or reuse, by exact name in-realm) a
//! lightweight `memory_type=entity` node — embeddable and recallable like any
//! memory. We then connect:
//! - memory → its entities      (`REFERENCES`, written by the writer)
//! - entity ↔ entity            (`RELATES_TO`, predicate in `props`)
//! - entity → catalog resource  (`RESOLVES_TO`, best-effort via the
//!   `column_stats` distinct-value index, carrying `target_namespace` so the
//!   unified GraphView stitches the cross-graph edge).
//!
//! "Everything in the catalog" is reachable as a link target: the resolver
//! uses [`super::tools::find_references`], the same index that powers
//! `find_references_to`, so any value a connector ingested (repos, services,
//! traces, …) can anchor a memory.

use std::collections::HashMap;

use serde_json::{json, Value};

use kyma_memory::rows;
use kyma_memory::types::{CreateMemory, MemoryType};
use kyma_memory::{
    MemoryWriter, DEFAULT_DATABASE, EDGE_RELATES_TO, EDGE_RESOLVES_TO, NODE_TABLE,
};

use super::memory_extract::{ExtractedEntity, ExtractedRel};
use super::tools::{execute_sql, find_references, SharedToolCtx};

/// Result of resolving one realm's entities/relationships.
#[derive(Debug, Default)]
pub struct ResolveOutcome {
    /// Lowercased entity name → its memory-graph node id (`memory:<uuid>`).
    pub entity_nodes: HashMap<String, String>,
    pub entities_written: i64,
    pub relationships_written: i64,
}

/// Max catalog databases to cross-link a single entity into (best-effort).
const MAX_RESOLVE_LINKS: usize = 4;

/// Resolve `entities`, mint/reuse their nodes, then write `RELATES_TO` edges
/// for `relationships` and best-effort `RESOLVES_TO` edges into the catalog.
pub async fn resolve_and_link(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    entities: &[ExtractedEntity],
    relationships: &[ExtractedRel],
) -> ResolveOutcome {
    let mut out = ResolveOutcome::default();

    for ent in entities {
        let name = ent.name.trim();
        if name.is_empty() {
            continue;
        }
        let key = name.to_ascii_lowercase();
        if out.entity_nodes.contains_key(&key) {
            continue; // already handled in this batch
        }
        // Reuse an existing entity node with the same name in this realm.
        if let Some(id) = find_entity_node(shared, realm, &key).await {
            out.entity_nodes.insert(key, id);
            continue;
        }
        // Mint a new entity node (embeddable + recallable).
        let mut content = format!("Entity: {name}");
        if let Some(k) = &ent.kind {
            if !k.trim().is_empty() {
                content.push_str(&format!(" ({k})"));
            }
        }
        if !ent.aliases.is_empty() {
            content.push_str(&format!(". Also known as: {}.", ent.aliases.join(", ")));
        }
        let mut cm = CreateMemory::new(content);
        cm.title = Some(name.to_string());
        cm.memory_type = MemoryType::Entity;
        cm.realm = realm.to_string();
        cm.importance = 0.3;
        cm.tags = vec![format!("kind:{}", ent.kind.as_deref().unwrap_or("entity"))];
        match writer.save(&cm).await {
            Ok(id) => {
                let node_id = rows::node_id(&id);
                out.entities_written += 1;
                // Best-effort cross-graph links into the catalog.
                out.relationships_written +=
                    link_to_catalog(shared, writer, realm, &node_id, name, &ent.aliases).await;
                out.entity_nodes.insert(key, node_id);
            }
            Err(e) => {
                tracing::debug!(entity = %name, error = %e, "mint entity node failed");
            }
        }
    }

    // entity ↔ entity relationships (only when both endpoints resolved).
    let now = now_rfc3339();
    let mut rel_rows: Vec<Value> = Vec::new();
    for rel in relationships {
        let src = out.entity_nodes.get(&rel.src.trim().to_ascii_lowercase());
        let dst = out.entity_nodes.get(&rel.dst.trim().to_ascii_lowercase());
        if let (Some(src), Some(dst)) = (src, dst) {
            if src == dst {
                continue;
            }
            let props = json!({ "predicate": rel.predicate });
            rel_rows.push(rows::edge_row(
                src,
                dst,
                EDGE_RELATES_TO,
                realm,
                None,
                Some(&props),
                &now,
            ));
        }
    }
    if !rel_rows.is_empty() {
        let n = rel_rows.len() as i64;
        if writer.append_edge_rows(rel_rows).await.is_ok() {
            out.relationships_written += n;
        }
    }

    out
}

/// Best-effort: find catalog databases where this entity's name/aliases appear
/// (via the distinct-value index) and write a `RESOLVES_TO` edge per database.
/// The dst uses the `<db>::<value>` convention so the GraphView can stitch it;
/// it is heuristic (assumes the value is the node id), so failures are silent.
/// Returns the number of edges written.
async fn link_to_catalog(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    realm: &str,
    entity_node_id: &str,
    name: &str,
    aliases: &[String],
) -> i64 {
    let mut seen_db: Vec<String> = Vec::new();
    let mut candidates: Vec<&str> = vec![name];
    candidates.extend(aliases.iter().map(String::as_str));

    for value in candidates {
        if seen_db.len() >= MAX_RESOLVE_LINKS {
            break;
        }
        let hits = match find_references(&shared.pool, None, value).await {
            Ok(h) => h,
            Err(_) => continue,
        };
        for (db, _table, _col) in hits {
            if db == DEFAULT_DATABASE || seen_db.contains(&db) {
                continue; // skip the memory store itself + dedup per db
            }
            if seen_db.len() >= MAX_RESOLVE_LINKS {
                break;
            }
            let dst = format!("{db}::{value}");
            if writer
                .link(entity_node_id, &dst, EDGE_RESOLVES_TO, realm, Some(db.as_str()))
                .await
                .is_ok()
            {
                seen_db.push(db);
            }
        }
    }
    seen_db.len() as i64
}

/// Look up an existing `entity` node by exact (case-insensitive) title in realm.
async fn find_entity_node(shared: &SharedToolCtx, realm: &str, name_lc: &str) -> Option<String> {
    let q = format!(
        "WITH latest AS (SELECT id, title, memory_type, realm, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT id FROM latest WHERE rn = 1 AND memory_type = 'entity' \
           AND realm = {realm} AND lower(title) = {name} LIMIT 1",
        nt = NODE_TABLE,
        realm = kyma_memory::sql::sql_str(realm),
        name = kyma_memory::sql::sql_str(name_lc),
    );
    let res = execute_sql(shared, DEFAULT_DATABASE, &q, 1).await;
    res.get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
