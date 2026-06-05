# Discover v2 Phase 2 — Live Tail (WebSocket + Ingest Notify) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `GET /v1/explore/live` WebSocket: backfill via the existing search fanout, then push incremental rows the moment ingest commits — driven by an in-process rows-appended broadcast, with timer fallback, heartbeats, and a live UI mode.

**Architecture:** A `tokio::broadcast` bus in `kyma-ingest-core` publishes `{database, table, rows}` after every successful commit (single publish site: `ingest_with_idempotency` returns post-commit in all three commit modes). The WS handler (kyma-server `discover::live`) authenticates via first message against the existing `AuthBackend`, runs the existing fanout for backfill, then loops: notify-event → debounce → incremental fanout over touched sources with `TimeRange{from: cursor, to: now}` → forward `rows` frames. Window cursors (`>=from`, `<to` — already how compile.rs:67-79 builds filters) make scans gap- and overlap-free. The route mounts WITHOUT auth middleware in BOTH kyma-bin and kyma-local (browsers can't set WS headers).

**Tech Stack:** axum 0.7 ws (feature must be added), tokio broadcast, existing discover fanout/compile, React + native WebSocket in the SDK.

**Spec:** `docs/superpowers/specs/2026-06-05-discover-live-stream-design.md` §2–3.

**Branch:** `feat/discover-live` off main. Commit only named paths; the repo may carry other sessions' untracked files.

**Investigation ground truth (verified 2026-06-05):**
- Commit choke-point: `WritePath::ingest_with_idempotency` (kyma-ingest-core/src/lib.rs:133-279) returns `IngestAck` only after the snapshot commit in all modes (direct lib.rs:297; staging staging.rs:~569; coordinator commit_coordinator.rs:~231 via oneshot responder). REST/Kafka/OTLP/filedrop/memory all call it.
- No existing event bus. `IngestState`/`QueryState` are decoupled; only `Arc<dyn Catalog>` is shared.
- Construction sites: kyma-local/src/lib.rs ~line 260 (QueryState) and ~290 (WritePath/IngestState); kyma-bin/src/main.rs ~257-279 (WritePath/IngestState) and ~376-384 (QueryState); auth backend built in kyma-local ~296-323 and kyma-bin ~200-244.
- `AuthBackend::authenticate(&self, token: &str) -> Result<Principal, AuthError>`; `Principal { tenant, role, subject }`; `Role::Read` ordering comparable.
- Fanout: `run(FanoutInput, tx: mpsc::Sender<Frame>)` spawns + returns; emits Plan → per-source frames → Done. Reusable per-tick on a source subset.
- axum workspace dep = `{ version = "0.7", features = ["macros", "http2"] }` — **"ws" feature missing, must be added** (the ws module is feature-gated; do not trust claims otherwise).
- Frontend tolerates unknown frame `type`s (switch without default).

---

### Task 1: Ingest event bus in kyma-ingest-core

**Files:**
- Create: `crates/kyma-ingest-core/src/events.rs`
- Modify: `crates/kyma-ingest-core/src/lib.rs` (WritePath field + publish + module export)
- Modify: `crates/kyma-local/src/lib.rs`, `crates/kyma-bin/src/main.rs` (construct bus, attach to WritePath; keep the Sender around for Task 4 mounting)

- [ ] **Step 1: Failing test** in `events.rs` `#[cfg(test)]` — construct a WritePath with `.with_events(tx)` over the existing test fixtures used by lib.rs tests (read how WritePath is tested today; if only via integration, write the test at the lowest level available: e.g. a unit test that `RowsAppended` round-trips clone + a test in lib.rs's test module asserting a subscriber receives an event after a successful `ingest_with_idempotency`). If WritePath has no unit-test harness with a fake catalog/format, report it and test the publish helper in isolation instead:

```rust
#[tokio::test]
async fn publish_is_lossy_nonblocking_and_received() {
    let bus = IngestEvents::new(8);
    let mut sub = bus.subscribe();
    bus.publish(RowsAppended { database: "db".into(), table: "t".into(), rows: 5 });
    let ev = sub.try_recv().expect("event delivered");
    assert_eq!((ev.database.as_str(), ev.table.as_str(), ev.rows), ("db", "t", 5));
}

#[tokio::test]
async fn publish_without_subscribers_does_not_error() {
    let bus = IngestEvents::new(8);
    bus.publish(RowsAppended { database: "db".into(), table: "t".into(), rows: 1 }); // must not panic
}
```

- [ ] **Step 2: Implement `events.rs`:**

```rust
//! In-process "rows appended" notifications. Published by the write path
//! after a commit makes rows queryable; consumed by live Discover sessions.
//! Lossy by design (bounded broadcast) — consumers fall back to timer scans.

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct RowsAppended {
    pub database: String,
    pub table: String,
    pub rows: u64,
}

#[derive(Debug, Clone)]
pub struct IngestEvents {
    tx: broadcast::Sender<RowsAppended>,
}

impl IngestEvents {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }
    /// Fire-and-forget: no subscribers is fine; lagging subscribers drop.
    pub fn publish(&self, ev: RowsAppended) {
        let _ = self.tx.send(ev);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RowsAppended> {
        self.tx.subscribe()
    }
}
```

- [ ] **Step 3: Wire into WritePath.** Add `events: Option<IngestEvents>` field + builder `pub fn with_events(mut self, events: IngestEvents) -> Self`. In `ingest_with_idempotency`, AFTER the ack is obtained (single post-commit point covering all three commit modes), publish `RowsAppended { database, table, rows }`. **Check what `ingest_with_idempotency` actually has in scope**: it takes `&TableRef` — verify whether TableRef carries the database name (read kyma-core's TableRef). If it does NOT, change the publish site to use whatever identifier the callers pass (or thread `database: &str` through — the REST handler has it; if signature change is needed across the 5 callers, do it, it's mechanical). Report which shape you found.
- [ ] **Step 4: Construct + attach in both binaries.** kyma-local/src/lib.rs (~290): `let ingest_events = IngestEvents::new(256);` then `WritePath::new(...).with_events(ingest_events.clone())`. Same in kyma-bin/src/main.rs (~257-275, BOTH staging and non-staging arms). Keep `ingest_events` in scope for later mounting (Task 4) — for now just hold it; a TODO comment is fine. Also attach to the memory writer's WritePath ONLY if trivial (kyma-memory/src/writer.rs constructs its own) — if it needs plumbing through MemoryWriter's constructor, skip and note it (memory db is excluded from All-scope anyway).
- [ ] **Step 5: Verify** `cargo test -p kyma-ingest-core` green; `cargo build -p kyma-cli -p kyma-bin` compiles (kyma-bin needs cmake — if the build fails on rdkafka/cmake env, `cargo check -p kyma-bin` is acceptable evidence).
- [ ] **Step 6: Commit** the named files: `git commit -m "feat(ingest): rows-appended broadcast bus published post-commit"`.

---

### Task 2: Frame variants Live + Heartbeat (+ SDK mirror)

**Files:**
- Modify: `crates/kyma-server/src/discover/frames.rs`
- Modify: `web/src/sdk/discover.ts` (Frame union), `web/src/features/discover/discover-store.ts` (applyFrame), `web/src/features/discover/types.ts` (status)

- [ ] **Step 1: Failing Rust test** in frames.rs:

```rust
#[test]
fn live_and_heartbeat_frames_serialize() {
    let v: serde_json::Value = serde_json::from_str(frame_to_line(&Frame::Live).trim()).unwrap();
    assert_eq!(v["type"], "live");
    let v: serde_json::Value = serde_json::from_str(frame_to_line(&Frame::Heartbeat).trim()).unwrap();
    assert_eq!(v["type"], "heartbeat");
}
```

- [ ] **Step 2: Add unit variants** `Live,` and `Heartbeat,` to the Frame enum (serde tag handles them). Run `cargo test -p kyma-server --lib discover::frames` green.
- [ ] **Step 3: Frontend mirror.** `sdk/discover.ts`: add `| { type: "live" } | { type: "heartbeat" }` to the Frame union. `types.ts`: `DiscoverResultsState["status"]` gains `"live"`. `discover-store.ts` applyFrame: `case "live": next.status = "live"; return next;` and `case "heartbeat": return next;` (no-op, keeps types exhaustive). Add a store test: applying `{type:"live"}` after a plan sets status "live".
- [ ] **Step 4: Verify** web typecheck + `npx vitest run src/features/discover` green. **Commit** `feat(discover): live + heartbeat frames`.

---

### Task 3: WS session handler — auth, subscribe, backfill, live loop

**Files:**
- Create: `crates/kyma-server/src/discover/live.rs`
- Modify: `crates/kyma-server/src/discover/mod.rs` (`pub mod live;`)
- Modify: root `Cargo.toml` axum workspace dep: add `"ws"` to features
- Test: unit tests inside live.rs for the pure pieces; WS integration test comes in Task 4

**Protocol (from the spec):** client sends `{"type":"auth","token":"…"}` within 5s, then `{"type":"subscribe","query":"…","scope":{…},"time_range":{from,to}|null,"per_source_limit":N}`; may later send `{"type":"update",…same fields…}`, `{"type":"pause"}`, `{"type":"resume"}`. Server sends backfill frames (Plan/Rows/Histogram/SourceDone — reusing fanout, suppressing its `Done`), then `{"type":"live"}`, then incremental `Rows`/`Error` frames, `{"type":"heartbeat"}` every 15s. Auth failure/timeout → close. Each frame is one WS text message (the JSON line, no trailing newline needed).

- [ ] **Step 1: Router + handler skeleton.** Public router constructor (NO auth middleware — mounted outside it):

```rust
pub fn explore_live_router(
    state: crate::QueryState,
    backend: std::sync::Arc<dyn crate::auth::AuthBackend>,
    events: Option<kyma_ingest_core::events::IngestEvents>,
) -> axum::Router {
    use axum::routing::get;
    let shared = LiveDeps { state, backend, events };
    axum::Router::new().route(
        "/v1/explore/live",
        get(move |ws: axum::extract::ws::WebSocketUpgrade| {
            let deps = shared.clone();
            async move { ws.on_upgrade(move |sock| session(sock, deps)) }
        }),
    )
}
```

(kyma-server must gain a dependency on kyma-ingest-core if it doesn't have one — check Cargo.toml; if adding it is heavy/cyclic, define the event types in a crate both already depend on (kyma-core) instead, and adjust Task 1 — report which you did.)

- [ ] **Step 2: Session control messages** (serde):

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Auth { token: String },
    Subscribe(SubscribeBody),
    Update(SubscribeBody),
    Pause,
    Resume,
}
#[derive(serde::Deserialize)]
struct SubscribeBody {
    #[serde(default)]
    query: String,
    scope: super::scope::Scope,
    #[serde(default)]
    time_range: Option<super::handler::TimeRangeBody>,
    #[serde(default)]
    per_source_limit: Option<usize>,
}
```

(make `TimeRangeBody` + `parse_time_range` in handler.rs `pub(crate)` as needed.)

- [ ] **Step 3: Session loop.** Structure (adapt while reading the existing handler for scope resolution / grammar parse / budget defaults — budget: use defaults, no headers on WS):

```rust
async fn session(mut ws: WebSocket, deps: LiveDeps) {
    // 1. AUTH: first message within 5s or close.
    let Some(principal) = await_auth(&mut ws, &deps.backend).await else { return };
    if principal.role < Role::Read { let _ = ws.send(close_frame("forbidden")).await; return; }
    let tenant = principal.tenant;

    // 2. SUBSCRIBE: next message must be Subscribe.
    let Some(sub) = await_subscribe(&mut ws).await else { return };

    'resub: loop {
        // 3. Resolve scope + parse grammar (reuse resolve_scope / parse_grammar — same
        //    error frames as the HTTP handler, sent as WS messages, then continue/close).
        // 4. BACKFILL: run_fanout(FanoutInput{...sub fields...}, tx); forward frames,
        //    translating: Done → send Frame::Live instead and break to live loop.
        //    Track `cursor_to_ms` = the backfill's effective `to` (sub.time_range.to or now()).
        // 5. LIVE LOOP: tokio::select! over:
        //    - ws.recv(): Update → replace sub, continue 'resub; Pause → set paused;
        //      Resume → run catch-up scan from cursor, unset paused; Close/None → return.
        //    - events_rx.recv() (if Some): collect touched sources ∩ resolved set into a
        //      pending set; arm a 150ms debounce sleep.
        //    - debounce elapsed (when pending non-empty && !paused): incremental scan.
        //    - fallback interval (30s, !paused): incremental scan over ALL resolved sources.
        //    - heartbeat interval (15s): send Frame::Heartbeat.
        //
        //    Incremental scan = run_fanout over the touched subset with
        //    TimeRange{from_ms: cursor_to_ms, to_ms: now_ms}; forward ONLY Rows/Error
        //    frames (drop Plan/SourceProgress/Histogram/SourceDone/Done); on its Done,
        //    advance cursor_to_ms = now_ms. Sources without a timestamp column never
        //    produce incremental rows (compile skips the time filter — so SKIP no-ts
        //    sources from incremental scans entirely to avoid re-sending full pages).
    }
}
```

Key correctness details to honor:
- **Skip no-timestamp sources in incremental scans** (else every tick re-sends the same `take N` page). The resolved set's per-source `has_timestamp` comes from `compile_for_source` — compile once per resub to classify.
- **Cursor advances only after a scan completes** (on the mini-fanout's Done), to `now` captured BEFORE the scan started — rows committing mid-scan land in the next window.
- **Paused**: events still drain (don't let the receiver lag), pending set keeps accumulating, no scans/sends; Resume runs one catch-up scan.
- **broadcast Lagged error**: treat as "scan everything" (set pending = all sources), continue.
- **No events bus (None)**: timer-only mode — same loop minus the events arm.
- `let _ = ws.send(...)` failures → return (client gone).

- [ ] **Step 4: Pure-logic unit tests** in live.rs: (a) ClientMsg deserialization for all 5 message types; (b) the window math: consecutive `[from,to)` windows from a monotonic clock never overlap (test the small `next_window(cursor, now)` helper you extract); (c) touched-set intersection: events for sources outside the resolved set are ignored. Keep the session loop itself for the integration test (Task 4).
- [ ] **Step 5: Verify** `cargo test -p kyma-server --lib discover` green; `cargo build -p kyma-cli` (root Cargo.toml change: axum features now `["macros", "http2", "ws"]`).
- [ ] **Step 6: Commit** `feat(discover): live WebSocket session (auth, backfill, notify-driven tail)`.

---

### Task 4: Mount in both deployments + capabilities + integration test

**Files:**
- Modify: `crates/kyma-local/src/lib.rs` (merge `explore_live_router` WITHOUT auth layer; pass the Task-1 `ingest_events`)
- Modify: `crates/kyma-bin/src/main.rs` (same, in the final merge block ~634-649)
- Modify: `crates/kyma-server/src/capabilities.rs` (`explore_live: bool`, true in BOTH `SERVER` and `LOCAL` consts; check the web SDK's Capabilities type — mirror the field in `web/src/sdk/*` where capabilities are typed)
- Test: `crates/kyma-local/tests/` or wherever kyma-local's HTTP tests live (find them) — a test that the route exists and is reachable WITHOUT auth: plain GET (no upgrade headers) to `/v1/explore/live` must NOT return 401/404/405 (axum ws responds 426 or 400 to non-upgrade GETs — assert one of those, pinning "mounted + public"). Add tokio-tungstenite as a dev-dependency ONLY if you also write a full upgrade test (optional; the reachability test is the required gate — it prevents the dashboards-405 class of bug).

- [ ] Steps: failing mount test → mount in kyma-local → mount in kyma-bin → capabilities + SDK mirror → all tests green (`cargo test -p kyma-local`, `cargo test -p kyma-server --lib`, web typecheck) → commit `feat(discover): mount /v1/explore/live in local + server, capability flag`.

---

### Task 5: Frontend live client (SDK)

**Files:**
- Create: `web/src/sdk/discover-live.ts`
- Test: `web/src/sdk/discover-live.test.ts` (mock WebSocket class injected)

- [ ] **Step 1: Failing tests** for a `LiveSession` class with injectable WS factory: (a) sends auth as first message then subscribe; (b) emits parsed frames via onFrame callback; (c) `update()` sends an update message; (d) on close with code ≠ normal, schedules reconnect with backoff (test with fake timers) and resubscribes with `time_range.from` = last-seen event time minus 5s overlap; (e) `close()` prevents reconnect.
- [ ] **Step 2: Implement.** Shape:

```ts
export type LiveStatus = "connecting" | "live" | "backfilling" | "closed" | "error";
export class LiveSession {
  constructor(opts: {
    endpoint: string; token: string;
    body: { query: string; scope: Scope; time_range: { from: string; to: string } | null; per_source_limit?: number };
    onFrame: (f: Frame) => void;
    onStatus: (s: LiveStatus) => void;
    wsFactory?: (url: string) => WebSocket; // test seam
  })
  update(body: …): void
  close(): void
}
```
URL derivation: `endpoint.replace(/^http/, "ws") + "/v1/explore/live"`. Track `lastEventAt` (max wall-clock of received rows frames) for reconnect overlap. Backoff: 1s, 2s, 4s … cap 30s, reset on successful `live` frame.
- [ ] **Step 3:** Tests green + typecheck. **Commit** `feat(discover): live WebSocket SDK client`.

---

### Task 6: Live mode UI

**Files:**
- Modify: `web/src/features/discover/useDiscoverSearch.ts` (or new `useDiscoverLive.ts` — judgment: keep one hook that takes `live: boolean` and swaps transport, reusing `applyFrame` for both)
- Modify: `web/src/features/discover/DiscoverPage.tsx` (Live toggle button in the toolbar: 🔴 Live / ⏸; status dot; pass live state)
- Modify: `web/src/features/discover/SummaryLine.tsx` (status "live": render `· 🔴 live` instead of "as of", plus last-heartbeat-stale indicator if > 45s)
- Modify: `web/src/features/discover/StreamView.tsx` (when not scrolled to top and new rows arrive, freeze rendering at the previous row set and show a "N new events ↑" pill that scrolls to top + unfreezes; track via scroll container ref)
- Modify: `web/src/features/discover/discover-store.ts` ONLY if state needs a `live: boolean` per tab (it does: persist the toggle per tab — add `live: boolean` to DiscoverTabState with default false; NO migration bump needed — rehydrate tolerates missing booleans via `?? false` in accessors… verify how other optional fields are handled; if strict, bump to v4 with a defaulting migration following the v3 pattern)
- Tests: store test for the live flag default; StreamView freeze logic extracted as a pure helper with a test (e.g. `bufferedCount(prevLen, nextLen)`); SummaryLine live-state formatter test.

- [ ] Steps: failing tests for the pure parts → implement → wire DiscoverPage (live toggle starts a LiveSession with the CURRENT submitted query/scope/range; editing query while live calls `session.update(...)`; toggle off closes and falls back to one-shot search) → typecheck + full `npx vitest run src/features` green → commit `feat(discover): live mode UI — toggle, status, new-events buffer`.

---

### Task 7: End-to-end verification + docs

- [ ] **Step 1:** Rebuild + restart local serve (see memory: KYMA_AUTH_TOKENS pattern, check `lsof :8080` for squatters). Hard-reload :5173/discover.
- [ ] **Step 2:** Scripted WS check (node, run from web/ where `ws` types exist — or use a browser via the user): connect, auth, subscribe `{scope: all}`, observe backfill frames then `live`; then `curl POST /v1/ingest` a row into `demo.events` and assert a `rows` frame arrives within ~1s. Save as `scripts/discover-live-smoke.mjs` (plain node, no deps — Node 22 has built-in WebSocket).
- [ ] **Step 3:** UI eyeball with the user: toggle Live, ingest a row (`curl`), watch it appear; scroll down, ingest, see the "new events" pill.
- [ ] **Step 4:** Commit plan doc + smoke script; merge flow per finishing-a-development-branch.

---

## Self-review notes

- **Spec coverage:** WS endpoint + first-message auth (T3), both mounts + capability (T4), ingest-notify + debounce + timer fallback + heartbeat (T1/T3), backfill-then-live frame contract (T3), reconnect-with-overlap (T5), live UX: toggle/status/pause-on-scroll/new-events pill (T6), error handling per spec §error-handling (T3 close codes, T5 status surface, T6 fallback to one-shot).
- **Consistency:** `IngestEvents`/`RowsAppended` named once in T1, consumed T3/T4; `Frame::Live`/`Heartbeat` from T2 used in T3/T5/T6; `LiveSession` API of T5 consumed in T6.
- **Known judgment calls:** pause is client-initiated protocol but the scroll-freeze UX is client-side only (server pause reserved for explicit toggle/scroll-long-idle later); incremental scans reuse the full fanout per tick (cheap at local scale; extracting a leaner per-source scan is a later optimization); same-millisecond duplicate rows on window boundaries are accepted per spec.
