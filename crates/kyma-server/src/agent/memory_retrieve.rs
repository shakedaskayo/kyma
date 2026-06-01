//! Graph-aware hybrid memory retrieval — the near-realtime "find anything"
//! read path. No LLM in the hot path.
//!
//! Pipeline (all over kyma's own engine):
//! 1. embed the query once;
//! 2. generate candidates two ways IN PARALLEL — semantic (vector
//!    `cosine_distance`) and keyword (token-set `LIKE`, which the columnar
//!    engine prunes via `column_stats`);
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

use kyma_core::tenant::DEFAULT_TENANT;
use kyma_memory::types::{MemoryType, RecallFilter};
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
    let settings = memory_settings::load(&shared.pool, DEFAULT_TENANT).await;
    let limit = req.limit.unwrap_or(settings.default_limit).clamp(1, 100);
    let hops = req.expand_hops.unwrap_or(settings.default_expand_hops).min(MAX_HOPS);

    let writer = match build_writer(shared).await {
        Some(w) => w,
        None => return done(Vec::new(), Vec::new(), started),
    };
    if writer.ensure_provisioned().await.is_err() {
        return done(Vec::new(), Vec::new(), started);
    }
    let qvec = match writer.embed_one(&req.query).await {
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
        ..Default::default()
    };
    let tokens = sql::tokenize_query(&req.query);

    // 1+2. Candidate generation in parallel.
    let ann = (settings.ann_threshold > 0.0).then_some(settings.ann_threshold);
    let vec_sql = sql::recall_sql(NODE_TABLE, &qvec, &filter, CAND_K, ann);
    let (vec_res, kw_res) = if tokens.is_empty() {
        (
            execute_sql(shared, DEFAULT_DATABASE, &vec_sql, CAND_K).await,
            json!({ "rows": [] }),
        )
    } else {
        let kw_sql = sql::keyword_recall_sql(NODE_TABLE, &tokens, &filter, CAND_K);
        tokio::join!(
            execute_sql(shared, DEFAULT_DATABASE, &vec_sql, CAND_K),
            execute_sql(shared, DEFAULT_DATABASE, &kw_sql, CAND_K),
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
            let entry = cands.entry(id.clone()).or_insert_with(|| {
                Cand::from_row(row).unwrap_or_else(|| empty_cand(&id))
            });
            entry.kw_rank = Some(rank);
            if entry.kw_score.is_none() {
                entry.kw_score = get_f64(row, "kw_score");
            }
        }
    }

    // 4. Graph expansion from the top fused seeds.
    let mut linked: Vec<LinkedResource> = Vec::new();
    if hops >= 1 && !cands.is_empty() {
        graph_expand(shared, &mut cands, &mut linked, &filter.realms, hops, limit).await;
    }

    // 5. Final blend + sort.
    let kw_norm_denom = tokens.len().max(1) as f64;
    let mut scored: Vec<RetrievedMemory> = cands
        .into_values()
        .map(|c| finalize(c, kw_norm_denom, &settings))
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
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
        let mut ids: Vec<(&String, usize)> = cands
            .values()
            .map(|c| (&c.id, best_rank(c)))
            .collect();
        ids.sort_by_key(|(_, r)| *r);
        ids.into_iter().take(SEED_N).map(|(id, _)| id.clone()).collect()
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
                    cands.insert(
                        far.clone(),
                        graph_cand(&far, &seed, &etype, depth),
                    );
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

// ── scoring ──────────────────────────────────────────────────────────────────

fn finalize(c: Cand, kw_denom: f64, s: &MemorySettings) -> RetrievedMemory {
    let rrf = c.vec_rank.map(|r| 1.0 / (s.rrf_k + r as f64)).unwrap_or(0.0)
        + c.kw_rank.map(|r| 1.0 / (s.rrf_k + r as f64)).unwrap_or(0.0);
    let semantic = c.distance.map(|d| (1.0 - d).clamp(0.0, 1.0)).unwrap_or(0.0);
    let keyword = c.kw_score.map(|k| (k / kw_denom).clamp(0.0, 1.0)).unwrap_or(0.0);
    let recency = c
        .created_at
        .as_deref()
        .map(|t| recency_decay(t, s.half_life_days))
        .unwrap_or(0.5);
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

/// `exp(-ln2 * age_days / half_life_days)`, clamped to [0,1]. Unparseable → 0.5.
fn recency_decay(created_at: &str, half_life_days: f64) -> f64 {
    let hl = if half_life_days > 0.0 { half_life_days } else { 30.0 };
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(dt) => {
            let age_days =
                (chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_seconds() as f64 / 86_400.0;
            if age_days <= 0.0 {
                1.0
            } else {
                (-std::f64::consts::LN_2 * age_days / hl).exp().clamp(0.0, 1.0)
            }
        }
        Err(_) => 0.5,
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn build_writer(shared: &SharedToolCtx) -> Option<MemoryWriter> {
    let embed = kyma_memory::shared_embedding().await.ok()?;
    Some(MemoryWriter::new(
        shared.catalog.clone(),
        shared.format.clone(),
        shared.pool.clone(),
        embed,
    ))
}

fn rows_of(v: &Value) -> Vec<Value> {
    v.get("rows").and_then(Value::as_array).cloned().unwrap_or_default()
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

fn done(memories: Vec<RetrievedMemory>, linked: Vec<LinkedResource>, started: Instant) -> RetrieveResult {
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
