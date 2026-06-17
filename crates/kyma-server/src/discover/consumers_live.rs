//! WebSocket session handler for `GET /v1/consumers/live`.
//!
//! Drives the graph explorer's realtime "live consumers" overlay: who (which
//! coding agent / MCP client) is reading or writing memories right now, and
//! which nodes they touch. A simpler sibling of [`super::live`] — there is no
//! query/scope/fanout, just **auth → backfill → live tail**:
//!
//! 1. AUTH — mounted without auth middleware (browsers can't set WS headers),
//!    so the session authenticates via a first-message `{"type":"auth",...}`
//!    handshake against the same [`AuthBackend`] the HTTP surface uses.
//! 2. BACKFILL — a best-effort read of the most recent `memory.recall` /
//!    `memory.import` spans from `otel_traces`, so the dock isn't empty on open.
//!    Telemetry-off / empty table degrades cleanly to no backfill.
//! 3. LIVE — subscribe to the process [`ConsumerEvents`] broadcast and forward
//!    every activity for the session's tenant, with a 15s heartbeat.
//!
//! Frame contract (one JSON object per WS text message, `type`-tagged):
//! `{type:"backfill",activity}` · `{type:"live"}` · `{type:"events",events:[…]}`
//! · `{type:"heartbeat"}` · `{type:"lagged"}` · `{type:"error",code,message}`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::time::{interval_at, Instant as TokioInstant};

use crate::auth::{AuthBackend, Principal, Role};
use crate::QueryState;
use kyma_ingest_core::{ConsumerAction, ConsumerActivity, ConsumerEvents};

/// Authentication handshake deadline (whole handshake, not per-message).
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// Liveness ping cadence.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Most-recent activity rows pulled from `otel_traces` on connect.
const BACKFILL_LIMIT: usize = 50;

/// Shared, cheaply-clonable dependencies handed to each session.
#[derive(Clone)]
struct Deps {
    state: QueryState,
    backend: Arc<dyn AuthBackend>,
    events: Option<ConsumerEvents>,
    /// Database holding `otel_traces` (resolved from config — `otel` in local
    /// mode, `KYMA_OTLP_DATABASE` in server mode). Never hardcoded here.
    traces_db: String,
}

/// Build the public live-consumers router. NO auth middleware — the session
/// authenticates via its first message instead (mirrors [`super::live`]).
pub fn consumers_live_router(
    state: QueryState,
    backend: Arc<dyn AuthBackend>,
    events: Option<ConsumerEvents>,
    traces_db: String,
) -> Router {
    let deps = Deps {
        state,
        backend,
        events,
        traces_db,
    };
    Router::new().route(
        "/v1/consumers/live",
        get(move |ws: WebSocketUpgrade| {
            let deps = deps.clone();
            async move { ws.on_upgrade(move |sock| session(sock, deps)) }
        }),
    )
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Auth { token: String },
}

async fn session(mut ws: WebSocket, deps: Deps) {
    // 1. AUTH — first message within AUTH_TIMEOUT, then a Read-role gate.
    let principal = match await_auth(&mut ws, &deps.backend).await {
        Some(p) => p,
        None => {
            let _ = ws.send(close_policy("unauthorized")).await;
            return;
        }
    };
    if principal.role < Role::Read {
        let _ = send_json(
            &mut ws,
            &json!({"type":"error","code":"forbidden","message":"live consumers requires read access"}),
        )
        .await;
        let _ = ws.send(close_policy("forbidden")).await;
        return;
    }
    let tenant = principal.tenant.to_string();

    // 2. BACKFILL — best-effort recent history, then the `live` marker.
    for act in load_backfill(&deps, &tenant).await {
        if send_json(&mut ws, &json!({"type":"backfill","activity": act}))
            .await
            .is_err()
        {
            return;
        }
    }
    if send_json(&mut ws, &json!({"type":"live"})).await.is_err() {
        return;
    }

    // 3. LIVE — forward this tenant's activity until the client goes away.
    let mut rx = deps.events.as_ref().map(|e| e.subscribe());
    let mut hb = interval_at(TokioInstant::now() + HEARTBEAT, HEARTBEAT);
    loop {
        tokio::select! {
            _ = hb.tick() => {
                if send_json(&mut ws, &json!({"type":"heartbeat"})).await.is_err() {
                    return;
                }
            }
            msg = ws.recv() => {
                match msg {
                    // Client closed, disconnected, or errored: end the session.
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    // Ping/Pong/Text/Binary from the client are ignored — this
                    // socket is server-push only after the handshake.
                    _ => {}
                }
            }
            ev = recv_event(rx.as_mut()) => {
                match ev {
                    EventOutcome::Event(act) => {
                        // Tenant scoping: the bus is process-global, so drop
                        // activity belonging to other tenants.
                        if act.tenant == tenant
                            && send_json(&mut ws, &json!({"type":"events","events":[act]}))
                                .await
                                .is_err()
                        {
                            return;
                        }
                    }
                    EventOutcome::Lagged => {
                        // The UI re-renders from its own ring, so a dropped tick
                        // is cosmetic — just tell it some events were missed.
                        if send_json(&mut ws, &json!({"type":"lagged"})).await.is_err() {
                            return;
                        }
                    }
                    EventOutcome::Closed => rx = None,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Auth + send helpers (mirror `super::live`)
// ---------------------------------------------------------------------------

async fn await_auth(ws: &mut WebSocket, backend: &Arc<dyn AuthBackend>) -> Option<Principal> {
    let deadline = tokio::time::Instant::now() + AUTH_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, ws.recv()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let token = match serde_json::from_str::<ClientMsg>(&t) {
                    Ok(ClientMsg::Auth { token }) => token,
                    Err(_) => return None,
                };
                return backend.authenticate(&token).await.ok();
            }
            // Ping/Pong/Binary before auth: skip and keep waiting.
            Ok(Some(Ok(Message::Ping(_))))
            | Ok(Some(Ok(Message::Pong(_))))
            | Ok(Some(Ok(Message::Binary(_)))) => continue,
            // Timeout, disconnect, close, or transport error: give up.
            _ => return None,
        }
    }
}

async fn send_json(ws: &mut WebSocket, v: &Value) -> Result<(), ()> {
    ws.send(Message::Text(v.to_string())).await.map_err(|_| ())
}

/// Policy-violation close (1008), matching the discover live-tail convention.
fn close_policy(reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code: 1008,
        reason: reason.to_string().into(),
    }))
}

enum EventOutcome {
    Event(ConsumerActivity),
    Lagged,
    Closed,
}

/// Await the next consumer-activity event. With no receiver the future never
/// resolves, so the `select!` arm goes dormant instead of spinning.
async fn recv_event(rx: Option<&mut broadcast::Receiver<ConsumerActivity>>) -> EventOutcome {
    match rx {
        None => std::future::pending().await,
        Some(rx) => match rx.recv().await {
            Ok(ev) => EventOutcome::Event(ev),
            Err(broadcast::error::RecvError::Lagged(_)) => EventOutcome::Lagged,
            Err(broadcast::error::RecvError::Closed) => EventOutcome::Closed,
        },
    }
}

// ---------------------------------------------------------------------------
// Backfill from otel_traces
// ---------------------------------------------------------------------------

/// Read the most recent memory read/write spans for `tenant` from `otel_traces`.
/// Best-effort: any error (telemetry off, table absent, query failure) yields an
/// empty backfill rather than failing the socket.
async fn load_backfill(deps: &Deps, tenant: &str) -> Vec<ConsumerActivity> {
    // A minimal tool ctx just for the backfill read (no bus, no federation).
    let shared = crate::agent::SharedToolCtx {
        consumer_sink: None,
        federation: None,
        catalog: deps.state.catalog.clone(),
        format: deps.state.format.clone(),
        pool: None,
        memory: None,
        hitl: None,
    };
    let tenant_esc = tenant.replace('\'', "''");
    let sql = format!(
        "SELECT name, subject, start_time, attributes_json FROM otel_traces \
         WHERE (name = 'memory.recall' OR name = 'memory.remember' OR name = 'memory.import') \
           AND tenant = '{tenant_esc}' \
         ORDER BY start_time DESC LIMIT {BACKFILL_LIMIT}"
    );
    let res = crate::agent::execute_sql(&shared, &deps.traces_db, &sql, BACKFILL_LIMIT).await;
    let Some(rows) = res.get("rows").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<ConsumerActivity> = rows
        .iter()
        .filter_map(|row| backfill_row_to_activity(row, tenant))
        .collect();
    // Emit oldest → newest (the query returned newest first).
    out.reverse();
    out
}

/// Map one `otel_traces` row into a [`ConsumerActivity`]. Backfill rows carry no
/// node ids (otel doesn't record them) and a coarse `unknown` kind — live frames
/// are the rich source; this is just "who was active recently".
fn backfill_row_to_activity(row: &Value, tenant: &str) -> Option<ConsumerActivity> {
    let name = row.get("name")?.as_str()?;
    let action = if name == "memory.recall" {
        ConsumerAction::Recall
    } else {
        ConsumerAction::Remember
    };
    let subject = row
        .get("subject")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let attrs: Option<Value> = row
        .get("attributes_json")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let query_preview = attrs.as_ref().and_then(|a| {
        a.get("memory.query")
            .and_then(|q| q.as_str())
            .map(|q| q.to_string())
    });
    let ts = row
        .get("start_time")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    // Prefer the stamped client; otel-backfilled memory ops without one are
    // Kyma's own HTTP/internal reads (external agents use MCP, which doesn't
    // create these spans), so default to the Kyma agent — never "unknown".
    let kind = attrs
        .as_ref()
        .and_then(|a| a.get("kyma.client").and_then(|c| c.as_str()))
        .filter(|s| !s.is_empty())
        .unwrap_or("kyma")
        .to_string();
    Some(ConsumerActivity {
        consumer_id: format!("{kind}:{}", subject.as_deref().unwrap_or("anon")),
        label: subject.clone().unwrap_or_else(|| kind.clone()),
        kind,
        subject,
        tenant: tenant.to_string(),
        action,
        node_ids: Vec::new(),
        namespaces: Vec::new(),
        query_preview,
        ts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_maps_recall_row() {
        let row = json!({
            "name": "memory.recall",
            "subject": "shaked",
            "start_time": "2026-06-16T10:00:00Z",
            "attributes_json": "{\"memory.query\":\"auth flow\",\"memory.results\":\"3\"}",
        });
        let act = backfill_row_to_activity(&row, "default").expect("mapped");
        assert_eq!(act.action, ConsumerAction::Recall);
        assert_eq!(act.subject.as_deref(), Some("shaked"));
        assert_eq!(act.query_preview.as_deref(), Some("auth flow"));
        assert_eq!(act.tenant, "default");
        let expected = chrono::DateTime::parse_from_rfc3339("2026-06-16T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(act.ts, expected);
    }

    #[test]
    fn backfill_maps_write_row_and_handles_missing_fields() {
        let row = json!({ "name": "memory.import" });
        let act = backfill_row_to_activity(&row, "t1").expect("mapped");
        assert_eq!(act.action, ConsumerAction::Remember);
        assert!(act.subject.is_none());
        assert!(act.query_preview.is_none());
        // No stamped client → defaults to the Kyma agent, never "unknown".
        assert_eq!(act.consumer_id, "kyma:anon");
        assert_eq!(act.kind, "kyma");
    }

    #[test]
    fn backfill_reads_stamped_client() {
        let row = json!({
            "name": "memory.recall",
            "attributes_json": "{\"kyma.client\":\"claude-code\",\"memory.query\":\"q\"}",
        });
        let act = backfill_row_to_activity(&row, "t1").expect("mapped");
        assert_eq!(act.kind, "claude-code");
        assert_eq!(act.consumer_id, "claude-code:anon");
    }

    #[test]
    fn backfill_skips_rows_without_name() {
        assert!(backfill_row_to_activity(&json!({"subject":"x"}), "t").is_none());
    }
}
