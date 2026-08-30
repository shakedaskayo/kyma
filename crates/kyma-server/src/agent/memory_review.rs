//! HTTP handlers for the memory approval queue (the Review inbox).
//!
//! `GET  /v1/agent/memory/review`            — list queue rows (filterable)
//! `GET  /v1/agent/memory/review/count`      — pending / post-hoc counts (badge)
//! `POST /v1/agent/memory/review/:id/:action`— approve | reject | undo one row
//! `POST /v1/agent/memory/review/bulk`       — apply an action to many rows
//!
//! List/count are read-role; the mutating actions require write role (they
//! commit or roll back real memory mutations).

use std::sync::Arc;

use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{Principal, Role};

use super::memory_gate::{self, HitlGate, OpPayload, ResolveAction};
use super::memory_policy::MemoryOp;
use super::memory_queue_store::{QueueFilter, QueueStore};
use super::state::AgentState;
use super::tools::SharedToolCtx;

#[derive(Debug, Deserialize, Default)]
pub struct ReviewQuery {
    pub status: Option<String>,
    pub source: Option<String>,
    pub realm: Option<String>,
    pub op: Option<String>,
    pub source_run_id: Option<Uuid>,
    pub limit: Option<usize>,
}

/// `GET /v1/agent/memory/review` — list queue rows, newest first.
pub async fn list_review(
    State(state): State<AgentState>,
    Query(q): Query<ReviewQuery>,
) -> Response {
    let Some(store) = QueueStore::from_state(&state) else {
        return Json(json!({ "items": [] })).into_response();
    };
    let filter = QueueFilter {
        status: q.status,
        source: q.source,
        realm: q.realm,
        operation: q.op.as_deref().and_then(MemoryOp::parse),
        source_run_id: q.source_run_id,
        limit: q.limit.unwrap_or(0),
    };
    match store.list(&filter).await {
        Ok(items) => Json(json!({ "items": items })).into_response(),
        Err(e) => internal(e.to_string()),
    }
}

/// `GET /v1/agent/memory/review/count` — counts for the nav badge.
pub async fn review_count(State(state): State<AgentState>) -> Json<Value> {
    let counts = match QueueStore::from_state(&state) {
        Some(store) => store.counts().await.unwrap_or_default(),
        None => Default::default(),
    };
    Json(json!({ "pending": counts.pending, "post_hoc": counts.post_hoc }))
}

#[derive(Debug, Deserialize, Default)]
pub struct ResolveBody {
    #[serde(default)]
    pub payload: Option<OpPayload>,
    #[serde(default)]
    pub comment: Option<String>,
}

/// `POST /v1/agent/memory/review/:id/:action` — approve | reject | undo.
pub async fn resolve_review(
    State(state): State<AgentState>,
    Extension(principal): Extension<Principal>,
    Path((id, action)): Path<(Uuid, String)>,
    body: Option<Json<ResolveBody>>,
) -> Response {
    if principal.role < Role::Write {
        return forbidden("memory review requires write role");
    }
    let Some(act) = ResolveAction::parse(&action) else {
        return bad_request("action must be approve|reject|undo");
    };
    let body = body.map(|j| j.0).unwrap_or_default();
    let (gate, shared) = match build_gate(&state, &principal).await {
        Some(g) => g,
        None => return unavailable("no approval queue in this mode"),
    };
    match memory_gate::resolve(
        &gate,
        &shared,
        id,
        act,
        body.payload,
        body.comment.as_deref(),
    )
    .await
    {
        Ok(row) => Json(json!({ "ok": true, "row": row })).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct BulkBody {
    pub ids: Vec<Uuid>,
    pub action: String,
    #[serde(default)]
    pub comment: Option<String>,
}

/// `POST /v1/agent/memory/review/bulk` — apply one action across many rows.
pub async fn bulk_review(
    State(state): State<AgentState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<BulkBody>,
) -> Response {
    if principal.role < Role::Write {
        return forbidden("memory review requires write role");
    }
    let Some(act) = ResolveAction::parse(&body.action) else {
        return bad_request("action must be approve|reject|undo");
    };
    let (gate, shared) = match build_gate(&state, &principal).await {
        Some(g) => g,
        None => return unavailable("no approval queue in this mode"),
    };
    let mut results = Vec::with_capacity(body.ids.len());
    for id in body.ids {
        let r = memory_gate::resolve(&gate, &shared, id, act, None, body.comment.as_deref()).await;
        results.push(match r {
            Ok(row) => json!({ "id": id.to_string(), "ok": true, "status": row.status }),
            Err(e) => json!({ "id": id.to_string(), "ok": false, "error": e.to_string() }),
        });
    }
    Json(json!({ "results": results })).into_response()
}

/// Build the gate (policy + store + resolver) and a tool ctx for apply_op.
/// `None` when there is no durable queue store in this mode.
async fn build_gate(
    state: &AgentState,
    principal: &Principal,
) -> Option<(HitlGate, SharedToolCtx)> {
    let store = QueueStore::from_state(state)?;
    let settings = super::memory_settings::load_for(state).await;
    let gate = HitlGate {
        policy: settings.hitl,
        store: Arc::new(store),
        resolver: principal.subject.clone(),
        source: "review",
        source_run_id: None,
    };
    let shared = SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
        hitl: None,
        memory_settings_path: state.memory_settings_path.clone(),
    };
    Some((gate, shared))
}

fn forbidden(msg: &str) -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "error": msg }))).into_response()
}
fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
}
fn unavailable(msg: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": msg })),
    )
        .into_response()
}
fn internal(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::memory_queue_store::QueueRow;

    #[tokio::test]
    async fn list_filter_maps_op_string() {
        // op string that doesn't parse → operation filter is None (lists all).
        let q = ReviewQuery {
            op: Some("not-an-op".into()),
            ..Default::default()
        };
        assert!(MemoryOp::parse(q.op.as_deref().unwrap()).is_none());
    }

    #[tokio::test]
    async fn local_store_list_and_count_via_store() {
        // Exercise the store path the handlers use (no axum needed).
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::Local {
            path: dir.path().join("q.json"),
        };
        let row = QueueRow::new(
            MemoryOp::Merge,
            "r",
            "gate",
            "pending",
            None,
            None,
            "dreaming",
            None,
            json!({"op":"merge","into_id":"memory:a","from_ids":["memory:b"]}),
            None,
        );
        store.insert(&row).await.unwrap();
        let items = store.list(&QueueFilter::default()).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(store.counts().await.unwrap().pending, 1);
    }
}
