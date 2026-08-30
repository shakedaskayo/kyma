//! HTTP handler for the manual compaction trigger.
//!
//! Exposes:
//!   POST /v1/admin/compact   body (all optional): {
//!     "database": "default", "table": "claude_code_events",
//!     "max_merge": 32, "small_bytes": 67108864, "wait": true }
//!
//! Submits `compaction` tasks for the small live extents of the targeted
//! table(s) so the in-process compaction worker merges them. Unlike the gentle
//! background scheduler (which submits only the smallest `max_merge` per table
//! per tick), this drains the whole backlog: every small extent is chunked into
//! `max_merge`-sized groups, one task per group. Scoped to one table when
//! `table` is given, else every table (optionally within one `database`).
//!
//! Requires `Role::Write` — data maintenance, same class as `cleanup`.
//!
//! Note: compaction *soft-deletes* the source extents, so the slow
//! `… deleted_at IS NULL` scans shrink immediately; physical disk is reclaimed
//! by a separate GC/retention pass.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pensieve_core::catalog::{Catalog, PrunePredicate, TableRef};
use pensieve_core::errors::Error as PensieveError;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_MAX_MERGE: usize = 32;
const DEFAULT_SMALL_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB
const WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const WAIT_POLL: Duration = Duration::from_secs(2);

/// Shared state — just the catalog (task submission + extent listing).
#[derive(Clone)]
pub struct CompactState {
    pub catalog: Arc<dyn Catalog>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CompactRequest {
    /// Database to scope to. Defaults to `default` when `table` is set, or all
    /// databases when both are omitted.
    pub database: Option<String>,
    /// Table to compact. When omitted, every table in scope is compacted.
    pub table: Option<String>,
    /// Max extents merged per task (memory bound: the worker reads this many
    /// extents into memory). Clamped to [2, 256]. Default 32.
    pub max_merge: Option<usize>,
    /// Only extents smaller than this are compaction candidates. Default 64 MiB.
    pub small_bytes: Option<u64>,
    /// Block until the live extent count stops dropping (or 120 s). Default false.
    #[serde(default)]
    pub wait: bool,
}

/// `POST /v1/admin/compact`
pub async fn compact(
    State(state): State<CompactState>,
    Json(req): Json<CompactRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cat = &state.catalog;
    let max_merge = req.max_merge.unwrap_or(DEFAULT_MAX_MERGE).clamp(2, 256);
    let small_bytes = req.small_bytes.unwrap_or(DEFAULT_SMALL_BYTES);

    // (database, table) name pairs — resolved fresh each time we need a current
    // snapshot, since compaction advances snapshots as it commits.
    let target_names: Vec<(String, String)> = match &req.table {
        Some(t) => {
            let db = req.database.clone().unwrap_or_else(|| "default".into());
            vec![(db, t.clone())]
        }
        None => {
            let dbs = match &req.database {
                Some(d) => vec![d.clone()],
                None => cat.list_databases().await.map_err(PensieveError::from)?,
            };
            let mut out = Vec::new();
            for d in dbs {
                for t in cat.list_tables(&d).await.map_err(PensieveError::from)? {
                    out.push((d.clone(), t));
                }
            }
            out
        }
    };

    let before = live_extent_count(cat, &target_names).await?;

    let mut tasks_submitted = 0usize;
    let mut extents_queued = 0usize;
    for tref in resolve(cat, &target_names).await {
        let mut exts = cat
            .list_extents(tref.id, tref.current_snapshot_id, &PrunePredicate::default())
            .await?;
        exts.retain(|e| e.byte_size < small_bytes);
        exts.sort_by_key(|e| e.byte_size); // merge the smallest first
        for chunk in exts.chunks(max_merge) {
            if chunk.len() < 2 {
                continue; // a single extent has nothing to merge with
            }
            let ids: Vec<String> = chunk.iter().map(|e| e.id.as_uuid().to_string()).collect();
            // CompactionPayload shape (pensieve-compaction); built inline to avoid a dep.
            let payload = serde_json::json!({ "source_extent_ids": ids });
            cat.submit_task("compaction", Some(tref.id), payload, 0).await?;
            tasks_submitted += 1;
            extents_queued += chunk.len();
        }
    }

    let after = if req.wait && tasks_submitted > 0 {
        wait_for_drain(cat, &target_names).await?
    } else {
        before
    };

    Ok(Json(serde_json::json!({
        "tasks_submitted": tasks_submitted,
        "extents_queued": extents_queued,
        "tables": target_names.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(),
        "live_extents_before": before,
        "live_extents_after": after,
        "waited": req.wait,
        "note": "compaction soft-deletes source extents; physical disk is reclaimed by a separate GC pass",
    })))
}

/// Re-resolve target tables to current snapshots (compaction advances them).
async fn resolve(cat: &Arc<dyn Catalog>, names: &[(String, String)]) -> Vec<TableRef> {
    let mut out = Vec::with_capacity(names.len());
    for (db, table) in names {
        if let Ok(tref) = cat.lookup_table(db, table).await {
            out.push(tref);
        }
    }
    out
}

async fn live_extent_count(
    cat: &Arc<dyn Catalog>,
    names: &[(String, String)],
) -> Result<usize, ApiError> {
    let mut n = 0;
    for tref in resolve(cat, names).await {
        n += cat
            .list_extents(tref.id, tref.current_snapshot_id, &PrunePredicate::default())
            .await?
            .len();
    }
    Ok(n)
}

/// Poll until the live extent count stops dropping for two consecutive polls
/// (drained / plateaued) or `WAIT_TIMEOUT` elapses. Returns the final count.
async fn wait_for_drain(
    cat: &Arc<dyn Catalog>,
    names: &[(String, String)],
) -> Result<usize, ApiError> {
    let start = Instant::now();
    let mut last = usize::MAX;
    let mut stable = 0u8;
    loop {
        tokio::time::sleep(WAIT_POLL).await;
        let n = live_extent_count(cat, names).await?;
        if n == last {
            stable += 1;
        } else {
            stable = 0;
            last = n;
        }
        if stable >= 2 || start.elapsed() >= WAIT_TIMEOUT {
            return Ok(n);
        }
    }
}

// -------------------------------------------------------------------------
// Error type (mirrors cleanup_handler)
// -------------------------------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    Catalog(PensieveError),
}

impl From<PensieveError> for ApiError {
    fn from(e: PensieveError) -> Self {
        ApiError::Catalog(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let ApiError::Catalog(e) = self;
        use pensieve_core::errors::CatalogError;
        match e {
            PensieveError::Catalog(CatalogError::TableNotFound { database, name }) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("table '{database}'.'{name}' not found") })),
            )
                .into_response(),
            other => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": other.to_string() })),
            )
                .into_response(),
        }
    }
}
