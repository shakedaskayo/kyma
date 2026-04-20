# kyma Web UI — Foundation + Query Workspace (A + B)

**Date:** 2026-04-20
**Status:** Approved; implementation plan pending.
**Scope:** Sub-projects **A (Foundation)** and **B (Query Workspace)** of the
kyma web platform. Dashboards (C), admin/ops (D), and distribution polish (E)
are out of scope here and will each get their own spec.

## Summary

Ship an ADX-Web-UI-class explorer for kyma that runs both as a **single-binary
cloud deploy** (UI embedded in `kyma-server`) and as a **Tauri 2 desktop app**.
The end-to-end user experience is: open the app, paste a kyma URL + bearer
token, get a KQL editor with live schema-aware autocomplete, run queries
against a Flight-over-gRPC-web backend, see a virtualized results grid and an
auto-configured chart, share the tab via URL, and flip between multiple
kyma deployments with one click.

A is the foundation (repo structure, SDK, server additions, packaging). B is
the first end-user feature (the Query Workspace at `/explore`). Together they
are the minimum slice that makes the platform demonstrably useful.

## Goals

- End-user value in one round of work: a human can explore kyma data
  graphically without writing a single line of UI code themselves.
- Reuse what the engine already ships — the existing `FlightService`, KQL
  parser, catalog trait, bearer-token auth, and query-budget headers are all
  unchanged on the hot path.
- Honor the README's "one binary" promise. The UI compiles into
  `kyma-server` under a `web-ui` feature flag; headless prod deployments
  carry zero UI weight.
- Lay every piece that C (dashboards) and D (ops surface) will reuse — SDK,
  session store, schema cache, component library, styling system.

## Non-goals (explicit)

- No saved queries, dashboards, or alerts (C's problem).
- No user/team management or RBAC beyond the existing bearer-token roles
  (D's problem).
- No TLS termination, no multi-tenant hosting, no SSO (D / E).
- No visual-regression CI, no native Tauri plugins beyond secure token
  storage, no macOS/Windows code signing.

## Locked decisions

| Decision | Choice |
|---|---|
| Frontend framework | **Vite + React + TypeScript** |
| Desktop shell | **Tauri 2** (same `dist/` bundle) |
| Query protocol from browser | **Apache Arrow Flight via gRPC-web** (`tonic-web` on the server) |
| Auth UX (A+B) | **Bring-your-own bearer token**, pasted into Settings; stored in `localStorage` (web) or OS keyring via `@tauri-apps/plugin-store` (desktop) |
| Editor | **Monaco** with a custom `kql` language registration |
| Charts | **ECharts** |
| Grid | **TanStack Table + `@tanstack/react-virtual`** |
| UI system | **Tailwind CSS + shadcn/ui** (Radix primitives, copy-in components) |
| State | Tiny **zustand** stores (session, workspace); **@tanstack/react-query** for schema cache |
| Packaging model | **Single binary** (UI baked into `kyma-server` via `include_dir!` under `--features web-ui`) |

---

## 1. System architecture

Two new top-level entries in the repo:

```
engine/
├── crates/
│   ├── kyma-server/         # gains: static-file route, tonic-web, /v1/catalog
│   └── kyma-web-assets/     # NEW: include_dir!-wraps web/dist
└── web/                     # NEW Vite + React + TS workspace
    ├── src/
    ├── src-tauri/           # Tauri 2 wrapper
    └── dist/                # built by `pnpm build`
```

Build chain:

1. `pnpm -C web build` → `web/dist/`
2. `cargo build -p kyma-bin --release --features web-ui` → single binary with
   UI bytes baked in
3. `pnpm -C web tauri build` (delegates to `@tauri-apps/cli`) → desktop
   installers using the same `dist/`

Runtime model: one `kyma-server` process. `axum` listens on `:8080` (HTTP,
static files, `/v1/catalog`, existing `/v1/query`, `/v1/ingest`,
`/flight/*` via `tonic-web`). The native Flight `tonic` server continues to
listen on `:9090` for server-to-server clients. Browsers hit `:8080` only.
Cloud deploy is `docker run -p 8080:8080 kyma:latest`.

Tauri desktop connects to any user-configured kyma endpoint over HTTPS/HTTP;
it does not host `kyma-server` itself.

## 2. Server-side additions (kyma-server)

### 2.1 `/v1/catalog/schema` endpoint

- `GET /v1/catalog/schema` → JSON:
  ```json
  { "databases": [ {
      "name": "obs",
      "tables": [ {
        "name": "otel_traces",
        "columns": [
          {"name": "timestamp", "type": "datetime", "nullable": false},
          {"name": "service.name", "type": "string", "nullable": false},
          {"name": "attributes", "type": "dynamic", "nullable": true}
        ]
      } ]
    } ] }
  ```
- Implemented against the existing `Catalog` trait: list databases → list
  tables → resolve latest `schema_snapshot` per table.
- Cached in-memory with a 5 s TTL keyed by database; invalidated on
  `alter_table_add_column`.
- Requires bearer token with `read` role. Returns 403 otherwise.

### 2.2 `tonic-web` middleware

- The existing `FlightService` gains a `tonic-web` layer that exposes the
  same service at `/flight/*` on the HTTP listener.
- No changes to `do_get` hot path; the Ticket protocol
  (`{database, query, language}`) and the `FlightDataEncoderBuilder`
  response stream are unchanged.
- Native Flight on `:9090` is preserved as-is.

### 2.3 Static UI serving (feature-gated)

- New crate `kyma-web-assets`: `include_dir!("../../web/dist")`. Compiles
  bytes into the binary at build time.
- `kyma-server` gains, behind `#[cfg(feature = "web-ui")]`:
  - `GET /` → `index.html`
  - `GET /assets/<path>` → matched file with correct `Content-Type`
  - SPA fallback: unknown paths not under `/v1/`, `/flight/`, `/metrics`,
    `/health` → `index.html` (for client-side routing)
  - Hashed asset filenames → `Cache-Control: immutable` for 1 year;
    `index.html` → `no-cache`.
- Auth middleware updated to allow unauthenticated `GET /` and
  `GET /assets/*`. `/health` and `/metrics` remain open as today. All
  `/v1/*` and `/flight/*` routes remain auth-gated.
- With the feature off (default for headless prod), none of the static-serve
  code compiles in.

## 3. Frontend structure

```
web/
├── src-tauri/
├── public/
├── src/
│   ├── main.tsx
│   ├── app/
│   │   ├── router.tsx              TanStack Router, file-based routes
│   │   ├── providers.tsx           QueryClient, theme, toast, session
│   │   └── shell.tsx               top bar + left rail + outlet
│   ├── routes/
│   │   ├── index.tsx               → redirect /explore
│   │   ├── explore.tsx             Query Workspace (Section 4)
│   │   └── settings.tsx            server URL + bearer token
│   ├── sdk/
│   │   ├── client.ts               createClient({endpoint, token})
│   │   ├── catalog.ts              fetch /v1/catalog/schema
│   │   ├── query.ts                Flight gRPC-web client
│   │   ├── arrow.ts                RecordBatch → grid/chart adapter
│   │   └── types.ts                ts-rs-generated types
│   ├── features/
│   │   ├── editor/                 Monaco + kql language + completion
│   │   ├── schema-browser/
│   │   ├── results-grid/
│   │   ├── chart/
│   │   ├── time-range/
│   │   └── tabs/
│   ├── components/ui/              shadcn/ui primitives
│   ├── lib/                        url-state codec, kbd, formatters
│   └── styles/
├── index.html
├── tailwind.config.ts
├── tsconfig.json
└── package.json
```

### State model

- **Session state** (server URL, bearer token, active database) — zustand
  store persisted to `localStorage`.
- **Schema cache** — `@tanstack/react-query`; `useSchema()` hook with
  `staleTime: 5 min`, background revalidation.
- **Workspace state** — zustand; open tabs and each tab's
  `{ query, timeRange, results, chartConfig }`, persisted to `localStorage`
  keyed by server URL (so hopping between kyma instances does not clobber).
- **URL state** — `/explore?q=<b64-gzip>&from=<ts>&to=<ts>` encodes the
  active tab's query and time range; first-class shareable.

### SDK surface

```ts
const kyma = createClient({ endpoint, token });
const schema = await kyma.catalog.schema();
const stream = kyma.flight.doGet({ database, query, language: "kql" });
for await (const batch of stream) { /* Arrow RecordBatch */ }
```

Both cloud-web and Tauri use the same client; Tauri defaults `endpoint` to
`http://localhost:8080` and loads the token from the OS keyring.

## 4. Query Workspace UX (`/explore`)

### Layout

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  kyma     [obs ▼ database]   [⌂ Explore] [⚙ Settings]      me@host · 2.1.0 │
├──────────┬──────────────────────────────────────────────────────────────────┤
│ SCHEMA   │  [●  payments errors]  [  session walk ]  [ + ]                  │
│ obs      │ ┌────────────────────────────────────────────────────────────┐   │
│  ▾ otel  │ │ Time range: [Last 1h ▼]  ⌘↵ Run  ⌘⇧↵ Run selection          │   │
│    tr…   │ │──────────────────────────────────────────────────────────── │   │
│          │ │ 1  otel_traces                                             │   │
│          │ │ 2  | where service.name == "payments-svc"                  │   │
│          │ │ 3  | summarize n = count() by span_name                    │   │
│          │ └────────────────────────────────────────────────────────────┘   │
│          │  [ Results (12,831 · 412 ms) ]  [ Chart ]                        │
│          │ ┌────────────────────────────────────────────────────────────┐   │
│          │ │ span_name           │ n                                    │   │
│          │ │ http.client.auth…   │ 8,412                                │   │
│          │ └────────────────────────────────────────────────────────────┘   │
└──────────┴──────────────────────────────────────────────────────────────────┘
```

### Component behavior

- **Schema browser (left rail, resizable).** Tree `database → table →
  columns`. Column row shows type glyph + name. Click a column → inserts at
  cursor. Lazy-expand; cached via `useSchema()`.
- **Tab bar.** Each tab owns `{query, timeRange, results, chartConfig}`.
  `⌘T` new, `⌘W` close, `⌘1..9` jump. Dirty indicator when editor text
  differs from last-run.
- **Time-range picker.** Presets: 5 m / 15 m / 1 h / 6 h / 24 h / 7 d / 30 d
  / custom. Emits a `-- time: last 1h` preamble the runner rewrites into
  `timestamp between (ago(1h) .. now())` on the leading source's timestamp
  column (server's KQL parser already resolves).
- **Editor (Monaco + `kql`).** Tokenizer colorizes KQL keywords
  (`where`, `summarize`, `by`, ...), functions (`ago`, `bin`, `count`,
  `avg`, `tostring`, ...), strings, numbers, comments. Completion provider
  fires on `project `, `where `, `by `, `extend `, `| ` → suggests columns
  of the leading source or operators, respectively. Diagnostics (parse
  errors) come from 400 responses — see Section 6.
- **Run.** `⌘↵` runs the current tab. Status bar reports submitted-at,
  duration, rows, bytes, extents-scanned (from Flight response trailer).
  `Cancel` aborts the gRPC-web stream.
- **Results grid.** TanStack Table + virtualization. Client-side sort by
  any column. CSV/JSON export of the current page or full result. Click a
  cell copies; shift-click adds `| where col == 'val'`.
- **Chart panel.** Same data as grid. Auto-pick:
  - 1 numeric + 1 time → line
  - 1 categorical + 1 numeric → bar
  - 2 numerics → scatter
  - 1 numeric only → stat
  - otherwise → unselected state with helper text
  User override: chart-type dropdown + X/Y/series pickers.
- **⌘K command palette.** Fuzzy search every action (run, open tab, switch
  server, insert snippet, export, ...).
- **Server switcher.** Top-bar chip shows current endpoint; click opens
  Settings. Per-server workspace isolation.

## 5. Data flow — one user query, end to end

1. User presses `⌘↵` in the active tab's editor.
2. Runner reads `{query, timeRange, database}` from tab state and prepends
   a time-filter preamble unless the user wrote their own
   `timestamp between (…)`.
3. SDK: `kyma.flight.doGet({database, query, language: "kql"})` emits a
   gRPC-web framed `DoGet` to `POST :8080/flight/arrow.flight.protocol.FlightService/DoGet`
   with `Authorization: Bearer <token>` and
   `X-Kyma-Max-Wall-Clock-Ms` / `X-Kyma-Max-Memory-Bytes` from tab settings.
4. `tonic-web` unwraps frames → existing `FlightService::do_get` runs
   unchanged: KQL → SQL via `kyma-kql::translate` → DataFusion
   `SessionContext` → `FlightDataEncoderBuilder` stream.
5. Client consumes the stream with `@bufbuild/connect-web` +
   `apache-arrow`'s `RecordBatchStreamReader`. First batch's Arrow schema
   drives grid columns + default chart axes. Each subsequent batch
   appends rows into the grid store and downsamples into the chart buffer
   (≤ 4 k points for line charts).
6. Status bar updates live (`streaming · 18,200 rows`). Stream end →
   finalize timing, persist tab snapshot to `localStorage`.

**Schema refresh.** `useSchema()` loads on app boot, revalidates every 5
minutes, and force-refreshes when a query response carries
`X-Schema-Changed: true` (server emits this on DDL).

## 6. Error and loading states

- **Network unreachable** → full-screen "can't reach kyma" card with
  Settings link.
- **401 / 403** → inline banner in Settings with the `WWW-Authenticate`
  scope hint.
- **KQL parse error** (400 `{error:{code:"kql_parse", ...}}`) → red
  underline in Monaco at the reported span with hovertext.
- **Query runtime error** → toast + results tab swaps to an error card
  showing `request_id` (server-propagated `X-Request-ID`) and a "copy as
  bug report" button.
- **Budget exceeded** (429 `Retry-After`) → toast with a "bump budget for
  this tab" action that doubles the tab's wall-clock and memory limits on
  retry, up to a hard ceiling (60 s / 2 GiB) after which the button
  disables.
- **Stream cut mid-flight** → partial grid/chart marked
  "partial · connection lost"; `Retry` affordance.
- **Schema load**: shimmer on first load per server; silent background
  revalidation thereafter.
- **Query load**: Run becomes indeterminate spinner + `Cancel`; status bar
  streams row count; chart throttled to RAF.
- **Empty result** → friendly card ("0 rows in 28 ms — try widening the
  time range") instead of blank grid.
- **Reconnect watcher** pings `/health` every 30 s when idle; "reconnecting"
  chip on failure. In-flight queries are not auto-retried.

## 7. Testing strategy

**Rust (kyma-server additions)**

- `tests/catalog_http.rs` — integration test via `testcontainers`. Empty
  catalog → `{databases: []}`; seeded tables → full tree; `read`-role
  required; cache TTL honored.
- `tests/flight_web_smoke.rs` — gRPC-web `DoGet` using `reqwest`; first
  Arrow IPC frame decodes. Extends the existing `tests/flight_smoke.rs`.
- `tests/web_assets.rs` (compiled with `--features web-ui`) — `GET /`
  returns HTML; `GET /assets/<hash>.js` has `immutable` cache headers;
  SPA fallback routes unknown paths to `index.html`.

**TypeScript (Vitest + Testing Library + Playwright)**

- Unit: SDK factory, URL-state codec, time-range → KQL preamble, Arrow →
  grid adapter, chart auto-axis picker.
- Component: schema browser expand/collapse, grid sort/filter/export,
  editor completion provider against a synthetic schema.
- E2E (Playwright, runs in CI against `docker-compose up`): settings →
  paste token → write KQL → run → see grid → switch to chart → share URL
  → reload → tab state restored.

**Not in scope:** visual-regression gates, editor-fuzz tests, grid-perf
benchmarks.

## 8. Packaging

### Cloud

- Root `Dockerfile` (multi-stage):
  1. `node:20 + pnpm` → build `web/dist/`
  2. `rust:1.84` → `cargo build -p kyma-bin --release --features web-ui`
  3. `gcr.io/distroless/cc` → copy binary, expose `:8080` and `:9090`.
- `docker-compose.yml` adds a `kyma` service alongside Postgres, MinIO,
  Redpanda. `docker-compose up` from a clean clone launches the entire
  platform at `http://localhost:8080`.
- TLS out of scope; recommend Caddy or nginx in front for HTTPS. Documented
  in the README update that ships with this change.

### Desktop (Tauri 2)

- `web/src-tauri/` scaffolded with `tauri init`. `distDir: ../dist`,
  `devPath: http://localhost:5173`.
- First-run flow: empty Settings → user enters server URL + token → UI
  works. Default server URL `http://localhost:8080`.
- Token persistence via `@tauri-apps/plugin-store` (macOS Keychain, Windows
  Credential Vault, Linux Secret Service).
- `tauri build` → `.dmg`, `.msi`, `.AppImage`. Signing deferred.

### Dev loop

- `pnpm -C web dev` → Vite on 5173, proxies `/v1/*` and `/flight/*` to a
  local `cargo run -p kyma-bin`. Hot reload works for UI; server edits
  require a rebuild.

### CI

- `.github/workflows/web.yml` — `pnpm lint + typecheck + vitest + build`
  on `web/`.
- `.github/workflows/release.yml` — Tauri build matrix
  (linux/mac/windows), uploads installers. Tracked but off A+B's critical
  path.

## Open questions (tracked, not blocking)

- Whether to add a `PromQL` frontend hint in the editor language selector
  before PromQL lands in the engine. Leaning no — one frontend at a time.
- Whether to codegen the TS SDK types from `kyma-web-sdk` with `ts-rs` or
  with `schemars` + `quicktype`. Either works; pick during implementation.
- Whether `X-Schema-Changed` should be a response *trailer* (Flight) or a
  response *header* (HTTP). Both are supportable; pick during the server
  PR review.

## What's deferred to later specs

- **C (Dashboards):** saved panels, composition, refresh intervals, shared
  state across panels, layout editor.
- **D (Ops surface):** users, teams, RBAC beyond bearer-token roles,
  data-source config, alerts, scheduled queries.
- **E (Distribution):** TLS, SSO/OIDC, code signing for Tauri, embeddable
  iframe mode, multi-tenant hosting.
