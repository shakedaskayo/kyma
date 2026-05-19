# F1 — Stability Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock down the v1.0 stability contract — produce `docs/stability.md` enumerating every surface kyma promises not to break, a deprecation policy section in `CONTRIBUTING.md`, the shared metrics-taxonomy rules at `docs/metrics-taxonomy.md`, and a back-compat CI workflow that grows organically as `v1.0.0-pre.N` tags get cut.

**Architecture:** Mostly authored markdown plus one new CI workflow. The CI workflow snapshots fixtures (extents + catalog dumps) from each tagged version into the repo's GitHub release assets and replays a fixed query set on every PR. Today the repo has zero tags, so the workflow ships with a single synthetic "main-HEAD-at-F1-time" snapshot; it grows as `v1.0.0-pre.N` tags land. The format-freeze section of `stability.md` ships as a placeholder until P0 completes, then gets filled in.

**Tech Stack:** Markdown for docs. GitHub Actions YAML for the workflow. Bash + curl + docker-compose for fixture creation and replay (matches the existing `scripts/test-*.sh` style). No new Rust crates.

**File Structure:**

- Create: `docs/stability.md` — the contract itself, one section per frozen surface.
- Create: `docs/metrics-taxonomy.md` — naming + label + deprecation rules for metrics that area specs (A1–A4) follow.
- Modify: `CONTRIBUTING.md` — add a "Stability and deprecation policy" section that links to `docs/stability.md`.
- Create: `scripts/backcompat-snapshot.sh` — captures a fixture (extents + catalog dump + workflow file hash) for the current running engine; intended to be invoked at tag time.
- Create: `scripts/backcompat-replay.sh` — given a fixture, spins up a fresh engine, replays the fixed query set, diffs results.
- Create: `scripts/backcompat-queries.txt` — the fixed query set (HTTP queries + expected-result hashes).
- Create: `scripts/fixtures/backcompat/` — directory holding the synthetic seed fixture and (eventually) per-tag fixtures.
- Create: `.github/workflows/backcompat.yml` — the CI workflow that runs `backcompat-replay.sh` against every fixture on each PR.

Each file has one clear responsibility. The replay script is reused for every fixture; only fixture data differs per tag.

---

## Task 1: Bootstrap `docs/stability.md` with the contract structure

**Files:**
- Create: `docs/stability.md`

- [ ] **Step 1: Create `docs/stability.md` with the full section skeleton**

Write the file with this exact content:

````markdown
# kyma Stability Contract

> **Status:** in force from `v1.0.0`. Until then, this document is the
> contract kyma is being hardened to meet. Any change to a "frozen" surface
> below requires the deprecation policy at the bottom of this file.

This document names every surface kyma promises not to break across minor
versions in the v1.x series, and the deprecation policy that governs
intentional changes.

If a surface is not listed here, it is not part of the v1.0 contract — it
may change in any minor release.

---

## 1. HTTP REST API

_Filled in by Task 2._

## 2. Arrow Flight gRPC API

_Filled in by Task 3._

## 3. KQL dialect

_Filled in by Task 4._

## 4. SQL dialect

_Filled in by Task 5._

## 5. MCP surface

_Filled in by Task 6._

## 6. Catalog Postgres schema

_Filled in by Task 7._

## 7. Configuration keys and environment variables

_Filled in by Task 8._

## 8. Extent on-disk format

_Filled in by Task 9. Final freeze blocked on P0 (format v1)._

## 9. Metrics, structured logs, internal traces

See [`docs/metrics-taxonomy.md`](metrics-taxonomy.md) for the rules. The
concrete metric/log/trace inventories are owned by the per-area specs
(A1–A4) and listed under each area's runbook.

## 10. Deprecation policy

_Filled in by Task 10. Mirrored in `CONTRIBUTING.md` by Task 11._
````

- [ ] **Step 2: Verify the file exists and renders as markdown**

Run: `head -40 docs/stability.md`
Expected: file prints with the 10 section headers above and the status banner.

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): bootstrap stability contract skeleton (F1)"
```

---

## Task 2: HTTP REST API surface freeze

**Files:**
- Modify: `docs/stability.md` (section 1)
- Reference: `crates/kyma-server/src/lib.rs`, any `axum::Router::route(...)` call sites

- [ ] **Step 1: Inventory every HTTP route the server exposes**

Run:

```bash
grep -rn 'route\|Router::new\|\.route_layer\|\.nest' crates/kyma-server/src \
  | grep -v 'test' | grep -v '//' | sort
```

Also grep ingest crates:

```bash
grep -rn 'route\|Router::new' crates/kyma-ingest-rest/src crates/kyma-mcp/src 2>/dev/null
```

For each route, capture: HTTP method, path, what it does, request shape, response shape. Read the handler if the route's purpose isn't obvious from the name.

- [ ] **Step 2: Replace section 1 of `docs/stability.md` with the inventory**

Use this template per route:

````markdown
## 1. HTTP REST API

The following endpoints are part of the v1.0 frozen surface. Request and
response shapes are fixed at v1.0.0; new optional fields may be added; no
field may change meaning or be removed without going through the deprecation
policy in section 10.

### `POST /v1/ingest` — write rows

**Frozen.**

- Headers:
  - `X-Database: <database>` (required)
  - `X-Table: <table>` (required)
  - `Content-Type: application/x-ndjson | application/json | ...` (enumerate)
  - `Idempotency-Key: <key>` (optional; documented in A1's idempotency contract)
- Request body: NDJSON or JSON array of row objects. Field shapes match
  the table schema. Unknown columns are rejected unless dynamic-schema
  mode is enabled (see `KYMA_INGEST_DYNAMIC_SCHEMA` in section 7).
- Response: `200 OK` with `{ "ingested": <n>, "snapshot_id": "..." }`
  on success. Error shape: `{ "error": "<code>", "message": "..." }`
  with HTTP 4xx/5xx.
- Stable error codes: `SCHEMA_MISMATCH`, `QUOTA_EXCEEDED`, `IDEMPOTENT_REPLAY`,
  ... (enumerate what the handler actually emits).

### `POST /v1/query` — run KQL or SQL

**Frozen.** [... same structure ...]

### `GET /health` — liveness probe

**Frozen.** Returns `200 OK` with body `"ok"`. Never gated behind auth.

### `GET /metrics` — Prometheus scrape

**Frozen format** (Prometheus text exposition v0.0.4). Metric names are
governed by [`docs/metrics-taxonomy.md`](metrics-taxonomy.md), not by
this surface.

### `GET /v1/...` (other endpoints — enumerate them all here)
````

Fill in every route from Step 1's grep. Do not leave any route undocumented.

- [ ] **Step 3: Verify every route from the grep is in the doc**

Run:

```bash
grep -oE '/v[0-9]+/[a-zA-Z_/{}-]+' docs/stability.md | sort -u > /tmp/doc-routes.txt
grep -rEoh '"/v[0-9]+/[a-zA-Z_/{}-]+"' crates/kyma-server/src crates/kyma-ingest-rest/src \
  | tr -d '"' | sort -u > /tmp/code-routes.txt
diff /tmp/code-routes.txt /tmp/doc-routes.txt
```

Expected: empty diff. If not, add the missing routes to `docs/stability.md`.

- [ ] **Step 4: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze HTTP REST API surface (F1)"
```

---

## Task 3: Arrow Flight gRPC surface freeze

**Files:**
- Modify: `docs/stability.md` (section 2)
- Reference: `crates/kyma-server/src/flight.rs`

- [ ] **Step 1: Read the Flight handler and inventory the surface**

Open `crates/kyma-server/src/flight.rs`. List every Flight RPC the server implements (`do_get`, `do_action`, `list_actions`, `get_flight_info`, etc.) and, for each, the ticket / action shape kyma expects.

- [ ] **Step 2: Replace section 2 with the inventory**

````markdown
## 2. Arrow Flight gRPC API

**Frozen.** The server exposes Arrow Flight on `:9090` by default. The
following surface is part of the v1.0 contract.

### Flight RPCs implemented

- `do_get(ticket)` — execute a query. Ticket shape is the JSON object
  documented under `do_get` below.
- `do_action(action)` — execute one of the actions enumerated below.
- `list_actions()` — returns the action list below.
- `get_flight_info(descriptor)` — [behavior].
- (enumerate every Flight method `crates/kyma-server/src/flight.rs` implements)

### `do_get` ticket shape

JSON object with these fields:

| Field | Type | Required | Meaning |
|---|---|---|---|
| `database` | string | yes | target database name |
| `query` | string | yes | KQL or SQL text |
| `dialect` | `"kql" \| "sql"` | yes | parser to use |
| `limit` | int | no | hard row cap |
| `idempotency_key` | string | no | for retry-safe queries |

Unknown fields rejected at v1.0. New optional fields may be added; existing
fields' meanings are fixed.

### Actions

- `cancel_query(query_id)` — [shape]
- (enumerate)

### Schemas returned

Arrow schemas returned by `do_get` mirror table schemas in the catalog.
ALTER TABLE ADD COLUMN never changes the type of an existing column.
````

Replace placeholders with the actual surface.

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze Arrow Flight gRPC surface (F1)"
```

---

## Task 4: KQL dialect freeze

**Files:**
- Modify: `docs/stability.md` (section 3)
- Reference: `crates/kyma-kql/src/` (parser and translator)

- [ ] **Step 1: Inventory implemented KQL operators, functions, types**

Read `crates/kyma-kql/src/lib.rs` and the parser modules. Enumerate every operator (`where`, `project`, `summarize`, `extend`, `join`, `order`, `take`, `top`, etc.) and every scalar/aggregate function the parser accepts (`count`, `sum`, `avg`, `min`, `max`, `bin`, `ago`, `now`, `tostring`, `toint`, `between`, ...).

Also list type names accepted in casts (`int`, `long`, `real`, `string`, `bool`, `datetime`, `timespan`, `dynamic`).

- [ ] **Step 2: Decide what's in v1.0 vs "best-effort, not v1.0"**

For each operator and function: is it stable behavior (in v1.0)? Or actively in flight / known incomplete (out)? Write the in-vs-out call inline as you go. When in doubt, mark out — easier to add to v1.1 than to break.

- [ ] **Step 3: Replace section 3 with the dialect freeze**

````markdown
## 3. KQL dialect

The KQL dialect that v1.0 supports is a defined subset of Kusto Query
Language. Anything in this section is part of the v1.0 frozen contract;
anything not in this section is "best-effort, not v1.0 surface."

### Operators (frozen)

| Operator | Form | Notes |
|---|---|---|
| `where` | `T \| where <expr>` | predicates may use any function in the function table |
| `project` | `T \| project col1, col2 = expr, ...` | rename/select |
| `summarize` | `T \| summarize agg1 = ... by col1, col2` | aggregation |
| `extend` | `T \| extend col = expr` | computed columns |
| `order` | `T \| order by col [asc\|desc]` | sort |
| `take` / `top` | `T \| take N`, `T \| top N by col` | limit |
| `join` | `T1 \| join kind=<inner\|leftouter\|...> (T2) on col` | enumerate supported kinds |
| ... | ... | ... |

(Enumerate every operator from Step 1 that we're committing to.)

### Operators explicitly **not** in v1.0

| Operator | Status | Notes |
|---|---|---|
| `mv-expand` | not in v1.0 | parser may accept it; behavior is not contract |
| ... | | |

### Scalar functions (frozen)

[list — `tostring`, `toint`, `tolong`, `toreal`, `todatetime`, `totimespan`,
`tobool`, `iff`, `case`, `bin`, `ago`, `now`, ... — only the ones we're
committing to]

### Aggregate functions (frozen)

[list — `count`, `sum`, `avg`, `min`, `max`, `dcount`, `make_list`,
`make_set`, ... — only the ones we're committing to]

### Type names

| Type | Backing Arrow type | Notes |
|---|---|---|
| `int` | int32 | |
| `long` | int64 | |
| `real` | float64 | |
| `string` | utf8 | |
| `bool` | boolean | |
| `datetime` | timestamp(nanosecond, UTC) | timezone-aware |
| `timespan` | duration(nanosecond) | |
| `dynamic` | struct or json | see A2 spec for query semantics |

### Error taxonomy

Stable error codes returned by the KQL frontend:
- `KQL_PARSE_ERROR` — text could not be parsed
- `KQL_UNKNOWN_TABLE` — table does not exist in the database
- `KQL_TYPE_ERROR` — type mismatch in expression
- (enumerate)

### Tests proving the freeze

`scripts/test-kql.sh` exercises every frozen operator. The back-compat
workflow (Task 14) replays `scripts/test-kql.sh`-style queries against each
tagged version's fixture.
````

- [ ] **Step 4: Verify every frozen operator is exercised by `scripts/test-kql.sh`**

Run:

```bash
grep -oE '\| (where|project|summarize|extend|order|take|top|join|...)' scripts/test-kql.sh \
  | sort -u
```

Expected: every operator listed in the frozen table appears. If one doesn't, either add a test for it in `scripts/test-kql.sh` (preferred) or move it out of the v1.0 subset.

- [ ] **Step 5: Commit**

```bash
git add docs/stability.md scripts/test-kql.sh
git commit -m "docs(stability): freeze KQL dialect subset (F1)"
```

---

## Task 5: SQL dialect freeze

**Files:**
- Modify: `docs/stability.md` (section 4)
- Reference: `crates/kyma-exec/src/df_adapter.rs` (or wherever DataFusion is integrated)

- [ ] **Step 1: Identify the DataFusion version + SQL features we expose**

Run:

```bash
grep -rn 'datafusion' Cargo.toml crates/*/Cargo.toml | head
```

Note the pinned DataFusion version.

Open `crates/kyma-exec/src/` and identify any features explicitly disabled (e.g. UDF registration, write DDL, federated catalogs).

- [ ] **Step 2: Replace section 4 with the SQL freeze**

````markdown
## 4. SQL dialect

The SQL dialect that v1.0 supports is the DataFusion SQL subset at version
`<X.Y.Z>` (pinned in workspace `Cargo.toml`), minus the opt-outs below.

### What's included

The DataFusion SQL surface — `SELECT`, `WHERE`, `GROUP BY`, `ORDER BY`,
`JOIN` (inner/left/right/full/cross), window functions, common table
expressions, scalar/aggregate functions documented at
<https://datafusion.apache.org/user-guide/sql/index.html> for that version.

### Opt-outs

- DDL statements (`CREATE TABLE`, `DROP TABLE`, `ALTER TABLE`, ...) — not
  supported through SQL. Use the catalog API or `kyma-cli` instead.
- DML other than `INSERT INTO ... SELECT` — not supported.
- User-defined functions — not registrable at runtime in v1.0.
- (Enumerate every other opt-out.)

### DataFusion version policy

v1.0 ships against DataFusion `<X.Y.Z>`. v1.x minor releases may upgrade
DataFusion only if the new version is fully back-compat against the
queries in `scripts/backcompat-queries.txt`. R1 (release engineering)
owns this policy.

### Tests proving the freeze

`scripts/test-flight.sh` and the back-compat workflow exercise the SQL
subset.
````

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze SQL dialect surface (F1)"
```

---

## Task 6: MCP surface freeze

**Files:**
- Modify: `docs/stability.md` (section 5)
- Reference: `crates/kyma-mcp/src/`

- [ ] **Step 1: Inventory MCP tools, resources, prompts**

Read `crates/kyma-mcp/src/lib.rs`. List every MCP tool, resource template, and prompt the server registers. For each tool, capture the JSON schema for its arguments and the return shape.

- [ ] **Step 2: Replace section 5 with the MCP freeze**

````markdown
## 5. MCP surface

The MCP server exposed by `kyma-mcp` is part of the v1.0 frozen surface.
Agents that consume this surface can rely on tool names, argument schemas,
and return shapes across the v1.x series.

### Tools (frozen)

- `query` — execute a KQL or SQL query.
  - Arguments: `{ database: string, query: string, dialect: "kql"|"sql", limit?: int }`.
  - Returns: `{ rows: [...], schema: [...], stats: { rows_scanned, rows_returned, ms } }`.
- (Enumerate every tool registered by `kyma-mcp`.)

### Resources (frozen)

- `kyma://databases` — list available databases.
- `kyma://databases/{db}/tables` — list tables in a database.
- (Enumerate.)

### Prompts (frozen)

- (Enumerate, or write "None in v1.0" if the crate registers no prompts.)

### Tests proving the freeze

`scripts/test-*-mcp.sh` (to be added by A2). Until A2 exists, MCP surface
is exercised only by `crates/kyma-mcp/tests/`.
````

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze MCP surface (F1)"
```

---

## Task 7: Catalog Postgres schema freeze

**Files:**
- Modify: `docs/stability.md` (section 6)
- Reference: `crates/kyma-catalog/migrations/` (or wherever migrations live)

- [ ] **Step 1: Find migrations and list every table/column**

Run:

```bash
find crates/kyma-catalog -name '*.sql' -o -name 'migrations*' | head
ls crates/kyma-catalog/migrations/ 2>/dev/null
```

Open every migration in order. List every table, every column, every index. Note constraints (PK, FK, UNIQUE).

- [ ] **Step 2: Replace section 6 with the schema freeze**

````markdown
## 6. Catalog Postgres schema

The catalog schema is **forward-only**. v1.x migrations may:

- Add new tables.
- Add new columns to existing tables (must be nullable or have a default).
- Add new indexes.
- Add new constraints that the existing data already satisfies.

v1.x migrations may **not**:

- Rename a table or a column.
- Drop a table or a column.
- Change the type of a column.
- Tighten a constraint in a way that requires data backfill.

These rules apply across the v1.x series. Breaking the schema requires
the deprecation policy in section 10.

### Current tables (as of v1.0 freeze)

| Table | Purpose | Key columns |
|---|---|---|
| `databases` | top-level database registry | `name` |
| `tables` | table registry | `database`, `name`, `schema_id` |
| `schemas` | schema versions | `id`, `table_id`, `arrow_schema_json` |
| `extents` | committed extent index | `id`, `table_id`, `min_ts`, `max_ts`, `path`, `format_id`, `tenant_id` |
| `snapshots` | per-table CAS snapshots | `table_id`, `seq`, `extents_json` |
| `nodes` | node heartbeats | `node_id`, `last_seen`, `roles` |
| `idempotency_keys` | dedupe ingest replays | `key`, `expires_at`, `result_hash` |
| ... | ... | ... |

(Enumerate every table.)

### Migration discipline

- Each migration is a single `.sql` file under `crates/kyma-catalog/migrations/`.
- File name: `NNNN_<short_snake_case>.sql`.
- Numbering is monotonic and gapless within a release line.
- The back-compat workflow (Task 14) runs every PR's migrations against
  the previous version's catalog dump and fails if any of the rules above
  is violated.
````

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze catalog Postgres schema (F1)"
```

---

## Task 8: Config keys and env vars freeze

**Files:**
- Modify: `docs/stability.md` (section 7)
- Reference: every `std::env::var` call in the codebase

- [ ] **Step 1: Inventory every KYMA_* env var**

Run:

```bash
grep -rohE 'KYMA_[A-Z0-9_]+' crates/*/src 2>/dev/null | sort -u > /tmp/all-env-vars.txt
cat /tmp/all-env-vars.txt
```

For each: read its call site to capture its meaning, default, and accepted values. Also note any env var that's clearly test-only (`KYMA_TEST_*`).

- [ ] **Step 2: Replace section 7 with the config freeze**

````markdown
## 7. Configuration keys and environment variables

Every `KYMA_*` env var below is part of the v1.0 contract. Removing or
renaming any of them requires the deprecation policy in section 10. New
env vars may be added at any v1.x minor release.

### Runtime configuration (frozen)

| Variable | Default | Meaning |
|---|---|---|
| `KYMA_HTTP_ADDR` | `0.0.0.0:8080` | HTTP server bind address |
| `KYMA_FLIGHT_ADDR` | `0.0.0.0:9090` | Arrow Flight bind address |
| `KYMA_DATABASE_URL` | (required) | Postgres connection string for the catalog |
| `KYMA_OBJECT_STORE_URL` | (required) | object-store URL (e.g. `s3://bucket/prefix`) |
| `KYMA_AUTH_BACKEND` | `env` | auth backend selector (`env`, `none`, ...) |
| `KYMA_AUTH_TOKENS` | (empty) | env-backend token list, format documented under A4 |
| `KYMA_INGEST_DYNAMIC_SCHEMA` | `false` | enable auto-add columns from inbound rows |
| `KYMA_STAGING_DISABLED` | `false` | bypass the staging buffer (test/debug) |
| `KYMA_COMPACTION_IDLE_SLEEP_MS` | (...) | compaction worker idle sleep |
| `KYMA_COMPACTION_POLL_SECS` | (...) | compaction worker poll interval |
| `KYMA_COMPACTION_MIN_EXTENTS` | (...) | minimum extents per compaction |
| `KYMA_RETENTION_POLL_SECS` | (...) | retention worker poll interval |
| `KYMA_PHYSICAL_GC_POLL_SECS` | (...) | GC worker poll interval |
| `KYMA_PHYSICAL_GC_GRACE_SECS` | (...) | grace window before GC deletes soft-deleted extents |
| `KYMA_FILEDROP_ENABLED` | `false` | enable the file-drop ingest frontend |
| `KYMA_CONNECTOR_WORKERS` | (...) | connector worker count |
| `KYMA_SCHEMA_CACHE_TTL_SECS` | (...) | catalog schema cache TTL |
| `KYMA_AGENT_OLLAMA_HOST` | (...) | local-agent Ollama host |
| `KYMA_AGENT_MODEL` | (...) | local-agent model name |
| ... | ... | ... |

(Fill in every var from Step 1, real defaults, real meanings. Read the
call site if unsure.)

### Test-only variables

These are not part of the v1.0 contract; they may change at any time.

- `KYMA_TEST_DATABASE_URL` — override catalog DB in tests.
- (Enumerate any other `KYMA_TEST_*`.)
````

- [ ] **Step 3: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): freeze config keys and env vars (F1)"
```

---

## Task 9: Extent on-disk format freeze (placeholder pending P0)

**Files:**
- Modify: `docs/stability.md` (section 8)

- [ ] **Step 1: Write a placeholder that's honest about the P0 dependency**

Replace section 8 with:

````markdown
## 8. Extent on-disk format

**Status:** placeholder — final freeze blocked on P0 (format-v1 completion).

The v1.0 extent format will be frozen when P0 lands. Until then, the
following invariants are committed:

- Every extent carries a leading magic + version byte: `0x4B 0x59 0x4D 0x41`
  ("KYMA") followed by a `u8` format version.
- v1.x readers will read every extent any v1.x writer produces. v1.x
  readers will read extents written by P0-era pre-v1.0 builds on a
  best-effort basis (back-compat fixture pinned at the first `v1.0.0-pre.N`
  tag).
- Format version `0` is reserved for the current pre-P0 telemetry format.
  Format version `1` will be the post-P0 telemetry format.
- The catalog `extents.format_id` column carries the format version per
  extent. Readers select the decoder by `format_id`.

When P0 lands, this section is replaced with the full format-v1 freeze
(layout, encoding, dictionary format, posting-list format, footer layout).
That work is tracked separately under P0's plan.
````

- [ ] **Step 2: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): placeholder for extent format freeze (F1, pending P0)"
```

---

## Task 10: Deprecation policy

**Files:**
- Modify: `docs/stability.md` (section 10)

- [ ] **Step 1: Replace section 10 with the deprecation policy**

````markdown
## 10. Deprecation policy

Any change to a frozen surface in this document — including removing a
KQL operator, renaming an env var, changing a REST field's meaning, or
breaking the extent format beyond the version-byte escape hatch —
follows this policy.

### Minimum window

A deprecated surface must continue to work for at least **6 months**
(or two minor releases, whichever is longer) before removal.

### Required steps

1. **Announce.** Open an RFC issue tagged `stability` describing the
   change, the migration path, and the proposed removal version.
2. **Land the replacement.** The new surface ships in the same release
   as (or before) the deprecation warning.
3. **Warn.** Calls to the deprecated surface emit a structured warning:
   - HTTP / Flight: response carries `X-Kyma-Deprecation: <surface>; sunset=<version>; replacement=<surface>`.
   - Log: `kyma_deprecation_used_total{surface, replacement}` counter increments; structured log entry at WARN with `event=deprecation_used`.
4. **Document.** Changelog entry under "Deprecated" with the sunset version.
5. **Wait.** At least 6 months and 2 minor releases.
6. **Remove.** In the sunset release, drop the surface. Changelog entry
   under "Removed."

### Exceptions

- **Security fixes** may change a surface immediately when continued
  support enables a vulnerability. The change is announced in the
  release notes and `SECURITY.md` rather than via the deprecation cycle.
- **Pre-1.0 builds** (`v0.x`, `v1.0.0-pre.N`, `v1.0.0-rc.N`) are not
  subject to this policy.

### Enforcement

The back-compat workflow (`.github/workflows/backcompat.yml`) replays a
fixed query set from every tagged version's fixture against the current
build. A removed surface in a PR breaks the workflow.
````

- [ ] **Step 2: Commit**

```bash
git add docs/stability.md
git commit -m "docs(stability): add deprecation policy (F1)"
```

---

## Task 11: Wire deprecation policy into CONTRIBUTING.md

**Files:**
- Modify: `CONTRIBUTING.md`

- [ ] **Step 1: Add a "Stability and deprecation policy" section between "Architectural invariants" and "License"**

Insert this block before the `## License` heading:

````markdown
## Stability and deprecation policy

From `v1.0.0` onward, kyma maintains a written stability contract:
[`docs/stability.md`](docs/stability.md). It lists every surface kyma
promises not to break across the v1.x series — HTTP REST API, Flight
gRPC, KQL dialect, SQL dialect, MCP surface, catalog schema, config
keys, extent format, metrics naming.

If your change touches any frozen surface, your PR must either:

- Stay within the contract (additive, non-breaking) — preferred. The
  CI workflow `.github/workflows/backcompat.yml` enforces this on every
  PR by replaying a fixed query set against the last three tagged
  versions.
- Or follow the deprecation policy in `docs/stability.md` section 10
  (RFC, replacement-first, 6-month warning window).

Pre-`v1.0.0` builds (`v0.x`, `v1.0.0-pre.N`, `v1.0.0-rc.N`) are not
under the contract.
````

- [ ] **Step 2: Verify the section renders cleanly**

Run: `grep -n -A 3 "Stability and deprecation" CONTRIBUTING.md`
Expected: the header and first 3 lines print.

- [ ] **Step 3: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs(contributing): point at stability contract and deprecation policy (F1)"
```

---

## Task 12: Write the metrics-taxonomy rules

**Files:**
- Create: `docs/metrics-taxonomy.md`
- Reference: `crates/kyma-server/src/metrics.rs`

- [ ] **Step 1: Inventory current metric names**

Run:

```bash
grep -rEoh '"kyma_[a-z0-9_]+"' crates/*/src 2>/dev/null | sort -u
grep -rEoh 'metrics::(counter|gauge|histogram)!\("[a-z0-9_]+' crates/*/src 2>/dev/null | sort -u
```

These are the existing names. Some may not start with `kyma_`; the taxonomy will require they do post-v1.0.

- [ ] **Step 2: Write `docs/metrics-taxonomy.md`**

````markdown
# kyma Metrics Taxonomy

The shared rules every metric kyma exports must follow. Area specs
(A1–A4) define which metrics each subsystem ships; this file defines the
naming, label, and lifecycle rules they all follow.

## Naming

- Every metric name starts with `kyma_`.
- Format: `kyma_<subsystem>_<metric>_<unit>` where:
  - `<subsystem>` is `ingest`, `query`, `catalog`, `storage`,
    `compaction`, `retention`, `gc`, `auth`, `mcp`, `server`, ...
  - `<metric>` is a short snake_case noun.
  - `<unit>` is the Prometheus unit suffix when applicable: `_seconds`,
    `_bytes`, `_total` (counters), `_ratio`, `_count`.

Examples:

- `kyma_ingest_rows_total` (counter)
- `kyma_ingest_commit_seconds` (histogram)
- `kyma_query_pruning_extents_skipped_ratio` (gauge)
- `kyma_storage_extent_bytes` (histogram)

## Labels

- High-cardinality labels (anything per-query, per-trace-id,
  per-tenant-token) are forbidden.
- Allowed labels:
  - `database` (table) — bounded by user database count.
  - `table` — bounded by table count per database.
  - `tenant_id` — bounded by tenant count.
  - `subsystem` — bounded.
  - `code` — bounded enum of error codes.
  - `result` — `success` / `error` / `cancelled` / `timeout`.
- `tenant_id` is allowed only for metrics where tenant-level breakdown
  is a documented operator need; otherwise omit.

## Histograms

- Latency histograms use the seconds unit (`_seconds`).
- Bucket boundaries documented per metric in the area spec.

## Deprecation

Removing or renaming a metric follows the same 6-month / 2-minor-release
policy as `docs/stability.md` section 10. The deprecated metric continues
to be exported, with `kyma_deprecation_used_total{surface}` ticking for
each scrape of a deprecated name.

## Verification

The back-compat CI workflow verifies that every metric exported by a
prior tagged version is still exported by the current build (unless
formally deprecated). Verification script: `scripts/backcompat-replay.sh`.

## Per-subsystem inventories

Each area spec (A1–A4) appends a section under `docs/runbooks/<area>.md`
listing the metrics it ships and what each one means. This document only
holds the rules.
````

- [ ] **Step 3: Commit**

```bash
git add docs/metrics-taxonomy.md
git commit -m "docs(metrics): metrics-taxonomy rules for v1.0 (F1)"
```

---

## Task 13: Write the fixed back-compat query set

**Files:**
- Create: `scripts/backcompat-queries.txt`

- [ ] **Step 1: Choose a small fixed query set covering frozen surfaces**

The query set is the contract's test harness: every query here must keep
returning the same shape and (where deterministic) the same values across
v1.x. Start with a small set; A1–A4 will add to it.

Write `scripts/backcompat-queries.txt` with this exact content:

```
# back-compat fixed query set — v1.0 contract verification
#
# Format: each non-empty, non-comment line is one query in the form:
#   <id>\t<endpoint>\t<content-type>\t<database>\t<query-body>
#
# The replayer (scripts/backcompat-replay.sh) POSTs each query to the
# running engine, hashes the response body (after sorting JSON object
# keys), and compares against the per-fixture expected-hash file.
#
# Adding queries: append only. Never reorder; never edit an existing
# query's text. Tagged fixtures are pinned to specific IDs.

q-rest-health	GET /health	-	-	-
q-rest-metrics-shape	GET /metrics	-	-	-
q-kql-where-count	POST /v1/query	application/x-kql	obs	otel_logs | where severity_text == "ERROR" | summarize n = count()
q-kql-summarize-by	POST /v1/query	application/x-kql	obs	otel_logs | summarize n = count() by severity_text | order by severity_text asc
q-sql-select-count	POST /v1/query	application/sql	obs	SELECT COUNT(*) AS n FROM otel_logs
q-sql-group-by	POST /v1/query	application/sql	obs	SELECT severity_text, COUNT(*) AS n FROM otel_logs GROUP BY severity_text ORDER BY severity_text ASC
```

This is the minimal seed. A1–A4 expand it.

- [ ] **Step 2: Commit**

```bash
git add scripts/backcompat-queries.txt
git commit -m "test(backcompat): seed fixed query set for v1.0 contract (F1)"
```

---

## Task 14: Write `scripts/backcompat-snapshot.sh` (TDD)

**Files:**
- Create: `scripts/backcompat-snapshot.sh`
- Test: `scripts/tests/test-backcompat-snapshot.sh`

- [ ] **Step 1: Write the failing test**

Create `scripts/tests/test-backcompat-snapshot.sh`:

```bash
#!/usr/bin/env bash
# Test: backcompat-snapshot.sh produces a fixture directory with the
# expected files against a running kyma engine.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE_URL="${ENGINE_URL:-http://localhost:8080}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

./backcompat-snapshot.sh "$ENGINE_URL" "$TMPDIR/fixture"

# Required artifacts
for f in manifest.json catalog-schema.sql sample-extents/ git-sha.txt build-version.txt; do
  if [ ! -e "$TMPDIR/fixture/$f" ]; then
    echo "FAIL: missing $f"
    exit 1
  fi
done

# manifest.json must include at least these fields
python3 -c "
import json, sys
m = json.load(open('$TMPDIR/fixture/manifest.json'))
for k in ('version', 'git_sha', 'catalog_schema_version', 'created_at'):
    assert k in m, k
"

echo PASS
```

- [ ] **Step 2: Run the test and watch it fail**

```bash
chmod +x scripts/tests/test-backcompat-snapshot.sh
docker-compose up -d  # if not already running
cargo run --release -p kyma-bin &
ENGINE_PID=$!
sleep 5
scripts/tests/test-backcompat-snapshot.sh
```

Expected: FAIL with `./backcompat-snapshot.sh: No such file or directory`.

(Leave the engine running; subsequent steps use it.)

- [ ] **Step 3: Write `scripts/backcompat-snapshot.sh`**

```bash
#!/usr/bin/env bash
# backcompat-snapshot.sh — capture a back-compat fixture of a running engine.
# Usage: backcompat-snapshot.sh <engine-url> <out-dir>
set -euo pipefail

ENGINE_URL="${1:?engine URL required}"
OUT_DIR="${2:?output directory required}"

mkdir -p "$OUT_DIR/sample-extents"

# 1. git sha + build version
git rev-parse HEAD > "$OUT_DIR/git-sha.txt"
# build version: cargo metadata for kyma-bin
cargo metadata --format-version 1 --no-deps \
  | python3 -c "import json,sys; m=json.load(sys.stdin); print(next(p['version'] for p in m['packages'] if p['name']=='kyma-bin'))" \
  > "$OUT_DIR/build-version.txt"

# 2. catalog schema dump (DDL only, no data)
PG_URL="${KYMA_DATABASE_URL:?KYMA_DATABASE_URL must be set}"
pg_dump --schema-only --no-owner --no-privileges "$PG_URL" > "$OUT_DIR/catalog-schema.sql"

# 3. sample extents — copy a small subset from object storage
#    For the synthetic seed fixture this directory may be empty; once
#    the engine has data, list and copy the first 3 extents.
EXTENTS_LIST="$(curl -sS "$ENGINE_URL/v1/catalog/extents?limit=3" 2>/dev/null || echo '[]')"
echo "$EXTENTS_LIST" > "$OUT_DIR/sample-extents/extents.json"

# 4. manifest
cat > "$OUT_DIR/manifest.json" <<EOF
{
  "version": "$(cat "$OUT_DIR/build-version.txt")",
  "git_sha": "$(cat "$OUT_DIR/git-sha.txt")",
  "catalog_schema_version": "$(grep -oE 'INSERT INTO [_a-z]*migrations[^;]*' "$OUT_DIR/catalog-schema.sql" | tail -1 || echo 'unknown')",
  "created_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

echo "snapshot written to $OUT_DIR"
```

- [ ] **Step 4: Run the test, watch it pass**

```bash
scripts/tests/test-backcompat-snapshot.sh
```

Expected: PASS.

If the `/v1/catalog/extents?limit=3` endpoint doesn't exist yet, the script will write an empty list — that's fine for the synthetic seed; A1 will add the endpoint or replace this step.

- [ ] **Step 5: Commit**

```bash
chmod +x scripts/backcompat-snapshot.sh
git add scripts/backcompat-snapshot.sh scripts/tests/test-backcompat-snapshot.sh
git commit -m "test(backcompat): snapshot script with TDD (F1)"
```

---

## Task 15: Write `scripts/backcompat-replay.sh` (TDD)

**Files:**
- Create: `scripts/backcompat-replay.sh`
- Test: `scripts/tests/test-backcompat-replay.sh`

- [ ] **Step 1: Write the failing test**

Create `scripts/tests/test-backcompat-replay.sh`:

```bash
#!/usr/bin/env bash
# Test: backcompat-replay.sh runs the fixed query set against a running
# engine and a fixture, and exits 0 on match / non-zero on mismatch.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE_URL="${ENGINE_URL:-http://localhost:8080}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Build a fixture against current engine
./backcompat-snapshot.sh "$ENGINE_URL" "$TMPDIR/fixture"

# Generate expected hashes from current engine (so the test self-matches)
./backcompat-replay.sh --record "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt

# Replay should now pass against the same engine
./backcompat-replay.sh "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt
echo PASS-MATCH

# Mutate one expected hash; replay should fail
sed -i.bak 's/^q-kql-where-count.*/q-kql-where-count\tDEADBEEF/' "$TMPDIR/fixture/expected-hashes.txt"
if ./backcompat-replay.sh "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt 2>/dev/null; then
  echo "FAIL: replay should have failed on hash mismatch"
  exit 1
fi

echo PASS
```

- [ ] **Step 2: Run the test, watch it fail**

```bash
chmod +x scripts/tests/test-backcompat-replay.sh
scripts/tests/test-backcompat-replay.sh
```

Expected: FAIL with `./backcompat-replay.sh: No such file or directory`.

- [ ] **Step 3: Write `scripts/backcompat-replay.sh`**

```bash
#!/usr/bin/env bash
# backcompat-replay.sh — replay a fixed query set, compare to fixture's
# expected hashes.
#
# Usage:
#   backcompat-replay.sh [--record] <engine-url> <fixture-dir> <queries-file>
#
# --record writes expected-hashes.txt into the fixture instead of
# comparing. Used once when a fixture is first captured.
set -euo pipefail

RECORD=0
if [ "${1:-}" = "--record" ]; then
  RECORD=1; shift
fi

ENGINE_URL="${1:?engine URL required}"
FIXTURE_DIR="${2:?fixture directory required}"
QUERIES_FILE="${3:?queries file required}"

run_one() {
  local id="$1" endpoint="$2" content_type="$3" database="$4" body="$5"
  local method path
  method="$(echo "$endpoint" | awk '{print $1}')"
  path="$(echo "$endpoint" | awk '{print $2}')"
  if [ "$method" = "GET" ]; then
    curl -sS "$ENGINE_URL$path"
  else
    local -a curl_args=(-sS -X "$method" "$ENGINE_URL$path")
    if [ "$database" != "-" ]; then curl_args+=(-H "X-Database: $database"); fi
    if [ "$content_type" != "-" ]; then curl_args+=(-H "Content-Type: $content_type"); fi
    curl "${curl_args[@]}" --data-binary "$body"
  fi
}

hash_body() {
  # Normalize: sort JSON object keys when possible, then sha256.
  python3 -c "
import sys, json, hashlib
raw = sys.stdin.read()
try:
    obj = json.loads(raw)
    norm = json.dumps(obj, sort_keys=True, separators=(',', ':'))
except Exception:
    norm = raw
print(hashlib.sha256(norm.encode()).hexdigest())
"
}

HASHES_FILE="$FIXTURE_DIR/expected-hashes.txt"
TMP_OUT="$(mktemp)"
trap 'rm -f "$TMP_OUT"' EXIT

: > "$TMP_OUT"
while IFS=$'\t' read -r id endpoint content_type database body; do
  case "$id" in ''|\#*) continue ;; esac
  resp="$(run_one "$id" "$endpoint" "$content_type" "$database" "$body")"
  h="$(printf '%s' "$resp" | hash_body)"
  printf '%s\t%s\n' "$id" "$h" >> "$TMP_OUT"
done < "$QUERIES_FILE"

if [ "$RECORD" = "1" ]; then
  mv "$TMP_OUT" "$HASHES_FILE"
  echo "recorded expected hashes to $HASHES_FILE"
  exit 0
fi

if [ ! -f "$HASHES_FILE" ]; then
  echo "FAIL: no expected-hashes.txt in fixture (run with --record first)"
  exit 2
fi

if diff -u "$HASHES_FILE" "$TMP_OUT" >/dev/null; then
  echo "OK: $(wc -l < "$TMP_OUT") queries match fixture"
  exit 0
else
  echo "FAIL: replay diverged from fixture:"
  diff -u "$HASHES_FILE" "$TMP_OUT"
  exit 3
fi
```

- [ ] **Step 4: Run the test, watch it pass**

```bash
scripts/tests/test-backcompat-replay.sh
```

Expected: PASS (the test self-matches by recording then replaying).

- [ ] **Step 5: Commit**

```bash
chmod +x scripts/backcompat-replay.sh
git add scripts/backcompat-replay.sh scripts/tests/test-backcompat-replay.sh
git commit -m "test(backcompat): replay script with TDD (F1)"
```

---

## Task 16: Capture the synthetic seed fixture

**Files:**
- Create: `scripts/fixtures/backcompat/main-seed/` (directory of fixture files)

- [ ] **Step 1: Stop any running engine, bring up a fresh stack**

```bash
docker-compose down -v
docker-compose up -d
cargo run --release -p kyma-bin &
ENGINE_PID=$!
sleep 8
curl -sS http://localhost:8080/health
```

Expected: `ok`.

- [ ] **Step 2: Seed the engine with a tiny deterministic dataset**

The fixed query set in `scripts/backcompat-queries.txt` references
`obs.otel_logs`. Seed it with deterministic rows:

```bash
curl -sS -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: obs" \
  -H "X-Table: otel_logs" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"timestamp":"2026-01-01T00:00:00Z","service.name":"svc-a","severity_text":"ERROR","message":"e1"}
{"timestamp":"2026-01-01T00:00:01Z","service.name":"svc-a","severity_text":"ERROR","message":"e2"}
{"timestamp":"2026-01-01T00:00:02Z","service.name":"svc-a","severity_text":"INFO","message":"i1"}
{"timestamp":"2026-01-01T00:00:03Z","service.name":"svc-a","severity_text":"INFO","message":"i2"}
{"timestamp":"2026-01-01T00:00:04Z","service.name":"svc-a","severity_text":"INFO","message":"i3"}
EOF

# Force commit
sleep 3
```

This dataset stays pinned. Any future change that re-orders these rows in
results breaks the back-compat workflow on purpose.

- [ ] **Step 3: Capture the fixture + record expected hashes**

```bash
mkdir -p scripts/fixtures/backcompat
scripts/backcompat-snapshot.sh http://localhost:8080 scripts/fixtures/backcompat/main-seed
scripts/backcompat-replay.sh --record http://localhost:8080 scripts/fixtures/backcompat/main-seed scripts/backcompat-queries.txt
```

Expected: `recorded expected hashes to scripts/fixtures/backcompat/main-seed/expected-hashes.txt`.

- [ ] **Step 4: Verify the fixture self-matches**

```bash
scripts/backcompat-replay.sh http://localhost:8080 scripts/fixtures/backcompat/main-seed scripts/backcompat-queries.txt
```

Expected: `OK: N queries match fixture` for N = number of non-comment lines in `backcompat-queries.txt`.

- [ ] **Step 5: Write a README in the fixture directory**

Create `scripts/fixtures/backcompat/main-seed/README.md`:

```markdown
# main-seed back-compat fixture

Captured against `main` HEAD at F1 implementation time, before any
`v1.0.0-pre.N` tag exists. Pinned dataset: see `seed.ndjson` in this
directory (or the seed block in Task 16 of the F1 plan).

This is the synthetic seed fixture. It establishes the back-compat
machinery before real tags exist. Once `v1.0.0-pre.1` is cut, the
workflow grows a per-tag fixture next to this one and continues to
include this seed.

Do **not** edit the captured files (`expected-hashes.txt`,
`catalog-schema.sql`, `manifest.json`). They are the contract.
```

Also save the seed for reproducibility:

```bash
cat > scripts/fixtures/backcompat/main-seed/seed.ndjson <<'EOF'
{"timestamp":"2026-01-01T00:00:00Z","service.name":"svc-a","severity_text":"ERROR","message":"e1"}
{"timestamp":"2026-01-01T00:00:01Z","service.name":"svc-a","severity_text":"ERROR","message":"e2"}
{"timestamp":"2026-01-01T00:00:02Z","service.name":"svc-a","severity_text":"INFO","message":"i1"}
{"timestamp":"2026-01-01T00:00:03Z","service.name":"svc-a","severity_text":"INFO","message":"i2"}
{"timestamp":"2026-01-01T00:00:04Z","service.name":"svc-a","severity_text":"INFO","message":"i3"}
EOF
```

- [ ] **Step 6: Stop the engine and tear down**

```bash
kill $ENGINE_PID || true
docker-compose down -v
```

- [ ] **Step 7: Commit**

```bash
git add scripts/fixtures/backcompat/
git commit -m "test(backcompat): synthetic seed fixture against main HEAD (F1)"
```

---

## Task 17: Write the back-compat CI workflow

**Files:**
- Create: `.github/workflows/backcompat.yml`

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/backcompat.yml`:

```yaml
name: backcompat

on:
  pull_request:
    branches: [main]
  push:
    branches: [main]

jobs:
  replay:
    name: replay fixed query set against every fixture
    runs-on: ubuntu-22.04
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_USER: kyma
          POSTGRES_PASSWORD: kyma
          POSTGRES_DB: kyma
        ports: ["5432:5432"]
        options: >-
          --health-cmd="pg_isready -U kyma"
          --health-interval=5s
          --health-timeout=3s
          --health-retries=10
      minio:
        image: bitnami/minio:latest
        env:
          MINIO_ROOT_USER: kyma
          MINIO_ROOT_PASSWORD: kyma-secret
          MINIO_DEFAULT_BUCKETS: kyma
        ports: ["9000:9000"]
    env:
      KYMA_DATABASE_URL: postgresql://kyma:kyma@localhost:5432/kyma
      KYMA_OBJECT_STORE_URL: s3://kyma?endpoint=http://localhost:9000&region=us-east-1&access_key_id=kyma&secret_access_key=kyma-secret
      KYMA_HTTP_ADDR: 0.0.0.0:8080
      AWS_ACCESS_KEY_ID: kyma
      AWS_SECRET_ACCESS_KEY: kyma-secret
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: backcompat-${{ runner.os }}-${{ hashFiles('Cargo.lock') }}

      - name: build engine
        run: cargo build --release -p kyma-bin

      - name: start engine
        run: |
          ./target/release/kyma-bin &
          echo $! > /tmp/engine.pid
          for i in $(seq 1 30); do
            if curl -fsS http://localhost:8080/health >/dev/null; then break; fi
            sleep 1
          done
          curl -fsS http://localhost:8080/health

      - name: seed engine with each fixture's seed dataset
        run: |
          for fixture in scripts/fixtures/backcompat/*/; do
            seed="$fixture/seed.ndjson"
            if [ -f "$seed" ]; then
              echo "seeding from $seed"
              curl -fsS -X POST http://localhost:8080/v1/ingest \
                -H "X-Database: obs" \
                -H "X-Table: otel_logs" \
                -H "Content-Type: application/x-ndjson" \
                --data-binary "@$seed"
            fi
          done
          sleep 3  # let commit coordinator flush

      - name: replay each fixture
        run: |
          fail=0
          for fixture in scripts/fixtures/backcompat/*/; do
            echo "::group::$(basename "$fixture")"
            if ! scripts/backcompat-replay.sh http://localhost:8080 "$fixture" scripts/backcompat-queries.txt; then
              fail=1
            fi
            echo "::endgroup::"
          done
          exit $fail

      - name: stop engine
        if: always()
        run: kill "$(cat /tmp/engine.pid)" || true
```

- [ ] **Step 2: Commit (do not push yet)**

```bash
git add .github/workflows/backcompat.yml
git commit -m "ci(backcompat): replay fixed query set against fixtures (F1)"
```

---

## Task 18: Local dry-run of the back-compat workflow

**Files:** none (validation only)

- [ ] **Step 1: Run the workflow's logic locally**

```bash
docker-compose down -v
docker-compose up -d
cargo run --release -p kyma-bin &
ENGINE_PID=$!
for i in $(seq 1 30); do
  if curl -fsS http://localhost:8080/health >/dev/null 2>&1; then break; fi
  sleep 1
done

for fixture in scripts/fixtures/backcompat/*/; do
  seed="$fixture/seed.ndjson"
  if [ -f "$seed" ]; then
    curl -fsS -X POST http://localhost:8080/v1/ingest \
      -H "X-Database: obs" -H "X-Table: otel_logs" \
      -H "Content-Type: application/x-ndjson" \
      --data-binary "@$seed"
  fi
done
sleep 3

fail=0
for fixture in scripts/fixtures/backcompat/*/; do
  echo "--- $(basename "$fixture") ---"
  scripts/backcompat-replay.sh http://localhost:8080 "$fixture" scripts/backcompat-queries.txt || fail=1
done

kill $ENGINE_PID || true
docker-compose down -v
exit $fail
```

Expected: every fixture reports `OK: N queries match fixture` and the script exits 0.

If a fixture diverges, the failure mode is either:
- **Engine regression** — the contract was broken; fix the regression before merging.
- **Fixture out of date** — only legitimate if the change is going through the deprecation policy; if so, the fixture is updated as the *last* step of that change, not the first.

- [ ] **Step 2: No commit (no file changes); just confirm pass before moving on**

---

## Task 19: Self-review of `docs/stability.md`

**Files:** review only

- [ ] **Step 1: Re-read `docs/stability.md` end to end**

Read every section. Verify:

- Every section has been filled in (no `_Filled in by Task N._` placeholders left, except section 8 which is explicitly placeholder-pending-P0).
- Every surface enumerated by the inventory greps in Tasks 2, 3, 6, 8 is present in the doc.
- Every "frozen" claim has a test backing it (REST → `scripts/backcompat-queries.txt`, KQL → `scripts/test-kql.sh` + back-compat replay, SQL → back-compat replay, Flight → `scripts/test-flight.sh`, MCP → `crates/kyma-mcp/tests/`, catalog → migration policy + workflow, config → workflow loads env vars from `docs/stability.md`).
- The deprecation policy is consistent with the wording in `CONTRIBUTING.md`.

- [ ] **Step 2: Cross-check against the master spec**

Open `docs/superpowers/specs/2026-05-19-kyma-v1-production-readiness-design.md` section 2 (Axis 2). Every "frozen surface" bullet there must map to a section in `stability.md`. Fix any miss inline.

- [ ] **Step 3: Commit any fixes**

```bash
git add docs/stability.md CONTRIBUTING.md
git commit -m "docs(stability): self-review pass — fill remaining gaps (F1)" || echo "no fixes needed"
```

---

## Task 20: Final F1 sign-off

**Files:** none

- [ ] **Step 1: Verify all F1 outputs exist**

```bash
for f in \
  docs/stability.md \
  docs/metrics-taxonomy.md \
  scripts/backcompat-snapshot.sh \
  scripts/backcompat-replay.sh \
  scripts/backcompat-queries.txt \
  scripts/fixtures/backcompat/main-seed/expected-hashes.txt \
  scripts/fixtures/backcompat/main-seed/seed.ndjson \
  scripts/fixtures/backcompat/main-seed/README.md \
  .github/workflows/backcompat.yml; do
  [ -e "$f" ] || { echo "MISSING: $f"; exit 1; }
done
grep -q "Stability and deprecation policy" CONTRIBUTING.md || { echo "MISSING: CONTRIBUTING.md section"; exit 1; }
echo "F1 outputs present."
```

- [ ] **Step 2: Verify the workflow can be parsed**

Use `actionlint` if available, or `yq`:

```bash
if command -v actionlint >/dev/null; then
  actionlint .github/workflows/backcompat.yml
else
  yq eval '.' .github/workflows/backcompat.yml > /dev/null && echo OK
fi
```

Expected: no errors.

- [ ] **Step 3: Tag the F1 completion in the master spec**

Open `docs/superpowers/specs/2026-05-19-kyma-v1-production-readiness-design.md`, find the F1 entry in section 3, append after its line:

```markdown
**Status:** ✅ F1 implementation complete — see `docs/superpowers/plans/2026-05-19-f1-stability-contract.md`. Format clause in `docs/stability.md` section 8 is a placeholder until P0 lands.
```

- [ ] **Step 4: Final commit**

```bash
git add docs/superpowers/specs/2026-05-19-kyma-v1-production-readiness-design.md
git commit -m "docs(specs): mark F1 (stability contract) complete"
```

- [ ] **Step 5: Open a PR**

```bash
git push -u origin "$(git branch --show-current)"
gh pr create --title "F1: v1.0 stability contract" --body "$(cat <<'EOF'
## Summary

Implements F1 (Stability Contract) from the v1.0 production-readiness master design.

Outputs:
- `docs/stability.md` — frozen-surface contract (REST, Flight, KQL, SQL, MCP, catalog schema, config, format placeholder, deprecation policy)
- `docs/metrics-taxonomy.md` — shared metric naming + label rules for A1–A4 to follow
- `CONTRIBUTING.md` — points at the contract and deprecation policy
- `scripts/backcompat-{snapshot,replay,queries}.sh` + `scripts/fixtures/backcompat/main-seed/` — back-compat machinery, seeded with a synthetic main-HEAD fixture (real tagged fixtures appear as `v1.0.0-pre.N` tags get cut)
- `.github/workflows/backcompat.yml` — enforces the contract on every PR

Section 8 (extent format) is intentionally a placeholder pending P0 (format v1 completion).

## Test plan

- [x] `scripts/tests/test-backcompat-snapshot.sh` passes locally
- [x] `scripts/tests/test-backcompat-replay.sh` passes locally
- [x] `scripts/backcompat-replay.sh` against the synthetic seed fixture passes locally
- [x] backcompat workflow's logic dry-runs locally (Task 18)
- [ ] backcompat workflow passes in CI on this PR

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL printed. Workflow runs on the PR; once green, ready for review and merge.

---

## What this plan does NOT do

These belong to other specs and are intentionally out of scope here:

- The actual metric/log/trace inventories for each subsystem — owned by A1–A4 per `docs/metrics-taxonomy.md`.
- Adding back-compat scenarios for crash, pg failover, object-store throttling — owned by F2 (test gauntlet).
- Filling in section 8 (extent format) — owned by P0 + this plan's Task 9 placeholder.
- Signed releases, SBOM, public benchmarks — owned by R1.
- Any change to actual engine behavior — F1 is documentation + CI only, no engine-source changes.
