//! Bidirectional memory sync between this local engine and a hosted control
//! plane (Phase B). Memory stays coherent across machines; bulk data stays local.
//!
//! - **Push** (local → cloud): local `memory_nodes`/`memory_edges` changed since
//!   the push watermark are POSTed as NDJSON to the plane's idempotent
//!   `POST /v1/ingest` (`X-Database: memory`). The plane's latest-wins /
//!   A.U.D.N. consolidation reconciles versions.
//! - **Pull** (cloud → local): `GET /v1/agent/memory/changes?since=` returns the
//!   plane's changed rows, appended into the local store (latest-wins on recall).
//!
//! Push runs first and advances its watermark to "now", so rows just pulled
//! (which carry their origin timestamp ≤ now) are not echoed back next run.
//! Watermarks persist in the local catalog's `sync_state` table.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::Engine;
use kyma_memory::MemoryWriter;
use kyma_server::agent::{execute_sql, SharedToolCtx};

pub(crate) const NODE_COLS: &str =
    "id, labels, realm, memory_type, title, content, content_preview, tags, \
    importance, status, source_session_id, source_run_id, embedding, created_at, updated_at, \
    valid_at, invalid_at, superseded_by, provenance, topic_key";
const EDGE_COLS: &str = "id, src, dst, type, realm, target_namespace, props, created_at";
const PUSH_WM_KEY: &str = "memory_push_watermark";
const PULL_WM_KEY: &str = "memory_pull_watermark";
const EPOCH: &str = "1970-01-01T00:00:00Z";

/// Sync configuration, resolved from env by the caller.
pub struct SyncConfig {
    pub cloud_url: String,
    pub token: Option<String>,
    pub realm: Option<String>,
    /// Stamp for the new push watermark (server/local "now" at sync start).
    pub now: String,
}

/// Run one bidirectional sync pass (push, then pull).
pub async fn run(engine: &Engine, cfg: SyncConfig) -> Result<()> {
    let base = cfg.cloud_url.trim_end_matches('/').to_string();
    let client = reqwest::Client::new();
    let shared = SharedToolCtx { federation: None,
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
        memory: None,
        hitl: None,
        memory_settings_path: None,
    };
    let realm_sql = cfg
        .realm
        .as_deref()
        .map(|r| format!(" AND realm = '{}'", r.replace('\'', "''")))
        .unwrap_or_default();

    // ── PUSH: local → cloud ───────────────────────────────────────────────
    let push_wm = engine
        .sqlite
        .get_sync_state(PUSH_WM_KEY)
        .await?
        .unwrap_or_else(|| EPOCH.to_string());
    let push_esc = push_wm.replace('\'', "''");
    let nodes_sql = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM memory_nodes) \
         SELECT {NODE_COLS} FROM latest WHERE __rn = 1 AND updated_at > '{push_esc}'{realm_sql}"
    );
    let edges_sql =
        format!("SELECT {EDGE_COLS} FROM memory_edges WHERE created_at > '{push_esc}'{realm_sql}");
    let local_nodes = sql_rows(&shared, &nodes_sql).await;
    let local_edges = sql_rows(&shared, &edges_sql).await;
    let (pushed_n, pushed_e) = push(&client, &base, &cfg, &local_nodes, &local_edges).await?;
    // Advance the push watermark even when nothing changed, so the window moves.
    engine.sqlite.set_sync_state(PUSH_WM_KEY, &cfg.now).await?;

    // ── PULL: cloud → local ───────────────────────────────────────────────
    let pull_wm = engine
        .sqlite
        .get_sync_state(PULL_WM_KEY)
        .await?
        .unwrap_or_else(|| EPOCH.to_string());
    let mut req = client
        .get(format!("{base}/v1/agent/memory/changes"))
        .query(&[("since", pull_wm.as_str())]);
    if let Some(r) = &cfg.realm {
        req = req.query(&[("realm", r.as_str())]);
    }
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.context("GET /v1/agent/memory/changes")?;
    let resp = resp
        .error_for_status()
        .context("memory/changes returned an error status")?;
    let body: Value = resp.json().await.context("decoding changes response")?;
    let nodes = body["memory_nodes"].as_array().cloned().unwrap_or_default();
    let edges = body["memory_edges"].as_array().cloned().unwrap_or_default();
    let until = body["until"].as_str().unwrap_or("").to_string();

    if !nodes.is_empty() || !edges.is_empty() {
        let embed = kyma_memory::shared_embedding()
            .await
            .map_err(|e| anyhow!("embedding backend: {e}"))?;
        let writer = MemoryWriter::new(engine.catalog.clone(), engine.format.clone(), embed);
        if !nodes.is_empty() {
            writer
                .append_node_rows(nodes.clone())
                .await
                .map_err(|e| anyhow!("apply pulled nodes: {e}"))?;
        }
        if !edges.is_empty() {
            writer
                .append_edge_rows(edges.clone())
                .await
                .map_err(|e| anyhow!("apply pulled edges: {e}"))?;
        }
    }
    if !until.is_empty() {
        engine.sqlite.set_sync_state(PULL_WM_KEY, &until).await?;
    }

    eprintln!(
        "sync ok — pushed {pushed_n} nodes / {pushed_e} edges; pulled {} nodes / {} edges (cloud: {base})",
        nodes.len(),
        edges.len()
    );
    Ok(())
}

/// Run a read query over the local memory db, returning its rows (or empty on
/// any error — e.g. the memory store doesn't exist yet).
async fn sql_rows(shared: &SharedToolCtx, sql: &str) -> Vec<Value> {
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, sql, 1_000_000).await;
    res.get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Apply local node/edge rows to the plane via `/v1/agent/memory/import`, which
/// writes through the MemoryWriter (canonical schema + embeddings) — not generic
/// `/v1/ingest`, which would infer every column as text. Returns (nodes, edges)
/// applied.
async fn push(
    client: &reqwest::Client,
    base: &str,
    cfg: &SyncConfig,
    nodes: &[Value],
    edges: &[Value],
) -> Result<(usize, usize)> {
    if nodes.is_empty() && edges.is_empty() {
        return Ok((0, 0));
    }
    let body = serde_json::json!({ "memory_nodes": nodes, "memory_edges": edges });
    let mut req = client
        .post(format!("{base}/v1/agent/memory/import"))
        .json(&body);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.context("POST /v1/agent/memory/import")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("memory/import returned {status}: {text}"));
    }
    // Surface server-side per-row errors (e.g. schema mismatch) rather than
    // silently reporting success.
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        if let Some(errs) = v.get("errors").and_then(Value::as_array) {
            if !errs.is_empty() {
                return Err(anyhow!("memory/import errors: {errs:?}"));
            }
        }
    }
    Ok((nodes.len(), edges.len()))
}
