//! SQL builders for memory recall / listing. The queries run server-side
//! (where `KymaTable` + the `cosine_distance` UDF live) against the columnar
//! memory tables. Each query first dedups to the latest version per node id
//! (`row_number() ... ORDER BY updated_at DESC`), matching the append-only
//! "latest-wins" model.

use crate::types::RecallFilter;

/// Quote + escape a string literal for inline SQL.
pub fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Render an embedding as a DataFusion `make_array(...)` literal.
fn make_array(embedding: &[f32]) -> String {
    let mut s = String::with_capacity(embedding.len() * 8 + 12);
    s.push_str("make_array(");
    for (i, x) in embedding.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        // f32 Display is the shortest round-trippable representation.
        s.push_str(&format!("{x}"));
    }
    s.push(')');
    s
}

fn filter_conditions(filter: &RecallFilter, default_non_archived: bool) -> Vec<String> {
    let mut conds = Vec::new();
    if filter.statuses.is_empty() {
        if default_non_archived {
            conds.push("status <> 'archived'".to_string());
        }
    } else {
        let list = filter
            .statuses
            .iter()
            .map(|s| sql_str(s.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        conds.push(format!("status IN ({list})"));
    }
    if !filter.realms.is_empty() {
        let list = filter
            .realms
            .iter()
            .map(|r| sql_str(r))
            .collect::<Vec<_>>()
            .join(", ");
        conds.push(format!("realm IN ({list})"));
    }
    if let Some(t) = filter.memory_type {
        conds.push(format!("memory_type = {}", sql_str(t.as_str())));
    }
    if let Some(min) = filter.importance_min {
        conds.push(format!("importance >= {}", min as f64));
    }
    if let Some(since) = &filter.since {
        conds.push(format!("created_at >= {}", sql_str(since)));
    }
    if let Some(until) = &filter.until {
        conds.push(format!("created_at <= {}", sql_str(until)));
    }
    for tag in &filter.tags {
        let needle = format!("%{}%", tag.replace('%', "").replace('_', ""));
        conds.push(format!("tags LIKE {}", sql_str(&needle)));
    }
    conds
}

/// Semantic recall: dedup → cosine distance → blended re-rank
/// (`relevance*0.7 + importance*0.3`) → top `limit`.
pub fn recall_sql(node_table: &str, embedding: &[f32], filter: &RecallFilter, limit: usize) -> String {
    let arr = make_array(embedding);
    let mut conds = filter_conditions(filter, true);
    if conds.is_empty() {
        conds.push("1 = 1".to_string());
    }
    let where_clause = conds.join(" AND ");
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n), \
         scored AS (\n  \
           SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, \
                  cosine_distance(embedding, {arr}) AS distance\n  \
           FROM latest WHERE __rn = 1\n) \
         SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, distance, \
                ({rw} * (1 - distance) + {iw} * importance) AS score \
         FROM scored WHERE {where_clause} ORDER BY score DESC LIMIT {limit}",
        nt = node_table,
        arr = arr,
        where_clause = where_clause,
        limit = limit,
        rw = crate::RELEVANCE_WEIGHT,
        iw = crate::IMPORTANCE_WEIGHT,
    )
}

/// Non-semantic listing with filters, newest first.
pub fn list_sql(node_table: &str, filter: &RecallFilter, limit: usize, offset: usize) -> String {
    let mut conds = filter_conditions(filter, true);
    if conds.is_empty() {
        conds.push("1 = 1".to_string());
    }
    let where_clause = conds.join(" AND ");
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n) \
         SELECT id, memory_type, title, content_preview, tags, importance, status, realm, created_at \
         FROM latest WHERE __rn = 1 AND {where_clause} ORDER BY created_at DESC LIMIT {limit} OFFSET {offset}",
        nt = node_table,
        where_clause = where_clause,
        limit = limit,
        offset = offset,
    )
}

/// Fetch the full latest version of a single node (for read-then-append
/// mutations like status/importance updates).
pub fn latest_node_sql(node_table: &str, node_id: &str) -> String {
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n) \
         SELECT id, labels, realm, memory_type, title, content, content_preview, tags, importance, status, \
                source_session_id, source_run_id, embedding, created_at, updated_at \
         FROM latest WHERE __rn = 1 AND id = {idv}",
        nt = node_table,
        idv = sql_str(node_id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryStatus, MemoryType};

    #[test]
    fn sql_str_escapes_quotes() {
        assert_eq!(sql_str("a'b"), "'a''b'");
    }

    #[test]
    fn recall_sql_has_cosine_and_blend() {
        let f = RecallFilter {
            realms: vec!["proj".into(), "global".into()],
            memory_type: Some(MemoryType::Fact),
            ..Default::default()
        };
        let s = recall_sql("memory_nodes", &[0.1, 0.2], &f, 8);
        assert!(s.contains("cosine_distance(embedding, make_array(0.1, 0.2))"));
        assert!(s.contains("0.7 * (1 - distance) + 0.3 * importance"));
        assert!(s.contains("realm IN ('proj', 'global')"));
        assert!(s.contains("memory_type = 'fact'"));
        assert!(s.contains("status <> 'archived'"));
        assert!(s.trim_end().ends_with("LIMIT 8"));
    }

    #[test]
    fn list_sql_respects_status_filter() {
        let f = RecallFilter {
            statuses: vec![MemoryStatus::Active],
            ..Default::default()
        };
        let s = list_sql("memory_nodes", &f, 50, 10);
        assert!(s.contains("status IN ('active')"));
        assert!(s.contains("OFFSET 10"));
    }
}
