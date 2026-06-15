//! Graph-aware hybrid memory retrieval — the near-realtime "find anything"
//! read path. No LLM in the hot path.
//!
//! Pipeline (all over kyma's own engine):
//! 1. embed the query once;
//! 2. generate candidates two ways IN PARALLEL — semantic (vector
//!    `cosine_distance`, accelerated by an IVF+RaBitQ ANN sidecar when present)
//!    and keyword (tantivy BM25 over an FTS sidecar when present, else the
//!    columnar token-set `LIKE` pruned via `column_stats`);
//! 3. fuse the ranked lists with Reciprocal Rank Fusion (RRF);
//! 4. graph-expand the top seeds 1–2 hops over `memory_edges` to pull in
//!    connected memories AND linked catalog resources/traces (cross-graph
//!    `target_namespace` endpoints) — the "contextual understanding" step;
//! 5. blend a final score (RRF + semantic + keyword + graph-proximity +
//!    importance + recency), bi-temporal validity already filtered in SQL;
//! 6. assemble a compact, citation-rich context block ready for an agent or a
//!    Claude Code hook to consume.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::Instrument as _;

use kyma_core::tenant::DEFAULT_TENANT;
use kyma_graph_topo::{CsrGraph, Direction as TopoDir};
use kyma_memory::types::{MemoryClass, MemoryType, RecallFilter};
use kyma_memory::{sql, MemoryWriter, DEFAULT_DATABASE, EDGE_TABLE, NODE_TABLE};

use super::memory_settings::{self, MemorySettings};
use super::tools::{execute_sql, SharedToolCtx};

/// Candidates pulled per modality before fusion (oversample for RRF).
const CAND_K: usize = 50;
/// Top fused candidates used as graph-expansion seeds.
const SEED_N: usize = 10;
/// Max neighbour edges materialized per hop.
const EXPAND_CAP: usize = 200;
/// Hard cap on hops regardless of request.
const MAX_HOPS: u8 = 2;

// ── request / response ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RetrieveRequest {
    pub query: String,
    #[serde(default)]
    pub realms: Vec<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub importance_min: Option<f32>,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub include_invalidated: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub expand_hops: Option<u8>,
    /// Requesting agent's identity for memory-space visibility (S3.3). When set,
    /// recall returns shared memories (space NULL/public) plus this agent's own
    /// `private:<agent>` memories. `None` (default) applies no space filter, so
    /// callers without an agent context are unchanged.
    #[serde(default)]
    pub space_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievedMemory {
    pub id: String,
    pub memory_type: String,
    pub title: Option<String>,
    pub content_preview: String,
    pub score: f64,
    pub distance: Option<f64>,
    pub kw_score: Option<f64>,
    pub graph_proximity: f64,
    pub importance: f64,
    pub realm: String,
    pub valid_at: Option<String>,
    pub invalid_at: Option<String>,
    /// `{seed, type, depth}` when this memory arrived via graph expansion.
    pub via: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkedResource {
    pub node_id: String,
    pub target_namespace: Option<String>,
    pub edge_type: String,
    pub depth: u8,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RetrieveResult {
    pub memories: Vec<RetrievedMemory>,
    pub linked: Vec<LinkedResource>,
    pub context: String,
    pub took_ms: u128,
}

// ── internal candidate ───────────────────────────────────────────────────────

#[derive(Clone)]
struct Cand {
    id: String,
    memory_type: String,
    title: Option<String>,
    content_preview: String,
    importance: f64,
    realm: String,
    created_at: Option<String>,
    valid_at: Option<String>,
    invalid_at: Option<String>,
    distance: Option<f64>,
    kw_score: Option<f64>,
    vec_rank: Option<usize>,
    kw_rank: Option<usize>,
    graph_proximity: f64,
    via: Option<Value>,
}

impl Cand {
    fn from_row(row: &Value) -> Option<Cand> {
        let id = get_str(row, "id")?;
        Some(Cand {
            id,
            memory_type: get_str(row, "memory_type").unwrap_or_default(),
            title: get_str(row, "title"),
            content_preview: get_str(row, "content_preview").unwrap_or_default(),
            importance: get_f64(row, "importance").unwrap_or(0.0),
            realm: get_str(row, "realm").unwrap_or_default(),
            created_at: get_str(row, "created_at"),
            valid_at: get_str(row, "valid_at"),
            invalid_at: get_str(row, "invalid_at"),
            distance: get_f64(row, "distance"),
            kw_score: get_f64(row, "kw_score"),
            vec_rank: None,
            kw_rank: None,
            graph_proximity: 0.0,
            via: None,
        })
    }
}

// ── orchestration ────────────────────────────────────────────────────────────

/// Run the full hybrid + graph-expanded retrieval. Never errors out: failures
/// degrade to fewer/zero results so callers (HTTP, MCP, hooks) stay simple.
pub async fn retrieve(shared: &SharedToolCtx, req: &RetrieveRequest) -> RetrieveResult {
    let started = Instant::now();
    let settings = memory_settings::load(shared.pool.as_ref(), DEFAULT_TENANT).await;
    let limit = req.limit.unwrap_or(settings.default_limit).clamp(1, 100);
    let hops = req
        .expand_hops
        .unwrap_or(settings.default_expand_hops)
        .min(MAX_HOPS);

    let writer = match build_writer(shared).await {
        Some(w) => w,
        None => return done(Vec::new(), Vec::new(), started),
    };
    if writer.ensure_provisioned().await.is_err() {
        return done(Vec::new(), Vec::new(), started);
    }
    let embed_span = tracing::info_span!(target: "kyma_telemetry", "memory.embed");
    let qvec = match writer.embed_one(&req.query).instrument(embed_span).await {
        Ok(v) => v,
        Err(_) => return done(Vec::new(), Vec::new(), started),
    };

    let filter = RecallFilter {
        realms: req.realms.clone(),
        memory_type: req.memory_type.as_deref().map(MemoryType::parse),
        tags: req.tags.clone(),
        importance_min: req.importance_min,
        as_of: req.as_of.clone(),
        include_invalidated: req.include_invalidated,
        space_agent: req.space_agent.clone(),
        ..Default::default()
    };
    let tokens = sql::tokenize_query(&req.query);

    // 1+2. Candidate generation in parallel.
    //
    // Vector leg: when the `memory_nodes.embedding` column carries an IVF+RaBitQ
    // ANN sidecar, retrieve the nearest-memory id set via `ann_topk` and run the
    // *same* semantic recall SQL restricted to those ids — preserving every
    // memory semantic (latest-version dedup, bi-temporal validity, realm/type/
    // importance/tag filters, blended score). With no sidecar (the default, and
    // every existing test), this is `None` and we use the exact SQL recall
    // verbatim — so behaviour and tests are unchanged.
    let ann = (settings.ann_threshold > 0.0).then_some(settings.ann_threshold);
    let vec_sql = match ann_candidate_ids(shared, &qvec, CAND_K).await {
        Some(ids) => sql::recall_sql_for_ids(NODE_TABLE, &qvec, &filter, CAND_K, &ids),
        None => sql::recall_sql(NODE_TABLE, &qvec, &filter, CAND_K, ann),
    };
    let vec_span = tracing::info_span!(target: "kyma_telemetry", "memory.search.vector");
    let (vec_res, kw_res) = if tokens.is_empty() {
        (
            execute_sql(shared, DEFAULT_DATABASE, &vec_sql, CAND_K)
                .instrument(vec_span)
                .await,
            json!({ "rows": [] }),
        )
    } else {
        // Keyword leg: when a tantivy BM25 sidecar covers the memory text column,
        // generate candidates by BM25 and re-apply keyword semantics over just
        // those ids; with no sidecar (the default, and every existing test) fall
        // back to the columnar `LIKE` token-set recall unchanged.
        let kw_sql = match bm25_candidate_ids(shared, &req.query, CAND_K).await {
            Some(ids) => {
                sql::keyword_recall_sql_for_ids(NODE_TABLE, &tokens, &filter, CAND_K, &ids)
            }
            None => sql::keyword_recall_sql(NODE_TABLE, &tokens, &filter, CAND_K),
        };
        let kw_span = tracing::info_span!(target: "kyma_telemetry", "memory.search.keyword");
        tokio::join!(
            execute_sql(shared, DEFAULT_DATABASE, &vec_sql, CAND_K).instrument(vec_span),
            execute_sql(shared, DEFAULT_DATABASE, &kw_sql, CAND_K).instrument(kw_span),
        )
    };

    // 3. Merge into a candidate map, recording each list's rank.
    let mut cands: HashMap<String, Cand> = HashMap::new();
    for (rank, row) in rows_of(&vec_res).iter().enumerate() {
        if let Some(mut c) = Cand::from_row(row) {
            c.vec_rank = Some(rank);
            cands.entry(c.id.clone()).or_insert(c);
        }
    }
    for (rank, row) in rows_of(&kw_res).iter().enumerate() {
        if let Some(id) = get_str(row, "id") {
            let entry = cands
                .entry(id.clone())
                .or_insert_with(|| Cand::from_row(row).unwrap_or_else(|| empty_cand(&id)));
            entry.kw_rank = Some(rank);
            if entry.kw_score.is_none() {
                entry.kw_score = get_f64(row, "kw_score");
            }
        }
    }

    // 4. Graph expansion from the top fused seeds.
    let mut linked: Vec<LinkedResource> = Vec::new();
    if hops >= 1 && !cands.is_empty() {
        let expand_span = tracing::info_span!(
            target: "kyma_telemetry",
            "memory.graph_expand",
            memory.hops = hops,
        );
        graph_expand(shared, &mut cands, &mut linked, &filter.realms, hops, limit)
            .instrument(expand_span)
            .await;
    }

    // 4b. Optional PPR rescoring (S3.4): replace the hop-decay graph_proximity
    // with personalized-PageRank mass over the candidate-induced memory
    // subgraph, seeded by the top fused candidates. Default-off
    // (KYMA_MEMORY_PPR) → candidates keep their hop-decay proximity and recall
    // is byte-identical to the pre-PPR behaviour.
    ppr_rescore(shared, &mut cands, &filter.realms).await;

    // 5. Final blend + sort.
    let kw_norm_denom = tokens.len().max(1) as f64;
    let mut scored: Vec<RetrievedMemory> = cands
        .into_values()
        .map(|c| finalize(c, kw_norm_denom, &settings))
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 5b. Optional cross-encoder rerank (S1.6): when a reranker is configured,
    // re-score the top candidates jointly against the query by their content and
    // reorder. Off by default (zero overhead); a failure keeps the blended order.
    rerank_memories(&req.query, &mut scored).await;
    scored.truncate(limit);

    // Dedup linked resources, cap for compactness.
    linked.sort_by(|a, b| a.depth.cmp(&b.depth));
    linked.dedup_by(|a, b| a.node_id == b.node_id);
    linked.truncate(50);

    done(scored, linked, started)
}

/// Expand `cands` with graph neighbours over `memory_edges`, up to `hops`.
/// Memory neighbours become candidates (with `via`); cross-graph endpoints
/// (those carrying a `target_namespace`, i.e. catalog resources / traces) are
/// recorded as `linked`.
async fn graph_expand(
    shared: &SharedToolCtx,
    cands: &mut HashMap<String, Cand>,
    linked: &mut Vec<LinkedResource>,
    realms: &[String],
    hops: u8,
    limit: usize,
) {
    // Seed with the strongest current candidates (by best rank across lists).
    let mut frontier: Vec<String> = {
        let mut ids: Vec<(&String, usize)> =
            cands.values().map(|c| (&c.id, best_rank(c))).collect();
        ids.sort_by_key(|(_, r)| *r);
        ids.into_iter()
            .take(SEED_N)
            .map(|(id, _)| id.clone())
            .collect()
    };

    let mut seen_seed: std::collections::HashSet<String> = frontier.iter().cloned().collect();

    for depth in 1..=hops {
        if frontier.is_empty() {
            break;
        }
        let sql = sql::neighbors_sql(EDGE_TABLE, &frontier, realms, EXPAND_CAP);
        let res = execute_sql(shared, DEFAULT_DATABASE, &sql, EXPAND_CAP).await;
        let frontier_set: std::collections::HashSet<&String> = frontier.iter().collect();

        let mut next: Vec<String> = Vec::new();
        let mut new_mem_ids: Vec<String> = Vec::new();
        for edge in rows_of(&res) {
            let src = get_str(&edge, "src").unwrap_or_default();
            let dst = get_str(&edge, "dst").unwrap_or_default();
            let etype = get_str(&edge, "type").unwrap_or_default();
            let tns = get_str(&edge, "target_namespace");
            // Determine the far endpoint relative to whichever side is a seed.
            let (seed, far) = if frontier_set.contains(&src) {
                (src.clone(), dst.clone())
            } else if frontier_set.contains(&dst) {
                (dst.clone(), src.clone())
            } else {
                continue;
            };
            if far.is_empty() {
                continue;
            }
            if far.starts_with("memory:") {
                if !cands.contains_key(&far) && seen_seed.insert(far.clone()) {
                    new_mem_ids.push(far.clone());
                    next.push(far.clone());
                    // Stash provenance on a placeholder; filled when materialized.
                    cands.insert(far.clone(), graph_cand(&far, &seed, &etype, depth));
                }
            } else {
                // Cross-graph endpoint: a catalog resource / trace.
                linked.push(LinkedResource {
                    node_id: far,
                    target_namespace: tns,
                    edge_type: etype,
                    depth,
                });
            }
        }

        // Materialize the new memory neighbours' display fields.
        if !new_mem_ids.is_empty() {
            let nsql = sql::nodes_by_id_sql(NODE_TABLE, &new_mem_ids);
            let nres = execute_sql(shared, DEFAULT_DATABASE, &nsql, new_mem_ids.len().max(1)).await;
            for row in rows_of(&nres) {
                if let Some(id) = get_str(&row, "id") {
                    if let Some(c) = cands.get_mut(&id) {
                        hydrate(c, &row);
                    }
                }
            }
        }

        frontier = next;
        if cands.len() > CAND_K * 4 || linked.len() > limit * 20 {
            break; // fan-out guard
        }
    }
}

// ── PPR graph proximity (S3.4) ────────────────────────────────────────────────

/// PPR restart probability and residual-stop threshold.
const PPR_ALPHA: f64 = 0.15;
const PPR_EPSILON: f64 = 1e-5;

/// Whether PPR graph-proximity rescoring is enabled. Off unless
/// `KYMA_MEMORY_PPR` is `1`/`true` — so the default recall path is unchanged
/// and every existing test/deployment is byte-identical.
fn ppr_enabled() -> bool {
    std::env::var("KYMA_MEMORY_PPR")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Personalized-PageRank proximity over a small edge set, seeded by `seeds`.
/// Returns each reached node's PageRank mass, max-normalized to `[0, 1]`. Pure
/// (no IO) so the proximity contract is unit-testable apart from the SQL glue.
fn ppr_proximity(
    edges: &[(String, String, String)],
    seeds: &[String],
) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    if edges.is_empty() || seeds.is_empty() {
        return out;
    }
    let csr = CsrGraph::build(
        edges.iter().flat_map(|(s, d, _)| [s.as_str(), d.as_str()]),
        edges.iter().map(|(s, d, t)| (s.as_str(), d.as_str(), t.as_str())),
    );
    let seed_refs: Vec<&str> = seeds.iter().map(String::as_str).collect();
    let mass = csr.personalized_pagerank(&seed_refs, PPR_ALPHA, PPR_EPSILON, TopoDir::Both);
    let max = mass.iter().map(|(_, m)| *m).fold(0.0f64, f64::max);
    if max <= 0.0 {
        return out;
    }
    for (id, m) in mass {
        out.insert(id, m / max);
    }
    out
}

/// Replace each candidate's hop-decay `graph_proximity` with its normalized PPR
/// mass over the candidate-induced memory subgraph, seeded by the top fused
/// candidates. A no-op when disabled, with fewer than two candidates, or when
/// the candidates have no internal edges (then the hop-decay proximity stands).
async fn ppr_rescore(shared: &SharedToolCtx, cands: &mut HashMap<String, Cand>, realms: &[String]) {
    if !ppr_enabled() || cands.len() < 2 {
        return;
    }
    let ids: Vec<String> = cands.keys().cloned().collect();
    let sql = sql::neighbors_sql(EDGE_TABLE, &ids, realms, EXPAND_CAP * 4);
    let res = execute_sql(shared, DEFAULT_DATABASE, &sql, EXPAND_CAP * 4).await;
    let cand_set: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut edges: Vec<(String, String, String)> = Vec::new();
    for row in rows_of(&res) {
        let src = get_str(&row, "src").unwrap_or_default();
        let dst = get_str(&row, "dst").unwrap_or_default();
        if cand_set.contains(src.as_str()) && cand_set.contains(dst.as_str()) {
            let t = get_str(&row, "type").unwrap_or_default();
            edges.push((src, dst, t));
        }
    }
    if edges.is_empty() {
        return;
    }
    // Seeds: the top fused candidates by best rank (same as graph_expand's).
    let seed_ids: Vec<String> = {
        let mut s: Vec<(&String, usize)> = cands.values().map(|c| (&c.id, best_rank(c))).collect();
        s.sort_by_key(|(_, r)| *r);
        s.iter().take(SEED_N).map(|(id, _)| (*id).clone()).collect()
    };
    let prox = ppr_proximity(&edges, &seed_ids);
    for c in cands.values_mut() {
        if let Some(&m) = prox.get(&c.id) {
            c.graph_proximity = m;
        }
    }
}

// ── reranking ────────────────────────────────────────────────────────────────

/// Candidates fed to the cross-encoder reranker (latency cap).
const RERANK_CANDIDATES: usize = 50;

/// When a reranker is configured ([`kyma_memory::shared_reranker`]), re-score the
/// blended top candidates jointly against `query` by their content and reorder
/// in place. No-op (and zero model overhead) when unset; a rerank error leaves
/// the blended order untouched.
async fn rerank_memories(query: &str, scored: &mut Vec<RetrievedMemory>) {
    let Some(reranker) = kyma_memory::shared_reranker().await else {
        return;
    };
    let cand = scored.len().min(RERANK_CANDIDATES);
    if cand == 0 {
        return;
    }
    let head: Vec<RetrievedMemory> = scored.drain(..cand).collect();
    let docs: Vec<String> = head
        .iter()
        .map(|m| {
            let title = m.title.as_deref().unwrap_or("");
            format!("{title} {}", m.content_preview).trim().to_string()
        })
        .collect();
    match reranker.rerank(query, &docs).await {
        Ok(rscores) if rscores.len() == head.len() => {
            let mut ranked: Vec<(f32, RetrievedMemory)> = rscores.into_iter().zip(head).collect();
            ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            // Reranked head first (cross-encoder order), then any tail beyond the
            // candidate cap (still in blended order).
            let reordered: Vec<RetrievedMemory> = ranked.into_iter().map(|(_, m)| m).collect();
            let mut out = reordered;
            out.append(scored);
            *scored = out;
        }
        _ => {
            // Rerank unavailable / shape mismatch → restore blended order.
            let mut out = head;
            out.append(scored);
            *scored = out;
        }
    }
}

// ── scoring ──────────────────────────────────────────────────────────────────

fn finalize(c: Cand, kw_denom: f64, s: &MemorySettings) -> RetrievedMemory {
    let rrf = c
        .vec_rank
        .map(|r| 1.0 / (s.rrf_k + r as f64))
        .unwrap_or(0.0)
        + c.kw_rank.map(|r| 1.0 / (s.rrf_k + r as f64)).unwrap_or(0.0);
    let semantic = c.distance.map(|d| (1.0 - d).clamp(0.0, 1.0)).unwrap_or(0.0);
    let keyword = c
        .kw_score
        .map(|k| (k / kw_denom).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    // Recency term: class-aware decay when enabled (episodic memories fade with
    // a 7-day half-life; semantic/procedural don't decay → 1.0), else the global
    // half-life recency. Default-off ⇒ unchanged ranking.
    let recency = if class_decay_enabled() {
        let class = MemoryClass::default_for(MemoryType::parse(&c.memory_type));
        c.created_at
            .as_deref()
            .and_then(age_days)
            .map(|a| class.decay_weight(a))
            .unwrap_or(1.0)
    } else {
        c.created_at
            .as_deref()
            .map(|t| recency_decay(t, s.half_life_days))
            .unwrap_or(0.5)
    };
    let score = s.w_rrf * rrf
        + s.w_semantic * semantic
        + s.w_keyword * keyword
        + s.w_graph * c.graph_proximity
        + s.w_importance * c.importance
        + s.w_recency * recency;
    RetrievedMemory {
        id: c.id,
        memory_type: c.memory_type,
        title: c.title,
        content_preview: c.content_preview,
        score,
        distance: c.distance,
        kw_score: c.kw_score,
        graph_proximity: c.graph_proximity,
        importance: c.importance,
        realm: c.realm,
        valid_at: c.valid_at,
        invalid_at: c.invalid_at,
        via: c.via,
    }
}

/// Whether class-aware recency decay is enabled. Off unless
/// `KYMA_MEMORY_CLASS_DECAY` is `1`/`true` — so the default recency term (a
/// uniform half-life) is unchanged and every existing test/deployment matches.
fn class_decay_enabled() -> bool {
    std::env::var("KYMA_MEMORY_CLASS_DECAY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Age in days from an RFC3339 timestamp (`>= 0`); `None` if unparseable.
fn age_days(created_at: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(created_at).ok().map(|dt| {
        ((chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86_400.0)
            .max(0.0)
    })
}

/// `exp(-ln2 * age_days / half_life_days)`, clamped to [0,1]. Unparseable → 0.5.
fn recency_decay(created_at: &str, half_life_days: f64) -> f64 {
    let hl = if half_life_days > 0.0 {
        half_life_days
    } else {
        30.0
    };
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(dt) => {
            let age_days = (chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_seconds()
                as f64
                / 86_400.0;
            if age_days <= 0.0 {
                1.0
            } else {
                (-std::f64::consts::LN_2 * age_days / hl)
                    .exp()
                    .clamp(0.0, 1.0)
            }
        }
        Err(_) => 0.5,
    }
}

// ── ANN candidate retrieval ───────────────────────────────────────────────────

/// Over-fetch factor: ANN retrieves `OVERSAMPLE · k` candidate ids so the
/// downstream filter+dedup SQL has enough survivors to fill `k`.
const ANN_OVERSAMPLE: usize = 4;

/// When the `memory_nodes.embedding` column has an IVF+RaBitQ ANN sidecar,
/// return the candidate memory ids nearest to `qvec` (over-fetched). The caller
/// re-applies all memory semantics via [`sql::recall_sql_for_ids`]. Returns
/// `None` when there is no object store, no sidecar, or any hard error — the
/// caller then uses the exact SQL recall unchanged.
async fn ann_candidate_ids(shared: &SharedToolCtx, qvec: &[f32], k: usize) -> Option<Vec<String>> {
    let store = shared.format.object_store()?;
    let tref = shared
        .catalog
        .lookup_table_in_tenant(DEFAULT_TENANT, DEFAULT_DATABASE, NODE_TABLE)
        .await
        .ok()?;

    // Capability gate: ≥1 IvfRabitq sidecar on the embedding column.
    let extents = shared
        .catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            tref.id,
            tref.current_snapshot_id,
            &kyma_core::catalog::PrunePredicate::default(),
        )
        .await
        .ok()?;
    if extents.is_empty() {
        return None;
    }
    let extent_ids: Vec<_> = extents.iter().map(|m| m.id).collect();
    let sidecars = shared
        .catalog
        .list_index_sidecars(
            DEFAULT_TENANT,
            tref.id,
            &extent_ids,
            Some(kyma_core::index_sidecar::SidecarKind::IvfRabitq),
        )
        .await
        .ok()?;
    if !sidecars.iter().any(|d| d.column == "embedding") {
        return None;
    }

    let cache = ann_sidecar_cache();
    let params = kyma_exec::AnnParams::with_k(k.saturating_mul(ANN_OVERSAMPLE).max(k));
    let hits = kyma_exec::ann_topk(
        &shared.catalog,
        DEFAULT_TENANT,
        &shared.format,
        &store,
        cache,
        &tref,
        "embedding",
        qvec,
        &params,
        None,
    )
    .await
    .ok()?;
    if hits.is_empty() {
        // Sidecar exists but nothing came back (e.g. empty table) — let the SQL
        // path run rather than forcing an empty id-restricted result.
        return None;
    }

    // Resolve each hit's `id` column value by reading its block once.
    let addrs: Vec<_> = hits
        .iter()
        .map(|h| (h.extent_id, h.addr.block.0, h.addr.row))
        .collect();
    resolve_addr_ids(shared, &tref, &extents, &addrs).await
}

// ── BM25 candidate retrieval ──────────────────────────────────────────────────

/// When the memory text column carries a tantivy BM25 (`TantivyFts`) sidecar,
/// return the candidate memory ids ranked by BM25 for `query` (over-fetched by
/// [`ANN_OVERSAMPLE`]). The caller re-applies all memory semantics via
/// [`sql::keyword_recall_sql_for_ids`]. Returns `None` when there is no object
/// store, no sidecar, an empty query, or any hard error — the caller then uses
/// the columnar `LIKE` keyword recall verbatim, so behaviour and every existing
/// test (no sidecar) are unchanged.
async fn bm25_candidate_ids(shared: &SharedToolCtx, query: &str, k: usize) -> Option<Vec<String>> {
    if query.trim().is_empty() {
        return None;
    }
    let store = shared.format.object_store()?;
    let tref = shared
        .catalog
        .lookup_table_in_tenant(DEFAULT_TENANT, DEFAULT_DATABASE, NODE_TABLE)
        .await
        .ok()?;

    // Capability gate: ≥1 TantivyFts sidecar (first indexed text column).
    let extents = shared
        .catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            tref.id,
            tref.current_snapshot_id,
            &kyma_core::catalog::PrunePredicate::default(),
        )
        .await
        .ok()?;
    if extents.is_empty() {
        return None;
    }
    let extent_ids: Vec<_> = extents.iter().map(|m| m.id).collect();
    let sidecars = shared
        .catalog
        .list_index_sidecars(
            DEFAULT_TENANT,
            tref.id,
            &extent_ids,
            Some(kyma_core::index_sidecar::SidecarKind::TantivyFts),
        )
        .await
        .ok()?;
    let fts_col = sidecars.first().map(|d| d.column.clone())?;

    let cache = ann_sidecar_cache();
    let hits = match kyma_exec::bm25_topk(
        &shared.catalog,
        DEFAULT_TENANT,
        &store,
        cache,
        &tref,
        &fts_col,
        query,
        k.saturating_mul(ANN_OVERSAMPLE).max(k),
        None,
    )
    .await
    {
        Ok(Some(h)) => h,
        // No coverage or a hard error → LIKE fallback rather than an empty set.
        Ok(None) | Err(_) => return None,
    };
    if hits.is_empty() {
        return None;
    }

    let addrs: Vec<_> = hits
        .iter()
        .map(|h| (h.extent_id, h.addr.block.0, h.addr.row))
        .collect();
    resolve_addr_ids(shared, &tref, &extents, &addrs).await
}

/// Resolve the `id` column value for a set of `(extent, block, row)` addresses,
/// reading each referenced block once. Order-agnostic: both candidate-id callers
/// re-rank in SQL ([`sql::recall_sql_for_ids`] by distance,
/// [`sql::keyword_recall_sql_for_ids`] by kw_score), so only set membership
/// matters. Returns `None` on any hard error or an empty result.
async fn resolve_addr_ids(
    shared: &SharedToolCtx,
    tref: &kyma_core::catalog::TableRef,
    extents: &[kyma_core::catalog::ExtentManifest],
    addrs: &[(kyma_core::types::ExtentId, u32, u32)],
) -> Option<Vec<String>> {
    let manifest_by_id: HashMap<_, _> = extents.iter().map(|m| (m.id, m)).collect();
    let id_col = kyma_core::segment_format::ColumnId(
        tref.schema.fields().iter().position(|f| f.name() == "id")? as u32,
    );
    let mut by_block: HashMap<(kyma_core::types::ExtentId, u32), Vec<u32>> = HashMap::new();
    for (eid, block, row) in addrs {
        by_block.entry((*eid, *block)).or_default().push(*row);
    }
    let mut ids: Vec<String> = Vec::with_capacity(addrs.len());
    let mut readers: HashMap<
        kyma_core::types::ExtentId,
        std::sync::Arc<dyn kyma_core::segment_format::ExtentReader>,
    > = HashMap::new();
    for ((extent_id, block), rows) in by_block {
        let manifest = manifest_by_id.get(&extent_id)?;
        let reader = match readers.get(&extent_id) {
            Some(r) => r.clone(),
            None => {
                let r = shared
                    .format
                    .open_extent(kyma_core::segment_format::OpenExtentInput {
                        extent_id,
                        table_id: manifest.table_id,
                        schema: tref.schema.clone(),
                        object_path: manifest.object_path.clone(),
                        byte_size: manifest.byte_size,
                    })
                    .await
                    .ok()?;
                readers.insert(extent_id, r.clone());
                r
            }
        };
        let batch = reader
            .read_block(kyma_core::segment_format::BlockId(block), &[id_col])
            .await
            .ok()?;
        use arrow_array::Array as _;
        let col = batch.column(0);
        let arr = col.as_any().downcast_ref::<arrow_array::StringArray>();
        for row in rows {
            let r = row as usize;
            if r >= batch.num_rows() {
                continue;
            }
            if let Some(a) = arr {
                if !a.is_null(r) {
                    ids.push(a.value(r).to_string());
                }
            }
        }
    }
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// Process-shared sidecar disk cache for the memory ANN path.
fn ann_sidecar_cache() -> &'static kyma_storage::sidecar_cache::SidecarCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<kyma_storage::sidecar_cache::SidecarCache> = OnceLock::new();
    CACHE.get_or_init(kyma_storage::sidecar_cache::SidecarCache::from_env)
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn build_writer(shared: &SharedToolCtx) -> Option<MemoryWriter> {
    let embed = kyma_memory::shared_embedding().await.ok()?;
    Some(MemoryWriter::new(
        shared.catalog.clone(),
        shared.format.clone(),
        embed,
    ))
}

fn rows_of(v: &Value) -> Vec<Value> {
    v.get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn get_str(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_string)
}

fn get_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

fn best_rank(c: &Cand) -> usize {
    c.vec_rank
        .into_iter()
        .chain(c.kw_rank)
        .min()
        .unwrap_or(usize::MAX)
}

fn empty_cand(id: &str) -> Cand {
    Cand {
        id: id.to_string(),
        memory_type: String::new(),
        title: None,
        content_preview: String::new(),
        importance: 0.0,
        realm: String::new(),
        created_at: None,
        valid_at: None,
        invalid_at: None,
        distance: None,
        kw_score: None,
        vec_rank: None,
        kw_rank: None,
        graph_proximity: 0.0,
        via: None,
    }
}

fn graph_cand(id: &str, seed: &str, etype: &str, depth: u8) -> Cand {
    let mut c = empty_cand(id);
    c.graph_proximity = 1.0 / (1.0 + depth as f64);
    c.via = Some(json!({ "seed": seed, "type": etype, "depth": depth }));
    c
}

/// Fill a graph-pulled candidate's display fields from its materialized row.
fn hydrate(c: &mut Cand, row: &Value) {
    c.memory_type = get_str(row, "memory_type").unwrap_or_default();
    c.title = get_str(row, "title");
    c.content_preview = get_str(row, "content_preview").unwrap_or_default();
    c.importance = get_f64(row, "importance").unwrap_or(0.0);
    c.realm = get_str(row, "realm").unwrap_or_default();
    c.created_at = get_str(row, "created_at");
    c.valid_at = get_str(row, "valid_at");
    c.invalid_at = get_str(row, "invalid_at");
}

fn done(
    memories: Vec<RetrievedMemory>,
    linked: Vec<LinkedResource>,
    started: Instant,
) -> RetrieveResult {
    let context = build_context(&memories, &linked);
    RetrieveResult {
        memories,
        linked,
        context,
        took_ms: started.elapsed().as_millis(),
    }
}

/// Compact, deterministic, citation-rich context block — LLM-free, suitable
/// for injecting into an agent prompt or a Claude Code SessionStart hook.
fn build_context(memories: &[RetrievedMemory], linked: &[LinkedResource]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut out = String::from("Relevant memories:\n");
    for m in memories {
        let validity = match (&m.invalid_at, &m.valid_at) {
            (Some(inv), _) => format!(" (invalidated {inv})"),
            (None, Some(v)) => format!(" (since {v})"),
            _ => String::new(),
        };
        let via = m
            .via
            .as_ref()
            .and_then(|v| v.get("type").and_then(Value::as_str))
            .map(|t| format!(" [via {t}]"))
            .unwrap_or_default();
        out.push_str(&format!(
            "- [{}] {}{}{} (score {:.2}) {}\n",
            m.memory_type, m.content_preview, validity, via, m.score, m.id
        ));
    }
    if !linked.is_empty() {
        out.push_str("\nConnected resources/traces:\n");
        for l in linked.iter().take(20) {
            let ns = l.target_namespace.as_deref().unwrap_or("");
            out.push_str(&format!("- {} ({}) via {}\n", l.node_id, ns, l.edge_type));
        }
    }
    out
}

impl RetrieveResult {
    /// JSON envelope for the HTTP API + MCP tool.
    pub fn to_json(&self) -> Value {
        json!({
            "memories": self.memories,
            "linked": self.linked,
            "context": self.context,
            "took_ms": self.took_ms,
        })
    }
}

#[cfg(test)]
mod ppr_tests {
    use super::*;

    #[test]
    fn ppr_proximity_concentrates_near_the_seed() {
        let edges = vec![
            ("a".to_string(), "b".to_string(), "R".to_string()),
            ("b".to_string(), "c".to_string(), "R".to_string()),
            ("c".to_string(), "d".to_string(), "R".to_string()),
        ];
        let prox = ppr_proximity(&edges, &["a".to_string()]);
        // Every node reached, normalized into [0, 1] with a max of exactly 1.0.
        assert_eq!(prox.len(), 4, "{prox:?}");
        assert!(prox.values().all(|m| (0.0..=1.0).contains(m)), "{prox:?}");
        assert!(
            (prox.values().cloned().fold(0.0_f64, f64::max) - 1.0).abs() < 1e-9,
            "max-normalized: {prox:?}"
        );
        // The seed carries more mass than the farthest node.
        assert!(prox["a"] > prox["d"], "seed-local mass: {prox:?}");
    }

    #[test]
    fn ppr_proximity_empty_inputs_are_empty() {
        assert!(ppr_proximity(&[], &["a".to_string()]).is_empty());
        assert!(ppr_proximity(
            &[("a".to_string(), "b".to_string(), "R".to_string())],
            &[]
        )
        .is_empty());
    }

    #[test]
    fn ppr_disabled_by_default() {
        // The default recall path must be unchanged: PPR only runs when the
        // env flag is explicitly set (not set in the test environment).
        assert!(!ppr_enabled());
    }

    #[test]
    fn class_decay_disabled_by_default() {
        // The recency term keeps its uniform half-life unless the flag is set.
        assert!(!class_decay_enabled());
    }

    #[test]
    fn age_days_parses_rfc3339_and_rejects_garbage() {
        let past = (chrono::Utc::now() - chrono::Duration::days(2)).to_rfc3339();
        let a = age_days(&past).expect("parses");
        assert!((1.9..2.1).contains(&a), "≈2 days, got {a}");
        assert!(age_days("not-a-date").is_none());
    }

    #[test]
    fn class_aware_decay_fades_episodic_not_semantic() {
        // The contract the recency term uses when enabled: episodic memories
        // (e.g. Summary) decay with a 7-day half-life; semantic ones (Fact) do
        // not. (default_for/decay_weight live in kyma-memory; this pins the
        // mapping the scorer relies on.)
        let episodic = MemoryClass::default_for(MemoryType::parse("summary"));
        let semantic = MemoryClass::default_for(MemoryType::parse("fact"));
        assert!(
            (episodic.decay_weight(7.0) - 0.5).abs() < 1e-9,
            "episodic at one half-life → 0.5"
        );
        assert_eq!(semantic.decay_weight(7.0), 1.0, "semantic never decays");
        assert_eq!(episodic.decay_weight(0.0), 1.0, "age 0 → full weight");
    }
}
