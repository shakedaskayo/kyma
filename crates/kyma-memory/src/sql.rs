//! SQL builders for memory recall / listing. The queries run server-side
//! (where `KymaTable` + the `cosine_distance` UDF live) against the columnar
//! memory tables. Each query first dedups to the latest version per node id
//! (`row_number() ... ORDER BY updated_at DESC`), matching the append-only
//! "latest-wins" model.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::types::{RecallFilter, SourceSummary};

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
    // Bi-temporal validity: exclude superseded/invalidated memories unless asked.
    // With an `as_of` instant we bound both ends of the validity interval; the
    // default ("now") just drops anything already invalidated. `valid_at` /
    // `invalid_at` are NULL on memories written before bi-temporal support, and
    // NULL is treated as "always valid" so older stores keep working.
    if !filter.include_invalidated {
        match &filter.as_of {
            Some(t) => {
                conds.push(format!(
                    "(invalid_at IS NULL OR invalid_at > {})",
                    sql_str(t)
                ));
                conds.push(format!("(valid_at IS NULL OR valid_at <= {})", sql_str(t)));
            }
            None => conds.push("invalid_at IS NULL".to_string()),
        }
    }
    conds
}

/// Semantic recall: dedup → cosine distance → blended re-rank
/// (`relevance*0.7 + importance*0.3`) → top `limit`.
pub fn recall_sql(
    node_table: &str,
    embedding: &[f32],
    filter: &RecallFilter,
    limit: usize,
    ann_threshold: Option<f64>,
) -> String {
    let arr = make_array(embedding);
    let mut conds = filter_conditions(filter, true);
    if conds.is_empty() {
        conds.push("1 = 1".to_string());
    }
    let where_clause = conds.join(" AND ");
    // Optional native-ANN activation: when `ann_threshold` is set, add a
    // `cosine_distance(embedding, q) < threshold` predicate. The planner pushes
    // it down to the columnar scan, where per-extent centroid+radius stats
    // prune extents (see kyma-exec/kyma-catalog). `None`/0 → exact full-scan.
    // Correctness holds either way; the predicate only filters out memories
    // already more distant than the threshold.
    let ann = match ann_threshold {
        Some(t) if t > 0.0 => format!(" AND cosine_distance(embedding, {arr}) < {t}"),
        _ => String::new(),
    };
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n), \
         scored AS (\n  \
           SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, \
                  cosine_distance(embedding, {arr}) AS distance\n  \
           FROM latest WHERE __rn = 1{ann}\n) \
         SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, distance, \
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

/// Semantic recall restricted to a pre-computed candidate id set.
///
/// Identical to [`recall_sql`] in every semantic respect — latest-version
/// dedup, bi-temporal validity, realm/type/importance/tag filters, and the
/// blended `relevance·0.7 + importance·0.3` re-rank — but additionally
/// constrains the scan to `candidate_ids`. The ANN query path (IVF+RaBitQ
/// sidecars) supplies that id set: it surfaces the nearest memories by cosine
/// distance, then this SQL re-applies *all* memory semantics over exactly that
/// pool. An empty `candidate_ids` yields no rows (the caller should fall back
/// to [`recall_sql`] when there are no ANN candidates). Cosine distance and
/// the score are still computed in SQL, so ordering/score are byte-identical to
/// [`recall_sql`] over the same surviving rows.
pub fn recall_sql_for_ids(
    node_table: &str,
    embedding: &[f32],
    filter: &RecallFilter,
    limit: usize,
    candidate_ids: &[String],
) -> String {
    let arr = make_array(embedding);
    let mut conds = filter_conditions(filter, true);
    let id_list = candidate_ids
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    // Empty set → impossible predicate so the query returns nothing rather than
    // a syntactically-invalid `IN ()`.
    if candidate_ids.is_empty() {
        conds.push("1 = 0".to_string());
    } else {
        conds.push(format!("id IN ({id_list})"));
    }
    let where_clause = conds.join(" AND ");
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n), \
         scored AS (\n  \
           SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, \
                  cosine_distance(embedding, {arr}) AS distance\n  \
           FROM latest WHERE __rn = 1\n) \
         SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, distance, \
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
                source_session_id, source_run_id, embedding, created_at, updated_at, \
                valid_at, invalid_at, superseded_by, provenance, topic_key \
         FROM latest WHERE __rn = 1 AND id = {idv}",
        nt = node_table,
        idv = sql_str(node_id),
    )
}

/// Tokenize a free-text query the same way the columnar writer tokenizes
/// stored text (lowercase, split on non-alphanumeric, drop tokens < 2 chars),
/// so query tokens line up with the per-extent token index used for pruning.
pub fn tokenize_query(q: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in q.split(|c: char| !c.is_alphanumeric()) {
        if tok.chars().count() >= 2 {
            let t = tok.to_ascii_lowercase();
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Keyword recall: rank latest memories by how many query tokens appear in
/// `content`/`title`/`tags`. Each `LIKE` triggers the extent token-set pruning
/// (`ContainsTokens`) so this is index-accelerated. Excludes rows with no hit.
pub fn keyword_recall_sql(
    node_table: &str,
    tokens: &[String],
    filter: &RecallFilter,
    limit: usize,
) -> String {
    let mut conds = filter_conditions(filter, true);
    if conds.is_empty() {
        conds.push("1 = 1".to_string());
    }
    let where_clause = conds.join(" AND ");
    let score_expr = if tokens.is_empty() {
        "0".to_string()
    } else {
        tokens
            .iter()
            .map(|t| {
                let needle = sql_str(&format!("%{}%", t.replace('%', "").replace('_', "")));
                format!(
                    "(CASE WHEN lower(content) LIKE {n} OR lower(title) LIKE {n} \
                       OR lower(tags) LIKE {n} THEN 1 ELSE 0 END)",
                    n = needle
                )
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n), \
         scored AS (\n  \
           SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, \
                  ({score}) AS kw_score\n  \
           FROM latest WHERE __rn = 1\n) \
         SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, kw_score \
         FROM scored WHERE {where_clause} AND kw_score > 0 ORDER BY kw_score DESC, importance DESC LIMIT {limit}",
        nt = node_table,
        score = score_expr,
        where_clause = where_clause,
        limit = limit,
    )
}

/// Keyword recall restricted to a BM25-selected candidate id set. Mirrors
/// [`keyword_recall_sql`] but, instead of letting the columnar `LIKE` token-set
/// scan generate the candidates, takes the ids a tantivy BM25 sidecar already
/// ranked and re-applies all memory semantics (latest-version dedup, filters,
/// validity) over just those rows. The same token-overlap `kw_score` is still
/// computed so the blend's keyword weight is populated, but rows are NOT dropped
/// on `kw_score = 0`: BM25 membership is the inclusion signal (our crude LIKE
/// tokenizer can miss a stemmed match the index found), and such rows still earn
/// RRF credit by their rank in the lexical list.
pub fn keyword_recall_sql_for_ids(
    node_table: &str,
    tokens: &[String],
    filter: &RecallFilter,
    limit: usize,
    candidate_ids: &[String],
) -> String {
    let mut conds = filter_conditions(filter, true);
    let id_list = candidate_ids
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    if candidate_ids.is_empty() {
        conds.push("1 = 0".to_string());
    } else {
        conds.push(format!("id IN ({id_list})"));
    }
    let where_clause = conds.join(" AND ");
    let score_expr = if tokens.is_empty() {
        "0".to_string()
    } else {
        tokens
            .iter()
            .map(|t| {
                let needle = sql_str(&format!("%{}%", t.replace('%', "").replace('_', "")));
                format!(
                    "(CASE WHEN lower(content) LIKE {n} OR lower(title) LIKE {n} \
                       OR lower(tags) LIKE {n} THEN 1 ELSE 0 END)",
                    n = needle
                )
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n), \
         scored AS (\n  \
           SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, \
                  ({score}) AS kw_score\n  \
           FROM latest WHERE __rn = 1\n) \
         SELECT id, memory_type, title, content, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at, kw_score \
         FROM scored WHERE {where_clause} ORDER BY kw_score DESC, importance DESC LIMIT {limit}",
        nt = node_table,
        score = score_expr,
        where_clause = where_clause,
        limit = limit,
    )
}

/// One-hop neighbour edges for a set of seed node ids (both directions), used
/// to graph-expand a contextual subgraph. Returns edge rows; the orchestrator
/// derives the far endpoint per seed and follows `target_namespace` for
/// cross-graph (catalog resource / trace) links. Caller must pass ≥1 seed.
pub fn neighbors_sql(
    edge_table: &str,
    seed_ids: &[String],
    realms: &[String],
    limit: usize,
) -> String {
    let ids = seed_ids
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    let mut where_parts = vec![format!("(src IN ({ids}) OR dst IN ({ids}))")];
    if !realms.is_empty() {
        let rl = realms
            .iter()
            .map(|r| sql_str(r))
            .collect::<Vec<_>>()
            .join(", ");
        where_parts.push(format!("realm IN ({rl})"));
    }
    format!(
        "SELECT src, dst, type, realm, target_namespace FROM {et} WHERE {wc} LIMIT {limit}",
        et = edge_table,
        wc = where_parts.join(" AND "),
        limit = limit,
    )
}

/// Fetch the latest versions of specific (currently-valid) memory node ids —
/// used to materialize graph-pulled neighbour memories. Caller must pass ≥1 id.
pub fn nodes_by_id_sql(node_table: &str, ids: &[String]) -> String {
    let idlist = ids
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n) \
         SELECT id, memory_type, title, content_preview, tags, importance, status, realm, created_at, valid_at, invalid_at \
         FROM latest WHERE __rn = 1 AND id IN ({idlist}) AND invalid_at IS NULL",
        nt = node_table,
        idlist = idlist,
    )
}

/// Counts per (provenance blob, realm) over the latest, non-archived node
/// versions — the raw input to the Data Sources "where do memories come from"
/// summary. `provenance` is an opaque JSON string column and the engine has no
/// JSON UDFs, so the query groups by the raw blob; [`fold_source_summary`]
/// parses `provenance.source` out of each group and merges in Rust.
pub fn source_summary_sql(node_table: &str) -> String {
    format!(
        "WITH latest AS (\n  \
           SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM {nt}\n) \
         SELECT provenance, realm, count(*) AS n FROM latest \
         WHERE __rn = 1 AND status <> 'archived' \
         GROUP BY provenance, realm",
        nt = node_table,
    )
}

/// Fold the `{provenance, realm, n}` rows of [`source_summary_sql`] into
/// per-(source, realm) counts. A missing/unparsable provenance or one without
/// a `source` string counts as `"manual"`; a NULL realm counts as the default
/// realm. Sorted by count descending, then (source, realm) for determinism.
pub fn fold_source_summary(rows: &[Value]) -> Vec<SourceSummary> {
    let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();
    for row in rows {
        let source = row
            .get("provenance")
            .and_then(Value::as_str)
            .and_then(|p| serde_json::from_str::<Value>(p).ok())
            .and_then(|p| p.get("source").and_then(Value::as_str).map(str::to_owned))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "manual".to_string());
        let realm = row
            .get("realm")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
            .unwrap_or(crate::DEFAULT_REALM)
            .to_string();
        // Arrow→JSON keeps count(*) numeric, but tolerate stringified numbers.
        let n = match row.get("n") {
            Some(Value::Number(v)) => v.as_i64().unwrap_or(0),
            Some(Value::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        *counts.entry((source, realm)).or_insert(0) += n;
    }
    let mut out: Vec<SourceSummary> = counts
        .into_iter()
        .map(|((source, realm), count)| SourceSummary {
            source,
            realm,
            count,
        })
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.realm.cmp(&b.realm))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryStatus, MemoryType};
    use serde_json::json;

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
        let s = recall_sql("memory_nodes", &[0.1, 0.2], &f, 8, None);
        assert!(s.contains("cosine_distance(embedding, make_array(0.1, 0.2))"));
        assert!(s.contains("0.7 * (1 - distance) + 0.3 * importance"));
        assert!(s.contains("realm IN ('proj', 'global')"));
        assert!(s.contains("memory_type = 'fact'"));
        assert!(s.contains("status <> 'archived'"));
        // Bi-temporal: invalidated memories excluded by default + columns projected.
        assert!(s.contains("invalid_at IS NULL"));
        assert!(s.contains(", valid_at, invalid_at, distance,"));
        assert!(s.trim_end().ends_with("LIMIT 8"));
    }

    #[test]
    fn recall_sql_as_of_bounds_validity_interval() {
        let f = RecallFilter {
            as_of: Some("2026-01-01T00:00:00Z".into()),
            ..Default::default()
        };
        let s = recall_sql("memory_nodes", &[0.1], &f, 5, None);
        assert!(s.contains("(invalid_at IS NULL OR invalid_at > '2026-01-01T00:00:00Z')"));
        assert!(s.contains("(valid_at IS NULL OR valid_at <= '2026-01-01T00:00:00Z')"));
    }

    #[test]
    fn recall_sql_for_ids_restricts_and_keeps_semantics() {
        let f = RecallFilter {
            memory_type: Some(MemoryType::Fact),
            ..Default::default()
        };
        let ids = vec!["a".to_string(), "b'c".to_string()];
        let s = recall_sql_for_ids("memory_nodes", &[0.1, 0.2], &f, 8, &ids);
        // Same cosine + blended-score semantics as recall_sql.
        assert!(s.contains("cosine_distance(embedding, make_array(0.1, 0.2))"));
        assert!(s.contains("0.7 * (1 - distance) + 0.3 * importance"));
        assert!(s.contains("memory_type = 'fact'"));
        assert!(s.contains("invalid_at IS NULL"));
        // The id restriction is present and escaped.
        assert!(s.contains("id IN ('a', 'b''c')"));
        assert!(s.trim_end().ends_with("LIMIT 8"));
    }

    #[test]
    fn recall_sql_for_ids_empty_set_is_impossible_predicate() {
        let f = RecallFilter::default();
        let s = recall_sql_for_ids("memory_nodes", &[0.1], &f, 5, &[]);
        assert!(s.contains("1 = 0"));
        assert!(!s.contains("IN ()"));
    }

    #[test]
    fn recall_sql_adds_ann_threshold_when_set() {
        let f = RecallFilter::default();
        let s = recall_sql("memory_nodes", &[0.1, 0.2], &f, 5, Some(0.4));
        assert!(s.contains("cosine_distance(embedding, make_array(0.1, 0.2)) < 0.4"));
        let s2 = recall_sql("memory_nodes", &[0.1, 0.2], &f, 5, None);
        assert!(!s2.contains("< 0.4"));
    }

    #[test]
    fn recall_sql_include_invalidated_drops_validity_guard() {
        let f = RecallFilter {
            include_invalidated: true,
            ..Default::default()
        };
        let s = recall_sql("memory_nodes", &[0.1], &f, 5, None);
        assert!(!s.contains("invalid_at IS NULL"));
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

    #[test]
    fn tokenize_query_lowercases_splits_and_dedups() {
        let toks = tokenize_query("Kyma uses PGVECTOR; kyma!!");
        assert_eq!(toks, vec!["kyma", "uses", "pgvector"]);
    }

    #[test]
    fn keyword_recall_builds_like_scoring() {
        let toks = tokenize_query("pgvector index");
        let f = RecallFilter {
            realms: vec!["proj".into()],
            ..Default::default()
        };
        let s = keyword_recall_sql("memory_nodes", &toks, &f, 10);
        assert!(s.contains("lower(content) LIKE '%pgvector%'"));
        assert!(s.contains("lower(tags) LIKE '%index%'"));
        assert!(s.contains("AS kw_score"));
        assert!(s.contains("kw_score > 0"));
        assert!(s.contains("invalid_at IS NULL")); // validity guard threads through
        assert!(s.contains("realm IN ('proj')"));
    }

    #[test]
    fn keyword_recall_for_ids_restricts_without_dropping_zero_score() {
        let toks = tokenize_query("pgvector index");
        let f = RecallFilter {
            realms: vec!["proj".into()],
            ..Default::default()
        };
        let ids = vec!["a".to_string(), "b'c".to_string()];
        let s = keyword_recall_sql_for_ids("memory_nodes", &toks, &f, 10, &ids);
        // Same token-overlap kw_score is computed.
        assert!(s.contains("lower(content) LIKE '%pgvector%'"));
        assert!(s.contains("AS kw_score"));
        // BM25-selected ids restrict the set, escaped.
        assert!(s.contains("id IN ('a', 'b''c')"));
        // Filters + validity still thread through.
        assert!(s.contains("realm IN ('proj')"));
        assert!(s.contains("invalid_at IS NULL"));
        // Unlike keyword_recall_sql, rows are NOT dropped on kw_score = 0.
        assert!(!s.contains("kw_score > 0"));
        assert!(s.trim_end().ends_with("LIMIT 10"));
    }

    #[test]
    fn keyword_recall_for_ids_empty_set_is_impossible_predicate() {
        let toks = tokenize_query("anything");
        let f = RecallFilter::default();
        let s = keyword_recall_sql_for_ids("memory_nodes", &toks, &f, 5, &[]);
        assert!(s.contains("1 = 0"));
        assert!(!s.contains("IN ()"));
    }

    #[test]
    fn neighbors_sql_both_directions_and_realm() {
        let s = neighbors_sql(
            "memory_edges",
            &["memory:a".into(), "memory:b".into()],
            &["proj".into()],
            100,
        );
        assert!(s.contains("src IN ('memory:a', 'memory:b')"));
        assert!(s.contains("OR dst IN ('memory:a', 'memory:b')"));
        assert!(s.contains("realm IN ('proj')"));
        assert!(s.trim_end().ends_with("LIMIT 100"));
    }

    #[test]
    fn nodes_by_id_filters_invalidated() {
        let s = nodes_by_id_sql("memory_nodes", &["memory:a".into()]);
        assert!(s.contains("id IN ('memory:a')"));
        assert!(s.contains("invalid_at IS NULL"));
    }

    #[test]
    fn source_summary_sql_dedups_and_excludes_archived() {
        let s = source_summary_sql("memory_nodes");
        assert!(s.contains("PARTITION BY id ORDER BY updated_at DESC"));
        assert!(s.contains("__rn = 1"));
        assert!(s.contains("status <> 'archived'"));
        assert!(s.contains("GROUP BY provenance, realm"));
    }

    #[test]
    fn fold_source_summary_parses_source_and_merges() {
        // Two distinct claude-code provenance blobs (different run ids) must
        // merge into one (source, realm) bucket; NULL provenance and a blob
        // without a `source` key both count as "manual".
        let rows = vec![
            json!({"provenance": "{\"source\":\"claude-code\",\"run_id\":\"a\"}", "realm": "kyma", "n": 1}),
            json!({"provenance": "{\"source\":\"claude-code\",\"run_id\":\"b\"}", "realm": "kyma", "n": 1}),
            json!({"provenance": null, "realm": "default", "n": 1}),
            json!({"provenance": "{\"run_id\":\"c\"}", "realm": "default", "n": 2}),
        ];
        let out = fold_source_summary(&rows);
        assert_eq!(
            out,
            vec![
                SourceSummary {
                    source: "manual".into(),
                    realm: "default".into(),
                    count: 3
                },
                SourceSummary {
                    source: "claude-code".into(),
                    realm: "kyma".into(),
                    count: 2
                },
            ]
        );
    }

    #[test]
    fn fold_source_summary_defaults_null_realm_and_string_counts() {
        let rows = vec![json!({"provenance": null, "realm": null, "n": "4"})];
        let out = fold_source_summary(&rows);
        assert_eq!(
            out,
            vec![SourceSummary {
                source: "manual".into(),
                realm: "default".into(),
                count: 4
            }]
        );
    }

    #[test]
    fn fold_source_summary_treats_malformed_provenance_as_manual() {
        let rows = vec![json!({"provenance": "not json", "realm": "kyma", "n": 1})];
        let out = fold_source_summary(&rows);
        assert_eq!(
            out,
            vec![SourceSummary {
                source: "manual".into(),
                realm: "kyma".into(),
                count: 1
            }]
        );
    }
}
