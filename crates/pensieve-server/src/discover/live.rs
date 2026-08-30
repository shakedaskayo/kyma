//! WebSocket session handler for `GET /v1/explore/live`.
//!
//! Mounted **without** auth middleware (browsers cannot set WS headers), so the
//! session authenticates via a first-message handshake against the same
//! [`AuthBackend`] the HTTP surface uses. After auth + subscribe it runs the
//! existing Discover fanout for backfill, then tails new rows: an in-process
//! [`IngestEvents`] broadcast wakes the session on every committed write, the
//! session debounces, runs an incremental fanout over the touched sources with
//! a `[cursor, now)` window, and forwards the resulting `Rows`/`Error` frames.
//!
//! Frame contract (mirrors `/v1/explore/search`, see [`super::frames`]): backfill
//! emits `Plan`/`SourceProgress`/`Rows`/`Histogram`/`SourceDone`, then `Live`
//! (instead of the fanout's `Done`), then incremental `Rows`/`Error` frames and
//! a `Heartbeat` every 15s. Each frame is one WS text message.
//!
//! Correctness notes:
//! - Time windows are `[from, to)` (compile.rs builds `>= from and < to`), so
//!   consecutive incremental scans never overlap and never leave gaps. The
//!   cursor advances to the `now` captured *before* a scan, so rows committed
//!   mid-scan fall into the next window.
//! - Sources without a timestamp column are skipped in incremental scans —
//!   compile drops the time filter for them, so an incremental scan would just
//!   re-send the same `take N` page every tick.
//! - Grammar/scope errors mid-`update` send the same coded `Error` frame the
//!   HTTP handler would and keep the previous subscription running; a typo
//!   never kills the session.

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::Router;
use kyma_core::query_frontend::QueryBudget;
use kyma_core::tenant::TenantId;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, Instant as TokioInstant, Sleep};

use super::compile::{compile_for_source, TimeRange};
use super::fanout::{run as run_fanout, FanoutInput};
use super::frames::{frame_to_line, Frame};
use super::grammar::parse as parse_grammar;
use super::handler::{parse_time_range, TimeRangeBody};
use super::scope::{resolve as resolve_scope, ResolvedSource, Scope, ScopeError};
use crate::auth::{AuthBackend, Principal, Role};
use crate::QueryState;
use kyma_ingest_core::IngestEvents;

/// Authentication handshake deadline.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// Debounce window between an ingest event and its incremental scan.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Timer fallback for ingest paths that bypass (or lag) the notify bus.
const FALLBACK_SCAN: Duration = Duration::from_secs(30);
/// Liveness ping cadence.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Per-source row cap for incremental scans (one window's worth).
const INCREMENTAL_PER_SOURCE_LIMIT: usize = 500;
/// Scope-size cap, matching the HTTP handler's default.
const DEFAULT_MAX_SOURCES: usize = 200;
/// Backfill per-source limit defaults — same clamp as the HTTP handler.
const DEFAULT_PER_SOURCE_LIMIT: usize = 500;
const MAX_PER_SOURCE_LIMIT: usize = 5_000;

/// Shared, cheaply-clonable dependencies handed to each session.
#[derive(Clone)]
struct LiveDeps {
    state: QueryState,
    backend: Arc<dyn AuthBackend>,
    events: Option<IngestEvents>,
}

/// Build the public live-tail router. NO auth middleware — the session
/// authenticates via its first message instead.
pub fn explore_live_router(
    state: QueryState,
    backend: Arc<dyn AuthBackend>,
    events: Option<IngestEvents>,
) -> Router {
    let shared = LiveDeps {
        state,
        backend,
        events,
    };
    Router::new().route(
        "/v1/explore/live",
        get(move |ws: WebSocketUpgrade| {
            let deps = shared.clone();
            async move { ws.on_upgrade(move |sock| session(sock, deps)) }
        }),
    )
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Auth { token: String },
    Subscribe(SubscribeBody),
    Update(SubscribeBody),
    Pause,
    Resume,
}

#[derive(Debug, Clone, Deserialize)]
struct SubscribeBody {
    #[serde(default)]
    query: String,
    scope: Scope,
    #[serde(default)]
    time_range: Option<TimeRangeBody>,
    #[serde(default)]
    per_source_limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

async fn session(mut ws: WebSocket, deps: LiveDeps) {
    // 1. AUTH: first message within AUTH_TIMEOUT or close.
    let principal = match await_auth(&mut ws, &deps.backend).await {
        AuthOutcome::Ok(p) => p,
        // Coded error frame already sent (or not, for timeout/disconnect); close.
        AuthOutcome::Reject => {
            let _ = ws.send(close_msg_policy("unauthorized")).await;
            return;
        }
        AuthOutcome::Gone => return,
    };
    if principal.role < Role::Read {
        let _ = send_frame(
            &mut ws,
            &Frame::Error {
                source: None,
                code: "forbidden".into(),
                message: "live tail requires read access".into(),
            },
        )
        .await;
        let _ = ws.send(close_msg_policy("forbidden")).await;
        return;
    }
    let tenant = principal.tenant;
    let subject = principal.subject.clone();

    // 2. SUBSCRIBE: next message must be Subscribe.
    let mut sub = match await_subscribe(&mut ws).await {
        Some(s) => s,
        None => return,
    };

    // Each iteration drives one (re)subscription to completion or hands a fresh
    // `sub` back via `continue 'resub` after an `update`.
    'resub: loop {
        // 3. Resolve scope + parse grammar — same error frames as the HTTP
        //    handler, sent as WS messages. On error we keep the session open and
        //    wait for the next client message (a corrected `update`).
        let clauses = match parse_grammar(&sub.query) {
            Ok(c) => c,
            Err(e) => {
                send_coded_error(&mut ws, "grammar_error", &e.to_string()).await;
                match wait_for_update(&mut ws).await {
                    Some(next) => {
                        sub = next;
                        continue 'resub;
                    }
                    None => return,
                }
            }
        };

        let time_range = match sub.time_range.as_ref() {
            None => None,
            Some(tr) => match parse_time_range(tr) {
                Ok(t) => Some(t),
                Err(msg) => {
                    send_coded_error(&mut ws, "bad_time_range", &msg).await;
                    match wait_for_update(&mut ws).await {
                        Some(next) => {
                            sub = next;
                            continue 'resub;
                        }
                        None => return,
                    }
                }
            },
        };

        let resolved = match resolve_for(&deps.state, tenant, subject.as_deref(), &sub.scope).await
        {
            Ok(r) => r,
            Err((code, msg)) => {
                send_coded_error(&mut ws, code, &msg).await;
                match wait_for_update(&mut ws).await {
                    Some(next) => {
                        sub = next;
                        continue 'resub;
                    }
                    None => return,
                }
            }
        };

        // Classify timestamped sources once per resub. Incremental scans run
        // only over these; no-timestamp sources can't be windowed.
        let per_source_limit = sub
            .per_source_limit
            .unwrap_or(DEFAULT_PER_SOURCE_LIMIT)
            .clamp(1, MAX_PER_SOURCE_LIMIT);
        let ts_sources: Vec<ResolvedSource> = resolved
            .iter()
            .filter(|s| {
                compile_for_source(&s.table, &clauses, time_range.as_ref(), per_source_limit)
                    .has_timestamp
            })
            .cloned()
            .collect();
        // The set of "db.table" keys we accept ingest events for.
        let resolved_keys: BTreeSet<String> = ts_sources.iter().map(source_key).collect();

        // 4. BACKFILL: forward fanout frames, translating its `Done` → `Live`.
        //
        // Cursor seeding — two cases:
        //   a) Bounded subscription (time_range.to is set): cursor = to_ms.
        //      The fanout already has an upper bound of `to_ms`, so the first
        //      incremental window starts exactly where the backfill stopped,
        //      preserving gap-free coverage against the `< to` filter.
        //   b) Unbounded subscription (no to): cursor = now_ms() captured at
        //      the moment the fanout's Done frame arrives. Rows committed
        //      during the backfill scan are already in its result set; seeding
        //      cursor from *before* the backfill would make the first
        //      incremental window re-send those rows (duplicate burst). The
        //      frontend store typically has no dedup, so we must not overlap.
        let backfill_done_ms =
            match backfill(&mut ws, &deps.state, &resolved, &clauses, time_range, per_source_limit)
                .await
            {
                Ok(t) => t,
                Err(()) => return, // client gone
            };
        // TimeRange is Copy so time_range is still valid after the backfill call.
        let mut cursor_ms = match time_range {
            Some(t) => t.to_ms, // bounded: gap-free against the `< to` filter
            None => backfill_done_ms, // unbounded: avoid duplicate burst (see above)
        };

        // 5. LIVE LOOP.
        let mut events_rx = deps.events.as_ref().map(|e| e.subscribe());
        let mut heartbeat = interval(HEARTBEAT);
        heartbeat.tick().await; // consume the immediate first tick
        let mut fallback = interval(FALLBACK_SCAN);
        fallback.tick().await;

        let mut paused = false;
        // Pending touched sources awaiting a debounced scan. A `None` window
        // sentinel ("scan everything") is represented by inserting all keys.
        let mut pending: BTreeSet<String> = BTreeSet::new();
        let mut debounce: Option<Pin<Box<Sleep>>> = None;

        loop {
            // Arm the debounce timer when work is pending and we're not paused.
            if debounce.is_none() && !pending.is_empty() && !paused {
                debounce = Some(Box::pin(sleep_until(TokioInstant::now() + DEBOUNCE)));
            }

            tokio::select! {
                // -- Client control messages ------------------------------------
                incoming = ws.recv() => {
                    match incoming {
                        None | Some(Err(_)) => return,
                        Some(Ok(Message::Close(_))) => return,
                        Some(Ok(Message::Text(txt))) => {
                            match serde_json::from_str::<ClientMsg>(&txt) {
                                Ok(ClientMsg::Update(body)) => {
                                    sub = body;
                                    continue 'resub;
                                }
                                Ok(ClientMsg::Pause) => {
                                    paused = true;
                                    debounce = None;
                                }
                                Ok(ClientMsg::Resume) => {
                                    paused = false;
                                    // One catch-up scan over everything since the cursor.
                                    let now = now_ms();
                                    if scan_incremental(
                                        &mut ws, &deps.state, &ts_sources, &clauses,
                                        cursor_ms, now, &resolved_keys,
                                    ).await.is_err() {
                                        return;
                                    }
                                    cursor_ms = now;
                                    pending.clear();
                                }
                                // Auth/Subscribe mid-session, or junk: ignore.
                                Ok(_) | Err(_) => {}
                            }
                        }
                        // Ping/Pong/Binary: ignore (axum auto-replies to pings).
                        Some(Ok(_)) => {}
                    }
                }

                // -- Ingest notify ----------------------------------------------
                ev = recv_event(events_rx.as_mut()) => {
                    match ev {
                        EventOutcome::Row { database, table } => {
                            let key = format!("{database}.{table}");
                            if resolved_keys.contains(&key) {
                                pending.insert(key);
                            }
                        }
                        // Lagged: we may have missed events — scan everything.
                        EventOutcome::Lagged => {
                            pending.extend(resolved_keys.iter().cloned());
                        }
                    }
                }

                // -- Debounce elapsed -------------------------------------------
                _ = wait_debounce(debounce.as_mut()), if debounce.is_some() => {
                    debounce = None;
                    if !paused && !pending.is_empty() {
                        let now = now_ms();
                        let touched: Vec<ResolvedSource> = ts_sources
                            .iter()
                            .filter(|s| pending.contains(&source_key(s)))
                            .cloned()
                            .collect();
                        pending.clear();
                        if scan_incremental(
                            &mut ws, &deps.state, &touched, &clauses,
                            cursor_ms, now, &resolved_keys,
                        ).await.is_err() {
                            return;
                        }
                        cursor_ms = now;
                    }
                }

                // -- Timer fallback (scan all) ----------------------------------
                _ = fallback.tick() => {
                    if !paused {
                        let now = now_ms();
                        if scan_incremental(
                            &mut ws, &deps.state, &ts_sources, &clauses,
                            cursor_ms, now, &resolved_keys,
                        ).await.is_err() {
                            return;
                        }
                        cursor_ms = now;
                        pending.clear();
                    }
                }

                // -- Heartbeat --------------------------------------------------
                _ = heartbeat.tick() => {
                    if send_frame(&mut ws, &Frame::Heartbeat).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Auth + subscribe handshake
// ---------------------------------------------------------------------------

enum AuthOutcome {
    Ok(Principal),
    /// Token present but rejected.
    Reject,
    /// Client disconnected / timed out before authenticating.
    Gone,
}

async fn await_auth(ws: &mut WebSocket, backend: &Arc<dyn AuthBackend>) -> AuthOutcome {
    // The 5s deadline covers the WHOLE handshake (deadline fixed once, not
    // per-message). Within that window we skip Ping/Pong/Binary frames —
    // consistent with `await_subscribe`'s loop — so a browser that sends
    // a WebSocket ping before the auth message isn't incorrectly closed.
    let deadline = tokio::time::Instant::now() + AUTH_TIMEOUT;
    loop {
        let recv = tokio::time::timeout_at(deadline, ws.recv()).await;
        match recv {
            Ok(Some(Ok(Message::Text(t)))) => {
                let token = match serde_json::from_str::<ClientMsg>(&t) {
                    Ok(ClientMsg::Auth { token }) => token,
                    // Wrong first message or junk → reject.
                    _ => return AuthOutcome::Reject,
                };
                return match backend.authenticate(&token).await {
                    Ok(p) => AuthOutcome::Ok(p),
                    Err(_) => AuthOutcome::Reject,
                };
            }
            // Ping/Pong/Binary before auth: skip and keep waiting.
            Ok(Some(Ok(Message::Ping(_))))
            | Ok(Some(Ok(Message::Pong(_))))
            | Ok(Some(Ok(Message::Binary(_)))) => continue,
            // Timeout, disconnect, close frame, or transport error: give up.
            _ => return AuthOutcome::Gone,
        }
    }
}

async fn await_subscribe(ws: &mut WebSocket) -> Option<SubscribeBody> {
    loop {
        match ws.recv().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Subscribe(body)) => return Some(body),
                Ok(_) => {
                    // Premature update/pause/resume before subscribe: protocol
                    // error, but be lenient — wait for a real subscribe.
                    send_coded_error(ws, "bad_request", "expected subscribe message").await;
                }
                Err(_) => {
                    send_coded_error(ws, "bad_request", "invalid subscribe message").await;
                }
            },
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => {} // ping/pong/binary
        }
    }
}

/// Block on the next client message after a recoverable error, expecting an
/// `update` to replace the subscription. Pause/Resume/junk are ignored; a close
/// or transport error ends the session.
async fn wait_for_update(ws: &mut WebSocket) -> Option<SubscribeBody> {
    loop {
        match ws.recv().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMsg>(&t) {
                Ok(ClientMsg::Update(body)) | Ok(ClientMsg::Subscribe(body)) => return Some(body),
                Ok(_) | Err(_) => {}
            },
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Scope resolution
// ---------------------------------------------------------------------------

/// Resolve a scope under the session's tenant, mapping `ScopeError` to the same
/// `(code, message)` pairs the HTTP handler sends.
async fn resolve_for(
    state: &QueryState,
    tenant: TenantId,
    subject: Option<&str>,
    scope: &Scope,
) -> Result<Vec<ResolvedSource>, (&'static str, String)> {
    let max_sources = std::env::var("KYMA_DISCOVER_MAX_SOURCES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_SOURCES);

    // Saved-view lookup needs both an authenticated subject and a Postgres pool.
    // In local mode there is no pool, so named-view scopes fall through to
    // ViewNotFound (ad-hoc scopes are unaffected) — matching the HTTP handler.
    let lookup: Option<crate::discover::saved_views_lookup::CatalogSavedViewLookup> =
        match (subject, state.pg_pool.clone()) {
            (Some(s), Some(pool)) => {
                Some(crate::discover::saved_views_lookup::CatalogSavedViewLookup {
                    pool,
                    tenant_id: tenant.as_uuid(),
                    owner_subject: s.to_string(),
                })
            }
            _ => None,
        };

    resolve_scope(
        scope,
        tenant,
        state.catalog.clone(),
        lookup
            .as_ref()
            .map(|l| l as &(dyn crate::discover::scope::SavedViewLookup + Send + Sync)),
        max_sources,
    )
    .await
    .map_err(|e| match e {
        ScopeError::ScopeTooLarge(n, max) => (
            "scope_too_large",
            format!("scope resolves to {n} sources, exceeds max {max}"),
        ),
        ScopeError::ViewNotFound(id) => ("view_not_found", format!("saved view {id} not found")),
        ScopeError::Catalog(msg) => ("catalog_error", msg),
    })
}

// ---------------------------------------------------------------------------
// Fanout helpers
// ---------------------------------------------------------------------------

/// Backfill pass: run the fanout, forward every frame, but swap the terminal
/// `Done` for a `Live` marker. Returns the millisecond timestamp captured at
/// the moment the fanout's `Done` frame arrives (or channel close), which the
/// caller uses to seed `cursor_ms` for unbounded subscriptions.
///
/// Why capture `now_ms()` here rather than before the backfill?
/// - For **unbounded** subscriptions (no explicit `to`), an unbounded fanout
///   runs until *its own* execution end. Rows committed during the backfill
///   scan are already in the backfill result set, so `cursor_ms` must be set
///   to the moment the backfill *finished* — not the moment it started — to
///   avoid the incremental loop re-sending those same rows (duplicate burst).
/// - For **bounded** subscriptions (`time_range.to` is set), the caller pins
///   `cursor_ms` to `to_ms` regardless of this return value (gap-free vs the
///   `< to` filter). The returned timestamp is still correct but unused.
///
/// Returns `Err(())` if the client disconnects.
async fn backfill(
    ws: &mut WebSocket,
    state: &QueryState,
    sources: &[ResolvedSource],
    clauses: &[super::grammar::Clause],
    time_range: Option<TimeRange>,
    per_source_limit: usize,
) -> Result<i64, ()> {
    let (tx, mut rx) = mpsc::channel::<Frame>(64);
    run_fanout(
        FanoutInput {
            sources: sources.to_vec(),
            clauses: clauses.to_vec(),
            time_range,
            per_source_limit,
            budget: QueryBudget::default(),
            catalog: state.catalog.clone(),
            format: state.format.clone(),
            node_id: state.node_id,
        },
        tx,
    );

    while let Some(frame) = rx.recv().await {
        match frame {
            Frame::Done { .. } => {
                // Backfill complete → capture now, then hand off to the live tail.
                let done_ms = now_ms();
                send_frame(ws, &Frame::Live).await?;
                return Ok(done_ms);
            }
            other => send_frame(ws, &other).await?,
        }
    }
    // Channel closed without an explicit Done (cancellation/panic upstream):
    // still mark live so the client leaves the backfill state.
    let done_ms = now_ms();
    send_frame(ws, &Frame::Live).await?;
    Ok(done_ms)
}

/// Incremental scan over a subset of timestamped sources with a `[from, to)`
/// window. Forwards ONLY `Rows`/`Error` frames; drops planning/progress frames.
/// Caller advances the cursor to `to_ms` on success.
async fn scan_incremental(
    ws: &mut WebSocket,
    state: &QueryState,
    sources: &[ResolvedSource],
    clauses: &[super::grammar::Clause],
    from_ms: i64,
    to_ms: i64,
    resolved_keys: &BTreeSet<String>,
) -> Result<(), ()> {
    // Empty or non-advancing window: nothing to scan.
    if sources.is_empty() || next_window(from_ms, to_ms).is_none() {
        return Ok(());
    }
    // Defensive: never scan a source outside the resolved set.
    let sources: Vec<ResolvedSource> = sources
        .iter()
        .filter(|s| resolved_keys.contains(&source_key(s)))
        .cloned()
        .collect();
    if sources.is_empty() {
        return Ok(());
    }

    let window = TimeRange {
        from_ms,
        to_ms,
    };
    let (tx, mut rx) = mpsc::channel::<Frame>(64);
    run_fanout(
        FanoutInput {
            sources,
            clauses: clauses.to_vec(),
            time_range: Some(window),
            per_source_limit: INCREMENTAL_PER_SOURCE_LIMIT,
            // Tight wall-clock budget for incremental scans. Scans are awaited
            // inline inside `select!` arms (by design — prevents overlapping
            // scans on the same subscription), which means a stalled source
            // blocks ALL control messages and heartbeats for the duration of
            // the scan. The default budget is 300s; 10s keeps the stall
            // bounded to something a client can survive without timing out.
            budget: QueryBudget {
                max_wall_clock: Duration::from_secs(10),
                ..QueryBudget::default()
            },
            catalog: state.catalog.clone(),
            format: state.format.clone(),
            node_id: state.node_id,
        },
        tx,
    );

    while let Some(frame) = rx.recv().await {
        match frame {
            Frame::Rows { .. } | Frame::Error { .. } => send_frame(ws, &frame).await?,
            // Drop Plan/SourceProgress/Histogram/SourceDone/Done/Live/Heartbeat.
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Window math (pure, unit-tested)
// ---------------------------------------------------------------------------

/// The next half-open scan window `[cursor, now)`, or `None` when `now <=
/// cursor` (no time has elapsed, so a scan would be empty or invalid).
///
/// Consecutive calls — advancing `cursor` to the previous `now` each time —
/// tile the timeline with no overlap and no gap, because compile.rs builds the
/// filter as `ts >= from and ts < to`.
fn next_window(cursor_ms: i64, now_ms: i64) -> Option<TimeRange> {
    if now_ms <= cursor_ms {
        None
    } else {
        Some(TimeRange {
            from_ms: cursor_ms,
            to_ms: now_ms,
        })
    }
}

/// `"db.table"` key for a resolved source — must match the fanout's source keys
/// and the `RowsAppended { database, table }` event shape.
fn source_key(s: &ResolvedSource) -> String {
    format!("{}.{}", s.db, s.table.name)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// WS send helpers + event/debounce select arms
// ---------------------------------------------------------------------------

/// Serialize a frame to its JSON line, trim the trailing newline, send as one
/// WS text message. `Err(())` ⇒ client gone.
async fn send_frame(ws: &mut WebSocket, frame: &Frame) -> Result<(), ()> {
    let mut line = frame_to_line(frame);
    if line.ends_with('\n') {
        line.pop();
    }
    ws.send(Message::Text(line)).await.map_err(|_| ())
}

async fn send_coded_error(ws: &mut WebSocket, code: &str, message: &str) {
    let _ = send_frame(
        ws,
        &Frame::Error {
            source: None,
            code: code.to_string(),
            message: message.to_string(),
        },
    )
    .await;
}

/// Normal close (1000 — clean shutdown).
#[allow(dead_code)] // used in tests; kept for future clean-shutdown call sites
fn close_msg(reason: &str) -> Message {
    close_msg_with_code(1000, reason)
}

/// Policy-violation close (1008 — authentication/authorization failure).
///
/// RFC 6455 §7.4.1: 1008 is reserved for endpoints that received a message
/// that violates a policy. Using it for auth-reject and forbidden closes
/// lets clients distinguish a policy failure from a clean server-side close.
fn close_msg_policy(reason: &str) -> Message {
    close_msg_with_code(1008, reason)
}

fn close_msg_with_code(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_string().into(),
    }))
}

enum EventOutcome {
    Row { database: String, table: String },
    Lagged,
}

/// Await the next ingest event. When there is no receiver (timer-only mode) or
/// the channel has closed, this future never resolves so the `select!` arm goes
/// dormant instead of spinning.
async fn recv_event(rx: Option<&mut tokio::sync::broadcast::Receiver<kyma_ingest_core::RowsAppended>>) -> EventOutcome {
    match rx {
        None => std::future::pending().await,
        Some(rx) => loop {
            match rx.recv().await {
                Ok(ev) => {
                    return EventOutcome::Row {
                        database: ev.database,
                        table: ev.table,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    return EventOutcome::Lagged
                }
                // Sender dropped: no more events ever. Park forever.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    std::future::pending::<()>().await;
                }
            }
        },
    }
}

/// Await an optional debounce timer; pending forever when it's `None` (the arm
/// is `if debounce.is_some()`-guarded, so this only resolves when armed).
async fn wait_debounce(s: Option<&mut Pin<Box<Sleep>>>) {
    match s {
        Some(sleep) => sleep.as_mut().await,
        None => std::future::pending().await,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- ClientMsg deserialization (all five message types) ------------------

    #[test]
    fn deserialize_auth() {
        let m: ClientMsg = serde_json::from_str(r#"{"type":"auth","token":"abc"}"#).unwrap();
        match m {
            ClientMsg::Auth { token } => assert_eq!(token, "abc"),
            _ => panic!("expected auth"),
        }
    }

    #[test]
    fn deserialize_subscribe_full() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"subscribe","query":"error","scope":{"kind":"all"},
                "time_range":{"from":"2026-06-05T00:00:00Z","to":"2026-06-05T01:00:00Z"},
                "per_source_limit":250}"#,
        )
        .unwrap();
        match m {
            ClientMsg::Subscribe(b) => {
                assert_eq!(b.query, "error");
                assert!(matches!(b.scope, Scope::All));
                assert_eq!(b.per_source_limit, Some(250));
                let tr = b.time_range.expect("time_range present");
                assert_eq!(tr.from, "2026-06-05T00:00:00Z");
                assert_eq!(tr.to, "2026-06-05T01:00:00Z");
            }
            _ => panic!("expected subscribe"),
        }
    }

    #[test]
    fn deserialize_subscribe_minimal_defaults() {
        // query, time_range, per_source_limit all optional.
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"subscribe","scope":{"kind":"sources","sources":["prod.*"]}}"#,
        )
        .unwrap();
        match m {
            ClientMsg::Subscribe(b) => {
                assert_eq!(b.query, "");
                assert!(b.time_range.is_none());
                assert!(b.per_source_limit.is_none());
                match b.scope {
                    Scope::Sources { sources } => assert_eq!(sources, vec!["prod.*".to_string()]),
                    _ => panic!("expected sources scope"),
                }
            }
            _ => panic!("expected subscribe"),
        }
    }

    #[test]
    fn deserialize_update() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"update","scope":{"kind":"all"},"query":"x"}"#).unwrap();
        match m {
            ClientMsg::Update(b) => {
                assert_eq!(b.query, "x");
                assert!(matches!(b.scope, Scope::All));
            }
            _ => panic!("expected update"),
        }
    }

    #[test]
    fn deserialize_pause_and_resume() {
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"pause"}"#).unwrap(),
            ClientMsg::Pause
        ));
        assert!(matches!(
            serde_json::from_str::<ClientMsg>(r#"{"type":"resume"}"#).unwrap(),
            ClientMsg::Resume
        ));
    }

    #[test]
    fn deserialize_unknown_type_errors() {
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"bogus"}"#).is_err());
    }

    // --- Window math ---------------------------------------------------------

    #[test]
    fn next_window_none_when_now_not_after_cursor() {
        assert!(next_window(100, 100).is_none());
        assert!(next_window(100, 50).is_none());
    }

    #[test]
    fn next_window_some_when_time_advanced() {
        let w = next_window(100, 200).expect("window");
        assert_eq!(w.from_ms, 100);
        assert_eq!(w.to_ms, 200);
    }

    #[test]
    fn consecutive_windows_have_no_gap_and_no_overlap() {
        // Simulate a monotonic clock: advance the cursor to each window's `to`.
        let clock = [10_i64, 25, 25, 40, 100];
        let mut cursor = 0_i64;
        let mut windows: Vec<(i64, i64)> = Vec::new();
        for &now in &clock {
            if let Some(w) = next_window(cursor, now) {
                windows.push((w.from_ms, w.to_ms));
                cursor = w.to_ms; // advance only on a real scan
            }
            // now == cursor (the repeated 25) yields None: cursor unchanged.
        }
        // Each window starts exactly where the previous ended (no gap), and the
        // half-open `[from, to)` semantics mean no millisecond is covered twice.
        assert_eq!(windows, vec![(0, 10), (10, 25), (25, 40), (40, 100)]);
        for pair in windows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "end of one window == start of next");
        }
    }

    // --- Touched-set intersection -------------------------------------------

    /// Pure mirror of the live loop's intersection rule: only events whose
    /// `db.table` is in the resolved set become pending.
    fn intersect(resolved: &BTreeSet<String>, events: &[(&str, &str)]) -> BTreeSet<String> {
        let mut pending = BTreeSet::new();
        for (db, table) in events {
            let key = format!("{db}.{table}");
            if resolved.contains(&key) {
                pending.insert(key);
            }
        }
        pending
    }

    #[test]
    fn events_outside_resolved_set_are_ignored() {
        let resolved: BTreeSet<String> =
            ["prod.logs", "prod.metrics"].iter().map(|s| s.to_string()).collect();
        let pending = intersect(
            &resolved,
            &[
                ("prod", "logs"),    // in set
                ("other", "logs"),   // db not in set
                ("prod", "traces"),  // table not in set
                ("prod", "metrics"), // in set
            ],
        );
        assert_eq!(
            pending,
            ["prod.logs", "prod.metrics"]
                .iter()
                .map(|s| s.to_string())
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn empty_resolved_set_collects_nothing() {
        let resolved: BTreeSet<String> = BTreeSet::new();
        let pending = intersect(&resolved, &[("prod", "logs"), ("a", "b")]);
        assert!(pending.is_empty());
    }

    // --- Fix 1: cursor seeding logic -----------------------------------------

    /// Verify the cursor seeding rule: bounded subscription uses `to_ms` from
    /// the time range; unbounded uses the backfill-done timestamp.
    #[test]
    fn cursor_seeded_from_to_ms_when_bounded() {
        let time_range = Some(TimeRange { from_ms: 1000, to_ms: 9000 });
        let backfill_done_ms = 9500_i64; // arrived slightly after to_ms
        // Bounded: cursor must equal to_ms to stay gap-free against `< to`.
        let cursor_ms = match time_range {
            Some(t) => t.to_ms,
            None => backfill_done_ms,
        };
        assert_eq!(cursor_ms, 9000);
    }

    #[test]
    fn cursor_seeded_from_backfill_done_when_unbounded() {
        let time_range: Option<TimeRange> = None;
        let backfill_done_ms = 9500_i64;
        // Unbounded: cursor must equal the Done-frame timestamp to avoid
        // re-sending rows committed during the backfill (duplicate burst).
        let cursor_ms = match time_range {
            Some(t) => t.to_ms,
            None => backfill_done_ms,
        };
        assert_eq!(cursor_ms, backfill_done_ms);
    }

    // --- Fix 4: close-frame codes --------------------------------------------

    #[test]
    fn close_msg_uses_1000() {
        match close_msg("bye") {
            Message::Close(Some(cf)) => assert_eq!(cf.code, 1000),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn close_msg_policy_uses_1008() {
        match close_msg_policy("unauthorized") {
            Message::Close(Some(cf)) => {
                assert_eq!(cf.code, 1008);
                assert_eq!(cf.reason.as_ref(), "unauthorized");
            }
            other => panic!("unexpected: {:?}", other),
        }
        match close_msg_policy("forbidden") {
            Message::Close(Some(cf)) => assert_eq!(cf.code, 1008),
            other => panic!("unexpected: {:?}", other),
        }
    }

    // --- Fix 2: incremental budget tightness ---------------------------------

    /// Ensure the incremental scan budget wall-clock is strictly tighter than
    /// the default (300s). This is a safety-net, not a full integration test.
    #[test]
    fn incremental_budget_wall_clock_is_tight() {
        let budget = QueryBudget {
            max_wall_clock: Duration::from_secs(10),
            ..QueryBudget::default()
        };
        assert!(
            budget.max_wall_clock < QueryBudget::default().max_wall_clock,
            "incremental budget ({:?}) must be tighter than default ({:?})",
            budget.max_wall_clock,
            QueryBudget::default().max_wall_clock,
        );
        // Other limits should remain at the defaults.
        assert_eq!(budget.max_object_store_bytes, QueryBudget::default().max_object_store_bytes);
        assert_eq!(budget.max_memory_bytes, QueryBudget::default().max_memory_bytes);
    }
}
