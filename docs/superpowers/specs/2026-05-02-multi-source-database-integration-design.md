# Multi-Source Database Integration — Design

**Status:** draft for review
**Date:** 2026-05-02
**Owner:** kyma core
**Related:** [`2026-04-20-connectors-design.md`](2026-04-20-connectors-design.md), [`2026-04-21-nl-query-agent-and-vectors-design.md`](2026-04-21-nl-query-agent-and-vectors-design.md)

---

## 1. Summary

kyma currently ingests telemetry (OTLP / REST / Kafka / file-drop) and queries it via KQL/SQL/PromQL over Arrow. This design adds **deep, first-class integration with external operational databases** — Postgres, MySQL, MongoDB — so agents and users can query *all* of their production data through one engine, with cross-source joins, at native source speed for live data and at kyma speed for historical/heavy data.

Two integration modes, exposed as a single configuration:

- **Federation** — register an external database as a DataFusion catalog. Queries push filters/projections/limits/aggregations down to the source over its native protocol; cross-source joins happen in DataFusion. No data is copied. Fresh, low-latency, no storage cost.
- **Sync** — replay the source's change log (Postgres logical replication, MySQL binlog, Mongo change streams) into kyma extents with an exactly-once, snapshot-consistent guarantee. Subsequent queries hit kyma's existing pruning cascade. Bounded lag, native kyma speed, full historical replay.

Both modes coexist on the same source instance (`mode = "both"`), share one connection pool, share schema introspection, and share status visibility. By default a synced table is queried from the synced extents; a SQL function `live(table)` opts into the federated path for that query.

The spec covers all three engines so the abstractions are validated against three impls. Implementation ships in vertical slices: Postgres → MySQL → Mongo.

---

## 2. Goals & non-goals

### Goals

- Make Postgres / MySQL / Mongo first-class data sources in kyma, queryable in KQL and SQL alongside kyma's native tables.
- Cross-source joins (kyma + external) and same-source joins (entirely within one external source).
- Best-practice federation pushdown: filters, projection, `LIMIT`, `ORDER BY`, single-source aggregation, opportunistic same-source join — never silently producing wrong results, with property tests gating every pushdown rule.
- Exactly-once, snapshot-consistent CDC sync with transactional cursor commits — matching the industry standard set by Debezium / Fivetran / Airbyte.
- Schema inference + schema evolution for nested/document data (Mongo, JSONB) with a typed-columns + `dynamic` overflow model.
- One connector config per source instance, with `mode = federation | sync | both`.
- TLS-required-by-default, secrets-by-reference (never plaintext in catalog), per-source connection pooling.
- A queryable health surface (`kyma_connector_health`) and a `pushdown_summary` on every federated response, so users can see what kyma did and trust it.
- The full surface fits behind kyma's existing primitives: object storage is the only durable source of truth, compute is stateless, catalog is externalized, format/parser plugability stays intact.

### Non-goals (v1)

- Engines beyond Postgres / MySQL / Mongo. The trait shape is designed to absorb others (SQLite, SQL Server, Oracle, DynamoDB, ClickHouse, Snowflake-as-source, BigQuery-as-source); they ship in v2 as new `ExternalSource` impls.
- JSONB / MySQL JSON / array drill-in (recursive schema inference inside `dynamic` content). Deferred to v1.5; v1 lands such columns whole in `dynamic`.
- Read-time consistency fences (the "wait for sync to catch up before answering" pattern). Federation mode covers this need.
- Window-function pushdown — engine semantics diverge enough that v1 evaluates window functions above the scan.
- Multi-region federation. Inherits whatever kyma does for multi-region overall.
- Auto-recreate of externally-dropped replication slots without operator sign-off.
- Connector alerts/SLOs in the connector config (Q7-D). Deferred; the data is exposed in `kyma_connector_health` for users to alert on with their existing tools.

---

## 3. Architecture

### 3.1 The single load-bearing abstraction

`ExternalSource` is the only engine-specific surface. Two consumers read from it:

```
                              ┌──────────────────────────────────────────┐
                              │  kyma-connectors::ExternalSource (trait) │
                              │   - connect / health                     │
                              │   - introspect_schema()                  │
                              │   - capabilities() -> Capabilities       │
                              │   - federated_scan(plan, batch_sink)     │
                              │   - open_cdc_stream(from_checkpoint)     │
                              │   - take_consistent_snapshot(at_lsn)     │
                              └──────────────────────────────────────────┘
                                  │                                │
            ┌─────────────────────┘                                └─────────────────────┐
            ▼                                                                            ▼
┌──────────────────────────────┐                              ┌────────────────────────────────────┐
│  Federation consumer         │                              │  Sync consumer                     │
│  (kyma-federation crate)     │                              │  (kyma-connectors::CdcConnector)   │
│                              │                              │                                    │
│  - FederatedTableProvider    │                              │  - Snapshot coordinator            │
│  - FederatedCatalogProvider  │                              │  - CDC stream consumer             │
│  - PushdownPlanner (shared)  │                              │  - Schema-evolver                  │
│  - registered into           │                              │  - Transactional cursor commit     │
│    kyma-exec SessionContext  │                              │    via kyma-ingest-core            │
└──────────────────────────────┘                              └────────────────────────────────────┘
            │                                                                            │
            ▼                                                                            ▼
        DataFusion                                                              kyma extents
   (joins kyma + ext sources)                                              (synced data lives here)
```

Three concrete trait impls in v1's spec, one shipped per slice: `PostgresSource`, `MySqlSource`, `MongoSource`.

### 3.2 Where new code lives

- `crates/kyma-connectors/src/external/` — `ExternalSource` trait, `Capabilities`, shared types.
- `crates/kyma-connectors/src/external/postgres.rs`, `mysql.rs`, `mongo.rs` — engine impls (slice-by-slice).
- `crates/kyma-connectors/src/cdc/` — `CdcConnector` subtype, snapshot coordinator, schema evolver.
- **New crate `kyma-federation`** — `FederatedTableProvider`, `FederatedCatalogProvider`, `PushdownPlanner`. Separate crate (not folded into `kyma-exec`) because it depends on `kyma-connectors`, and `kyma-exec` must not take that dep.
- `crates/kyma-server/src/connectors/` — admin API extensions: `mode` field, `live(...)` resolver wiring, `/v1/connectors/:id/status`, `/v1/connectors/:id/events`, `/v1/connectors/:id/test-connection`.
- `crates/kyma-connectors-testkit` — private dev crate housing the property-test query generator, fault injector, and chaos test harness.

### 3.3 Where existing code is touched (not rewritten)

- `kyma-exec` — `SessionContext` builder gains a step to register `FederatedCatalogProvider`s alongside `KymaTable`. No changes to existing query paths.
- `kyma-ingest-core::CommitCoordinator` — gains a `with_cursor_update(connector_id, source_table, checkpoint)` builder method; the cursor advance lands in the same CAS that already commits the extent manifest. This is the exactly-once knot.
- `kyma-connectors::Connector` trait — kept as-is for non-CDC connectors (Prometheus). `CdcConnector` is a sibling trait; the registry holds either.
- Catalog migrations — additive (see §4.5).

### 3.4 The five kyma invariants are preserved

1. **Object storage is the only source of truth** — synced data lands as normal kyma extents; federated data never persists.
2. **Compute is stateless** — `ExternalSource` instances are recreated from the catalog row on any node, including pool config and resolved secrets.
3. **Catalog is externalized** — connector mode, CDC checkpoints, schema-inference state all live in Postgres.
4. **Format is pluggable** — no changes to segment format; we are a new producer of Arrow batches.
5. **Parser is pluggable** — no changes; federation works behind the existing SQL/KQL parsers via DataFusion's catalog system.

---

## 4. Components

### 4.1 `ExternalSource` trait

```rust
#[async_trait]
pub trait ExternalSource: Send + Sync + 'static {
    fn type_id(&self) -> &'static str;          // "postgres" | "mysql" | "mongo"
    fn capabilities(&self) -> &Capabilities;     // pushdown surface (filters, agg, etc.)

    async fn connect(&self, conn: &ResolvedConnection) -> Result<SourceHandle>;
    async fn health(&self, h: &SourceHandle) -> Result<SourceHealth>;

    // Schema introspection — called at connector create AND on each tick (cheap; cached)
    async fn introspect(&self, h: &SourceHandle, scope: &Scope) -> Result<InferredSchema>;

    // Federation — DataFusion calls this through FederatedTableProvider.
    async fn scan(&self, h: &SourceHandle, plan: &PushedPlan, sink: BatchSink) -> Result<ScanReport>;

    // Sync — only required when mode includes sync; default returns Unsupported.
    async fn open_cdc(&self, h: &SourceHandle, from: Option<Checkpoint>) -> Result<CdcStream> { ... }
    async fn snapshot_at(&self, h: &SourceHandle, scope: &Scope) -> Result<(SnapshotStream, Checkpoint)> { ... }
}
```

**Responsibility:** be the *only* engine-specific code path. Everything in `kyma-federation` and `kyma-connectors::cdc` is engine-agnostic and reads from this trait.

**Explicitly does NOT:** hold pool config (lives on `SourceHandle`), make catalog writes, decide whether to push a filter (the planner decides; `Capabilities` is descriptive, not active), retry transient errors (the consumer decides retry policy).

`Capabilities` is a struct, not a method. It is a static description of what the engine supports for pushdown — filter operators, function names, aggregation shapes, join shapes, sort/limit. The `PushdownPlanner` reads it; the engine never executes pushdown decisions. This makes the planner unit-testable by swapping any `Capabilities` config without touching real engines.

### 4.2 `kyma-federation` crate

Three components:

- **`FederatedCatalogProvider`** — implements DataFusion's `CatalogProvider`. One per registered source. Lazily lists schemas/tables by calling `ExternalSource::introspect` (cached with TTL, default 5 min, invalidated on observed schema drift).
- **`FederatedTableProvider`** — implements DataFusion's `TableProvider`. Receives `(filters, projection, limit, sort)` from DataFusion, hands them to the planner.
- **`PushdownPlanner`** — *engine-agnostic*. Takes a DataFusion logical-plan fragment and a `Capabilities`, returns:
  - `PushedPlan` — what the source will execute, in source-native form (`Sql(String)` for Postgres/MySQL, `MongoPipeline(Vec<Document>)` for Mongo).
  - `ResidualExpr` — what DataFusion must evaluate above the scan.
  - `PushdownSummary` — telemetry record (which filters pushed, which didn't, why; agg pushed; sort/limit pushed; same-source join pushed).

**Correctness rule:** if the planner cannot prove an expression is safe under the source's `Capabilities`, it leaves it in the residual. Every planner decision has a paired property test that runs the same query both ways and asserts identical Arrow output. Slow-but-right beats fast-and-wrong.

**Explicitly does NOT:** know about Postgres, MySQL, or Mongo. The planner output is a `PushedPlan` enum variant per query shape that the engine consumes opaquely.

### 4.3 `CdcConnector` (in `kyma-connectors::cdc`)

A sibling to the existing `Connector` trait, registered into the same registry. The runner detects which trait an instance implements and dispatches accordingly. The CDC pipeline runs in two phases per source-table:

```
  Phase 1 — INITIAL SNAPSHOT (run once)
  ┌──────────────────────────────────────────────────────────────────────┐
  │ 1. ExternalSource::snapshot_at(scope) returns (rows, checkpoint_at)  │
  │ 2. Stream rows in batches → SchemaEvolver → ingest write path        │
  │ 3. Commit final batch + cursor=checkpoint_at in one CAS transaction  │
  │ 4. Mark connector phase = "streaming"                                │
  └──────────────────────────────────────────────────────────────────────┘

  Phase 2 — STREAMING (continuous)
  ┌──────────────────────────────────────────────────────────────────────┐
  │ 1. ExternalSource::open_cdc(from=last_checkpoint) → CdcStream        │
  │ 2. Drain events into batches (group-commit, just like ingest does)   │
  │ 3. SchemaEvolver checks each batch for new fields                    │
  │ 4. Commit batch + new_checkpoint in one CAS transaction              │
  │ 5. Lag heartbeat → kyma_connector_health table every N seconds       │
  └──────────────────────────────────────────────────────────────────────┘
```

**Exactly-once mechanism:** the cursor advance is a payload in the same `tables.current_snapshot_id` CAS that already commits the extent manifest. We extend `kyma-ingest-core::CommitCoordinator` with a single `with_cursor_update(connector_id, source_table, checkpoint)` builder method. Either both land or neither does — we inherit kyma's existing CAS retry logic.

**Explicitly does NOT:** parse CDC events itself (that's the engine impl), decide schema-evolution policy (that's `SchemaEvolver`), implement retry — transient stream errors bubble up; the runner reopens the stream from the last committed checkpoint, which is always safe because of exactly-once.

### 4.4 `SchemaEvolver`

Engine-agnostic. Takes a stream of `(field_name, observed_type, observed_count)` samples and decides:

- **Stable typed column** — appeared in ≥ N events with one consistent type → mapped to typed Arrow column.
- **Polymorphic / new** — appears with mixed types or below the stability threshold → routed to the `dynamic` (CBOR) overflow column.
- **Promote-from-dynamic** — a previously-polymorphic field that has stabilized → emit `ALTER TABLE ADD COLUMN` and start writing it typed.

Reuses kyma's existing schema-evolution machinery from the recent filedrop work (commit `5372dc5d`). We do not reimplement evolution — Mongo/Postgres-JSONB inferred fields route into the existing path.

**Default thresholds:** 100 events with consistent type within a sliding window of 1000 events (or the entire snapshot if < 1000 rows). Tunable per-connector via `sync.inference.stability_threshold`.

**Explicitly does NOT:** delete columns, narrow types, or rewrite history. Schema only widens. This is kyma's existing rule.

### 4.5 Catalog schema additions (additive only)

```sql
-- Mode + structured connection (split out from config_jsonb)
ALTER TABLE connectors ADD COLUMN mode TEXT NOT NULL DEFAULT 'sync';
ALTER TABLE connectors ADD COLUMN connection_jsonb JSONB;
ALTER TABLE connectors ADD COLUMN scope_jsonb JSONB;

-- CDC state (per source-table)
CREATE TABLE connector_cdc_state (
    connector_id  UUID NOT NULL REFERENCES connectors(id),
    source_table  TEXT NOT NULL,
    phase         TEXT NOT NULL,             -- 'pending' | 'snapshotting' | 'streaming' | 'errored'
    checkpoint    JSONB,                      -- engine-specific (LSN/GTID/resumeToken)
    rows_synced   BIGINT NOT NULL DEFAULT 0,
    last_event_at TIMESTAMPTZ,
    last_error    TEXT,
    PRIMARY KEY (connector_id, source_table)
);

-- Health, queryable as a kyma table — closes the observability loop.
-- Implemented as a view over connectors + connector_cdc_state + recent metrics.
CREATE VIEW kyma_connector_health AS ...;
```

Existing `connectors.config_jsonb` stays for backward compatibility with the Prometheus connector, which has no `connection`/`scope` structure.

### 4.6 Admin API extensions

`POST /v1/connectors` body extends with:

```json
{
  "type": "postgres",
  "mode": "both",
  "connection": {
    "url": "postgres://app@prod-rds:5432/app",
    "secret_ref": "pg_prod_password",
    "tls": "required",
    "pool_size": 10
  },
  "scope": {
    "include_schemas": ["public", "billing"],
    "exclude_tables": ["public.audit_log"]
  },
  "sync": {
    "tables": ["public.users", "billing.invoices"],
    "schedule_ms": null
  }
}
```

New endpoints:

- `GET /v1/connectors/:id/status` — structured health doc (federation + sync + source).
- `GET /v1/connectors/:id/events` — last 100 state transitions (phase changes, error onsets, recoveries).
- `POST /v1/connectors/:id/test-connection` — runs `ExternalSource::health` synchronously; used by the UI before save.
- `POST /v1/connectors/:id/pause?scope=sync|federation|all` — mode-isolated pause.

`POST /v1/connectors` validates that all `secret_ref`s resolve before persisting (fail-fast).

### 4.7 Component boundaries summary

| Component | Owns | Does NOT own |
|---|---|---|
| `ExternalSource` impl | Engine I/O, schema introspection, CDC stream parsing | Pushdown decisions, retry, catalog writes |
| `PushdownPlanner` | Plan translation, residual derivation, summary emission | Engine-specific syntax (delegates to `PushedPlan` builder) |
| `FederatedTableProvider` | DataFusion glue, scan-time pool checkout, telemetry | Pushdown logic, schema cache eviction policy |
| `CdcConnector` runner | Phase machine, batch group-commit, lag heartbeat | Event parsing, pool management |
| `SchemaEvolver` | Inference rules, ALTER TABLE emission | Source-specific type → Arrow mapping (engine provides) |
| Admin API | Validation, secret resolution gating, status aggregation | Engine logic |

---

## 5. Data flow

### 5.1 Federated query

User: `SELECT u.email, count(*) FROM pg_prod.public.users u JOIN kyma.default.otel_logs l ON l.user_id = u.id WHERE l.severity = 'ERROR' AND u.region = 'eu' GROUP BY u.email LIMIT 50`

```
Client (web UI / agent / SQL POST)
   │  POST /v1/query  (Content-Type: application/sql)
   ▼
kyma-server::query
   │  parse → kyma-plan LogicalPlan (frontend-neutral IR)
   ▼
kyma-exec::SessionContext  (DataFusion)
   │  SessionContext has registered:
   │    - "kyma" CatalogProvider                (existing)
   │    - "pg_prod" FederatedCatalogProvider    (new)
   │    - "mongo_shop" FederatedCatalogProvider
   │
   │  Logical plan resolves table refs to providers.
   ▼
DataFusion physical planning
   │
   │  For FederatedTableProvider, DataFusion calls scan(filters, projection, limit, sort):
   │     PushdownPlanner reads PostgresSource::capabilities() and builds:
   │       PushedPlan::Sql("SELECT id, email FROM public.users WHERE region = $1")
   │                  + bind: ['eu']
   │       residual: []
   │       summary:  { filters_pushed: 1, projection_pushed: true,
   │                   agg_pushed: false, agg_reason: "cross-source group-by" }
   │
   │  For KymaTable, the existing pruning cascade runs.
   ▼
Physical execution (parallel)
   │
   │  ┌─────────────────────────────────────────┐    ┌─────────────────────────────┐
   │  │ FederatedScan on PostgresSource         │    │ KymaScan on otel_logs       │
   │  │   pool.acquire() → conn                 │    │   (existing extent fetch)   │
   │  │   conn.execute(sql, bind)               │    │                             │
   │  │   stream rows → Arrow batches → sink    │    │   stream Arrow batches      │
   │  │   on EOF: pool.release(); emit summary  │    │                             │
   │  └─────────────────────────────────────────┘    └─────────────────────────────┘
   │                       │                                       │
   │                       └───────────────┬───────────────────────┘
   │                                       ▼
   │                              DataFusion HashJoin → HashAgg → TopK(50)
   ▼
kyma-server response
   │  Arrow Flight stream OR JSON
   │  Trailing record / response header includes pushdown_summary array
   │  (one entry per FederatedScan)
```

The `pushdown_summary` is **on every response, always**. It is the trust mechanism for federation.

### 5.2 Initial snapshot (sync mode, phase 1)

For `pg_prod` syncing `public.users`:

```
Connector created with mode = "sync" (or "both")
   │  POST /v1/connectors  →  row inserted, mode='sync', phase NULL
   ▼
Connector runner picks up the new row
   │  detects connector_cdc_state has no row for ('pg_prod', 'public.users')
   │  → enters PHASE 1: SNAPSHOT
   ▼
Acquire snapshot point on the source
   │  PostgresSource::snapshot_at(scope) — engine-specific:
   │    BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
   │    SELECT pg_create_logical_replication_slot('kyma_<connector_id>',
   │                                              'pgoutput',
   │                                              true);
   │    SET TRANSACTION SNAPSHOT '<exported_snapshot_id>';
   │  Returns: (SnapshotStream over rows, Checkpoint{ lsn: X })
   │
   │  Persist phase='snapshotting', checkpoint=X to connector_cdc_state.
   ▼
Stream snapshot rows
   │  For each batch of N rows:
   │    SchemaEvolver inspects → InferredSchema (typed cols + dynamic overflow)
   │    JSON → Arrow coercion (existing arrow_coerce.rs)
   │    Write through kyma-ingest-core staging buffer
   │  No cursor advance during snapshot — checkpoint stays at X.
   ▼
Snapshot complete (source COMMIT)
   │  Final batch lands AND connector_cdc_state advances to phase='streaming'
   │  in ONE catalog transaction (CAS commit on the table, plus the cdc_state UPDATE).
   │  Either the table sees the snapshot AND cdc_state advances, or neither does.
   ▼
Phase 2 begins automatically (next tick)
```

MySQL uses `START TRANSACTION WITH CONSISTENT SNAPSHOT` + GTID capture. Mongo uses `$changeStream` `startAtOperationTime` taken before the snapshot read. The shared shape: capture a checkpoint, read at it, the engine tells us what "at it" means.

### 5.3 Steady-state CDC (sync mode, phase 2)

```
Runner enters PHASE 2: STREAMING
   │
   ▼
Open CDC stream from last checkpoint
   │  PostgresSource::open_cdc(from=Checkpoint{lsn: X})
   │    START_REPLICATION SLOT kyma_<connector_id>
   │      LOGICAL <lsn>
   │      ('proto_version' '2', 'publication_names' '...')
   │  Returns: CdcStream of typed events (Insert/Update/Delete + schema messages)
   ▼
Group-commit loop
   │  Read events for up to MAX_BATCH_BYTES or MAX_BATCH_MS, whichever first.
   │  (Reuses kyma-ingest-core's existing staging-buffer logic.)
   │
   │  For each batch:
   │    1. SchemaEvolver checks for new fields → may emit ALTER TABLE
   │       (handled by existing schema-evolution path; mid-batch ALTERs flush-then-append).
   │    2. Coerce events to Arrow; deletes become tombstone rows
   │       (timestamp + pk + tombstone_flag).
   │    3. Track high-watermark LSN of last event in batch → new_checkpoint.
   │
   │    4. CommitCoordinator.commit_with_cursor(
   │           extent_manifest,
   │           cursor_update = (connector_id, source_table, new_checkpoint))
   │       ATOMIC: snapshot CAS + connector_cdc_state UPDATE in one txn.
   │       On CAS conflict: existing retry path runs (writes+cursor both retry).
   │
   │    5. Heartbeat → kyma_connector_health row
   │       (lag = source_now - last_event_at, events_per_sec, …)
   ▼
On stream error
   │  Drop the stream object;
   │  exponential backoff (existing background_tasks behavior);
   │  reopen with from=last_committed_checkpoint.
   │  Safe to reopen because the cursor IS the last committed point.
```

### 5.4 Same data, two views (`mode=both`)

When a source has `mode=both` and a table is in both `scope.include` and `sync.tables`:

- `SELECT * FROM pg_prod.public.users WHERE id = ?` → resolves to the synced kyma extent.
- `SELECT * FROM live(pg_prod.public.users) WHERE id = ?` → resolves to the federated path (live read).
- `GET /v1/catalog/schema` shows the table once with both modes annotated.

`live(...)` is implemented as a DataFusion `TableFunction` that swaps the resolved provider from kyma-extent-backed to federation-backed.

---

## 6. Schema mapping

### 6.1 Universal rules

1. Synced data is queryable as `<source>.<source_db>.<source_table>` and lives in normal kyma extents using kyma's existing column types (`int`, `long`, `real`, `bool`, `string`, `timestamp`, `dynamic`, `vector(N)`).
2. Schema only widens. Never narrow, never delete, never re-type.
3. Stability threshold for inference: ≥ 100 events with one consistent inferable type within a sliding window of 1000 events (or entire snapshot if < 1000 rows). Tunable via `sync.inference.stability_threshold`.
4. Nullability is permissive. Every inferred column is nullable.
5. **System identity columns** kyma adds to every synced table:
   - `_kyma_pk` (`string`) — concatenated source PK (or `_id` for Mongo).
   - `_kyma_op` (`string`) — `'insert' | 'update' | 'delete'`.
   - `_kyma_lsn` (`string`) — engine-specific cursor at commit time.
   - `_kyma_event_at` (`timestamp`) — wall-clock time the source emitted the event.
6. **Row semantics:** every CDC event produces exactly one row in kyma. An INSERT emits `_kyma_op='insert'` with the new values; an UPDATE emits `_kyma_op='update'` with the post-image values (kyma does not store the pre-image — it lives in earlier rows for the same `_kyma_pk`); a DELETE emits a tombstone row with `_kyma_op='delete'` and the PK columns populated. Querying "latest state per row" is `latest by _kyma_event_at over _kyma_pk where _kyma_op != 'delete'`.
7. Deletes are tombstones, not row removal. Reads at the federation/agent layer filter `_kyma_op != 'delete'` by default; `live(...)` is unaffected.
8. Compaction collapses tombstones older than `retention.tombstone_days` (default 30) via the existing `kyma-compaction` retention sweeper.

### 6.2 Postgres → kyma type mapping

| Postgres type | kyma type | Notes |
|---|---|---|
| `smallint`, `int` | `int` | 32-bit signed |
| `bigint`, `oid` | `long` | |
| `real`, `double precision` | `real` | 64-bit float |
| `numeric(p, s)` | `real` for `p ≤ 15`; `string` otherwise | Forced via `connection.numeric_mode = "string"` |
| `boolean` | `bool` | |
| `text`, `varchar`, `char`, `name`, `uuid` | `string` | UUIDs canonical form |
| `bytea` | `string` (base64) | Bytes-typed columns post-v1 |
| `timestamp`, `timestamptz`, `date`, `time` | `timestamp` | Always UTC; loses sub-microsecond precision |
| `json`, `jsonb` | `dynamic` (CBOR) | Whole document; field-level inference inside JSONB is post-v1 |
| `int[]`, `text[]`, etc. | `dynamic` | Arrow `List<T>` mapping post-v1 |
| `int4range`, `tstzrange`, etc. | `dynamic` | `{lower, upper, lower_inc, upper_inc}` |
| `geometry` (PostGIS) | `string` (WKT) | Opt-in via `scope.geometry_mode = "wkt"` |
| `enum` | `string` | |
| `hstore` | `dynamic` | Map representation in CBOR |
| `inet`, `cidr`, `macaddr` | `string` | |
| `vector` (pgvector) | `vector(N)` | Dimension fixed; mismatch errors loudly |

Composite types and domains unwrap to their base type; out-of-band types not in this table land in `dynamic` with a `last_error` warning.

### 6.3 MySQL → kyma type mapping

| MySQL type | kyma type | Notes |
|---|---|---|
| `tinyint`, `smallint`, `mediumint`, `int` | `int` | `tinyint(1)` → `bool` |
| `bigint`, `bigint unsigned` | `long` | `bigint unsigned` > i64 max → `string` with warning |
| `decimal(p, s)` | `real` for `p ≤ 15`; `string` otherwise | |
| `float`, `double` | `real` | |
| `bit` | `bool` (width 1) or `dynamic` (wider) | |
| `date`, `datetime`, `timestamp`, `time`, `year` | `timestamp` | UTC; `time` and `year` stringified post-v1 |
| `char`, `varchar`, `text`, `longtext`, `enum`, `set` | `string` | `set` comma-joined |
| `binary`, `varbinary`, `blob` | `string` (base64) | |
| `json` | `dynamic` (CBOR) | Same rule as Postgres JSONB |
| Spatial (`geometry`, `point`, …) | `string` (WKT) | |

**Collation safety (correctness rule):** the `PushdownPlanner` for MySQL refuses to push any `string` equality / `LIKE` filter unless the column's collation is one of the case-sensitive variants explicitly verified (`utf8mb4_bin`, `utf8mb4_0900_as_cs`). The `Capabilities` struct carries `string_collation_safe_columns: Set<TableQualifiedName>`, populated at introspection time. Filters on case-insensitive columns evaluate above the scan in DataFusion. This is the kind of trap that would silently corrupt federated query results if we got it wrong.

### 6.4 MongoDB → kyma type mapping

Top-level fields each get one inferred kyma column. Nested objects flatten with dotted names up to `scope.flatten_depth` (default 2). Anything deeper or polymorphic lands in `dynamic`.

| BSON type | kyma type |
|---|---|
| `Int32` | `int` |
| `Int64`, `Decimal128` (when fits) | `long` |
| `Double`, `Decimal128` (otherwise) | `real`; force `string` via `scope.decimal128_mode` |
| `Boolean` | `bool` |
| `String` | `string` |
| `ObjectId` | `string` (24-char hex) |
| `UUID` (binary subtype 4) | `string` |
| `Date` | `timestamp` (UTC, ms precision) |
| `Timestamp` (BSON timestamp) | `timestamp` |
| `Binary` (other subtypes) | `string` (base64) |
| `Null`, `Undefined` | NULL |
| `Array` of homogeneous primitive | `dynamic` (Arrow `List<T>` post-v1) |
| `Array` of mixed / objects | `dynamic` |
| `Object` (≤ flatten_depth) | flattened to dotted columns |
| `Object` (> flatten_depth) | `dynamic` |
| `RegExp`, `JavaScript`, `MinKey`, `MaxKey`, `DBPointer`, `Symbol` | `dynamic` (rare; warn) |

**Flattening example** (`flatten_depth = 2`):

```
Mongo doc:
{
  "_id": ObjectId("..."),
  "user": { "id": 42, "email": "a@b.com", "addr": { "city": "Berlin" } },
  "total": 99.5,
  "tags": ["a", "b"]
}

→ kyma columns:
  _kyma_pk        string  "<ObjectId hex>"
  user.id         int     42
  user.email      string  "a@b.com"
  user.addr       dynamic {city: "Berlin"}    ← depth 3, lands in dynamic
  total           real    99.5
  tags            dynamic ["a", "b"]           ← arrays go to dynamic in v1
```

**Polymorphic field handling:** if a field is observed as `int` 800 times then `string` shows up, the SchemaEvolver:

1. Stops promoting that field; it stays `dynamic`.
2. Existing typed-column data is preserved.
3. Future events go into `dynamic`. Queries union-read the typed and dynamic copies via `coalesce(typed_col, dynamic.field)`.

### 6.5 JSONB / nested-relational deferred to v1.5

Postgres JSONB and MySQL JSON columns land whole in `dynamic` for v1. Field-level inference inside JSONB is out of scope. Reasoning:

- The token index over `dynamic` already makes JSONB queryable.
- Inferring inside JSONB requires sampling JSONB content, which CDC events don't always carry in full (Postgres replication only sends changed columns by default).
- Doing it right means a second-level evolver running over `dynamic` content; a meaningful design exercise of its own.

The v1.5 path: a `scope.jsonb_drill: { columns: [...], stability_threshold: ... }` setting that turns specific JSONB columns into Mongo-style flattened sub-columns using the same `SchemaEvolver` recursively.

### 6.6 Schema introspection cache & invalidation

- `ExternalSource::introspect` is called on connector create, then on a TTL (default 5 min) or whenever a CDC event with a schema-change marker arrives.
- The cache lives in `kyma-federation` (federation consumer) and `kyma-connectors::cdc` (sync consumer); both ask the same `ExternalSource`, so they stay consistent.
- Stale cache → at worst, a federated query plans against an old schema, the source returns a column the planner didn't know about, and the scan errors. Next plan pulls fresh schema. Self-healing.

### 6.7 `_kyma_pk` derivation rules

| Source | `_kyma_pk` is |
|---|---|
| Postgres / MySQL (single-column PK) | string-cast of the PK |
| Postgres / MySQL (composite PK) | `<col1>:<col2>:...` (URL-safe, deterministic order from `information_schema`) |
| Postgres / MySQL (no PK) | connector **fails to start** for that table with `last_error = "table has no primary key — cannot CDC sync"` |
| MongoDB | `_id` always exists; stringified |

No-PK-table failure is intentional: CDC without a PK is unsound (cannot dedupe replays, cannot tombstone). The user sees the clear error and either adds a PK, excludes the table, or uses `mode=federation` only for it.

---

## 7. Pushdown best practices

### 7.1 Always push down (when capability says yes)

- **Column projection** — never fetch columns the query doesn't need.
- **Filters** — `=`, `!=`, `<`, `<=`, `>`, `>=`, `IN`, `IS NULL`, `LIKE` (with translated wildcards), AND/OR/NOT trees over those. Per-engine, expressions without 1:1 translation stay above the scan.
- **`LIMIT`** and **`ORDER BY`** — including the combined "TopK" shape.
- **Single-source aggregations** — `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `COUNT(DISTINCT)`, `GROUP BY` — when every column referenced is on the same source.

### 7.2 Push down when cheap and safe

- Joins where both sides are the same external source and same connection (opportunistic). DataFusion's logical plan makes this detectable. We do **not** push joins across different engines.
- Common scalar functions with verified semantics per engine (`LOWER`, `UPPER`, `COALESCE`, date truncation). Anything we cannot prove behaves identically on the source vs. DataFusion stays above the scan.

### 7.3 Never push down (in v1)

- Anything that touches a kyma-specific UDF (`cosine_distance`, token-index functions, `dynamic` accessors).
- Cross-source joins.
- Window functions.
- Anything where we cannot prove identical NULL/collation/timezone semantics between source and DataFusion (e.g., MySQL case-insensitive collation on `=` and `LIKE`).

### 7.4 Mechanism

Each `ExternalSource` exposes a per-engine `Capabilities` struct (operators, functions, agg shapes, sort/limit, joins, collation-safe columns). The shared `PushdownPlanner` walks the DataFusion logical plan, peels off everything `Capabilities` allows, builds a `PushedPlan` (SQL string for Postgres/MySQL, aggregation pipeline for Mongo), and leaves the rest above the scan as a normal DataFusion plan. One planner, three capability profiles — adding a fourth engine is "fill in the capabilities table."

### 7.5 Correctness rule

If the planner is uncertain, it does **not** push down. Slow-but-right beats fast-and-wrong. Every pushdown decision has a unit test that runs the same query both ways (pushed vs. unpushed) and asserts identical Arrow output. Property tests over a generated query space gate every pushdown change in CI.

---

## 8. Secrets, connections, security

### 8.1 SecretStore-by-reference

Connection credentials are never stored plaintext in the catalog. Config stores `{"password": {"$secret": "pg_prod_password"}}`; the actual value lives in whatever the deployment's `SecretStore` implementation points at. The connector framework resolves `$secret` references at run-time per tick.

The `SecretStore` trait already exists at `crates/kyma-connectors/src/secrets.rs`. v1 ships three concrete backends:

- **Env-var** (default) — `KYMA_SECRET_<name>` resolves to the value. Simplest; suitable for dev/local.
- **File-based** — JSON/YAML at `KYMA_SECRETS_FILE`. Suitable for single-node production with mounted secrets.
- **In-cluster** — multi-node deployments fetch secrets from a kyma-server endpoint; nodes do not need to mount secrets files.

Cloud-provider backends (AWS Secrets Manager, GCP Secret Manager, HashiCorp Vault) are documented extension points; their trait impls land post-v1.

The Prometheus connector is migrated to use `SecretStore` as part of M0.

### 8.2 Connection pooling

Each `ExternalSource` instance holds:

- A federation pool: `sqlx::PgPool` / `sqlx::MySqlPool` / `mongodb::Client`. Sized via `connection.pool_size` (default 5, max 50). Reused across federated query ticks; never reconnect per query.
- A sync connection: separate dedicated connection (replication/CDC connections are typically 1 per slot/binlog reader anyway).

Pool stats (`pool_in_use`, `pool_max`, `pool_acquire_timeout_ms`) surface in the status endpoint.

### 8.3 TLS by default

Federation/sync connectors require TLS unless `connection.tls = "disabled"` is explicitly set. Setting it without an override emits a loud warning in `last_error`. Connectors with `tls = "required"` (default) refuse to start against plaintext sources.

### 8.4 Secret-resolution gating

`POST /v1/connectors` runs a `validate_secrets_resolvable` step before persisting. A connector with a typo'd secret name fails fast at create time instead of silently retrying forever.

---

## 9. UX & addressing

### 9.1 Three-part naming

Tables are addressed as `<source>.<source_db>.<source_table>`. kyma's existing `<database>.<table>` extends to `kyma.<database>.<table>` for symmetry; bare `<table>` resolves against `kyma.default` first, then errors if ambiguous. Maps cleanly onto DataFusion's `CatalogProvider` model.

### 9.2 Views

Users can register `CREATE VIEW current_users AS SELECT * FROM pg_prod.public.users WHERE active = true` and reference `current_users` thereafter. Hides source addressing from agents/dashboards; lets ops swap a source without breaking queries. DataFusion supports views natively.

### 9.3 Agent integration

The agent's existing schema RAG (pgvector + `schema_embeddings`) extends to include external sources at registration time. The connector emits its inferred schema into the same vector index. The agent's `describe_table`, `run_sql`, `list_databases` tools transparently work over external sources. Users say "look at orders" and the agent finds the right table regardless of which source it lives on.

### 9.4 Mode resolution rule

When a table exists in both `mode=sync` and `mode=federation` (i.e., `mode=both`):

- Bare reference → synced extent (predictable, fast).
- `live(table)` → federated path (live, source-fresh).

No alternative namespaces, no SQL hints, no session vars. Just one explicit override function.

### 9.5 Catalog API

`GET /v1/catalog/schema` returns a tree grouped by source. Synced tables show their lag stats; federated tables show their source health. The web UI renders a unified browser.

---

## 10. Observability

### 10.1 Status endpoint

`GET /v1/connectors/:id/status` returns a structured doc:

```json
{
  "id": "...",
  "type": "postgres",
  "mode": "both",
  "source": {
    "reachable": true,
    "version": "PG 15.4",
    "replication_slot": "kyma_pg_prod",
    "last_health_check": "2026-05-02T10:00:00Z"
  },
  "federation": {
    "status": "healthy",
    "pool_in_use": 2,
    "pool_max": 10,
    "p50_query_ms": 14,
    "p99_query_ms": 230,
    "queries_total_5m": 1240,
    "errors_5m": 0,
    "last_error": null
  },
  "sync": {
    "status": "streaming",
    "phase": "streaming",
    "lag_seconds": 4,
    "last_event_at": "2026-05-02T10:00:00Z",
    "events_per_sec": 1200,
    "rows_synced": 5_240_000,
    "schema_drift": [
      { "table": "public.users", "field": "preferences",
        "status": "demoted_to_dynamic", "since": "2026-05-02T09:14:00Z" }
    ],
    "last_error": null
  }
}
```

### 10.2 `pushdown_summary` on every federation response

Every response from a query that touched a federated source carries a `pushdown_summary`, one entry per `FederatedScan`:

```json
{
  "source": "pg_prod",
  "table": "public.users",
  "filters_pushed": ["region = $1"],
  "filters_residual": [],
  "projection_pushed": true,
  "limit_pushed": 50,
  "sort_pushed": null,
  "agg_pushed": null,
  "agg_residual_reason": "cross-source group-by",
  "join_pushed": false,
  "scan_duration_ms": 14,
  "rows_returned": 50,
  "bytes_received": 4128
}
```

The web UI shows it; the agent records it; CI tests assert pushdown does not regress on key queries. This prevents pushdown from silently degrading as the planner evolves.

### 10.3 `kyma_connector_health` table

Exposed as a queryable kyma table (a view over `connectors` + `connector_cdc_state` + recent metrics). Schema:

| Column | Type | Description |
|---|---|---|
| `connector_id` | `string` | |
| `connector_name` | `string` | |
| `type` | `string` | `postgres` / `mysql` / `mongo` / `prometheus` / … |
| `mode` | `string` | `federation` / `sync` / `both` |
| `phase` | `string` | per source-table |
| `lag_seconds` | `real` | sync-mode lag |
| `events_per_sec` | `real` | sync throughput |
| `pool_in_use` | `int` | federation pool snapshot |
| `pool_max` | `int` | |
| `last_error` | `string` | |
| `last_event_at` | `timestamp` | |
| `timestamp` | `timestamp` | row emit time (matches kyma's standard column name) |

Agents query this with KQL/SQL; dashboards chart it; alerts fire on it. This is what closes the "kyma observing kyma observing your databases" loop without needing dedicated alert config.

### 10.4 Events trail

`GET /v1/connectors/:id/events` returns the last 100 state transitions: phase changes, error onsets, recoveries. Time-bounded by retention (default 30 days). For postmortem: when did sync break? when did it recover? what was the lag spike?

---

## 11. Error handling & failure modes

### 11.1 Failure taxonomy

| Class | Examples | Behavior |
|---|---|---|
| **Transient** | Network blip, source briefly unreachable, pool acquire timeout, CDC stream dropped | Retry with exponential backoff. Cursor never advances past unconfirmed work. `last_error` set; `phase` stays `streaming`. |
| **Operator-actionable** | Bad credentials, unreachable host (DNS), TLS handshake failure, missing replication slot privilege, table without PK, unrepresentable schema change | Connector enters `disabled` with `disabled_reason`. Stops retrying. Operator must fix and re-enable. |
| **Bug** | Panic in pushdown planner, schema-evolver assertion fail, kyma-internal CAS bug | Connector enters `errored`, `connector_panics_total` metric increments, stack trace logged. Pages on threshold. |

The classification happens at the `ExternalSource` impl boundary — engines map their native errors to one of these three. The shared runner does not interpret engine-specific errors.

### 11.2 Federation failure matrix

| Failure | Detection | What happens | What user sees |
|---|---|---|---|
| Source unreachable | `pool.acquire()` errors | HTTP 502 `{error: "source_unreachable", source, detail}` | Agent retries against synced data |
| Source slow → query budget exceeded | Existing kyma query budget | HTTP 504; pool released | `pushdown_summary` shows `cancelled: true` |
| Pool exhausted | `pool.acquire()` exceeds timeout | HTTP 503 `pool_exhausted` | Visible in status endpoint |
| Source returns unexpected column type | Arrow conversion fails | HTTP 5xx `schema_drift`; cache invalidated; next plan self-heals | One-shot self-heal |
| Pushdown produced incorrect SQL (bug) | Source SQL parse error | HTTP 5xx `pushdown_failed`; logs SQL + residual | Caught in CI by property tests |
| Connector disabled | Catalog provider lookup | Plan-time error `source_disabled` | Agent reacts (e.g., uses synced version) |
| TLS handshake fails | Pool init | Connector `disabled`, `disabled_reason="tls_failed: <detail>"` | Status red |

### 11.3 Sync failure matrix

| Failure | Detection | What happens | What user sees |
|---|---|---|---|
| Crash mid-snapshot | `phase='snapshotting'` on restart | Drop slot; nuke uncommitted partial state; restart snapshot from scratch | `rows_synced` resets; no duplicates because nothing was committed |
| Crash mid-streaming-batch | Runner restart | Reopen CDC stream from `connector_cdc_state.checkpoint`; source replays; lands in next batch | Brief lag spike; no data loss; no duplicates |
| Stream EOF / network error | Stream returns error | Drop stream; backoff; reopen from last checkpoint | Lag rises then recovers |
| CAS conflict on commit | Existing kyma path | Retry. Cursor + extent both retry together (atomic) | Invisible |
| Replication slot dropped externally (PG) | `START_REPLICATION` "slot does not exist" | Connector `disabled`, `disabled_reason="replication_slot_missing"`. Does NOT auto-recreate | User decides: re-enable (re-snapshot) or restore externally |
| Source DDL emits unrepresentable type | SchemaEvolver hits unknown type | Field routed to `dynamic` overflow; warning in `last_error`. Sync continues | Warning visible; data still queryable |
| Source DDL drops column | CDC event missing the column | Column null-fills going forward; existing data preserved | No-op |
| Source DDL renames column | New column appears, old goes silent | Both columns exist; old stops writing, new starts | Cosmetic dup; user can ignore |
| Source PK changes | Detected at next introspection | Connector `disabled`, `disabled_reason="pk_changed"`. Manual resync | Manual: PK changes are rare and dangerous; auto-recovery is more dangerous |
| Source clock skew | Heartbeat lag computation negative | Logged warning; `kyma_connector_health.lag_seconds` clamped to 0 | Cosmetic; cursors are LSN-based |
| Source character encoding garbage | UTF-8 decode fails | Field becomes `dynamic` with raw bytes base64'd; warning | Visible in dynamic |
| ALTER TABLE fails (catalog error) | CAS conflict on schema bump | Existing retry. If persistent: `errored`, paged | Should be impossible in practice |

### 11.4 Mode-isolated disable

`mode=both` connectors can have federation healthy while sync errored, and vice versa. The status doc separates them. `/v1/connectors/:id/pause?scope=sync|federation|all` (default `all`) lets ops disable one mode without re-snapshotting later.

### 11.5 Backpressure & resource isolation

- Federation pool sized per connector. Federation queries acquire from this pool.
- Sync uses a separate dedicated connection (replication connections are 1 per slot anyway).
- Per-tick row budget for sync: `sync.max_rows_per_batch` (default 50,000). Prevents huge transactions from starving.
- Federation budget = existing kyma per-query budget; nothing new.
- CDC backpressure: if the staging buffer fills (existing mechanism), the CDC consumer stops reading. Source CDC mechanism handles this — Postgres slots buffer on disk up to `max_slot_wal_keep_size`; Mongo's oplog has fixed retention. Sustained backpressure eventually loses the slot/oplog position and becomes the "replication slot dropped" error. Heartbeat lag is the early warning.

### 11.6 Runner state machine

| State | Periodic ticks? | Auto-recovery? | Operator action |
|---|---|---|---|
| `enabled, phase=pending` | yes | n/a | none |
| `enabled, phase=snapshotting` | continuous | continues | none |
| `enabled, phase=streaming` | continuous | yes (reopens stream) | none |
| `enabled, phase=errored` (transient) | yes (backoff) | yes | none unless persistent |
| `disabled` (operator-actionable) | no | no | fix root cause; `POST /resume` |
| `disabled` (panic / bug) | no | no | team triage |
| `paused` (operator-initiated) | no | n/a | `POST /resume` |

---

## 12. Testing strategy

### 12.1 Four layers

| Layer | Proves | Tooling | Where |
|---|---|---|---|
| Unit | Each piece in isolation | `cargo test` | Per crate |
| Property | Pushdown correctness across generated query space | `proptest` | Per crate, gated in CI |
| Integration | Real engines, real CDC, real failures | `testcontainers` | CI + on-demand |
| Chaos | Crash/network/clock failures don't corrupt state | testcontainers + fault injector | Nightly CI |

Each engine slice (Postgres → MySQL → Mongo) clears all four before merging.

### 12.2 Property tests for pushdown correctness

The bet-the-product layer.

**Property:** for any generated query over a federated table, executing with pushdown produces an Arrow result identical (modulo ordering when no `ORDER BY`) to executing with pushdown disabled.

**Generator:** parameterized over a fixed schema (`id long, email string, region string, score real, created_at timestamp`). Generates filter expressions (`=, !=, <, <=, >, >=, IN, IS NULL, LIKE`, AND/OR/NOT trees up to depth 4), projections, sorts, limits, single-source aggs. Functions drawn from a curated allowlist; everything outside is residual.

**Runner:** executes each generated query against a real source twice — pushdown enabled vs. disabled (DataFusion `MemoryExec` over the same data). Sorts both Arrow results; compares row-for-row.

**Per-engine instances:** the same generator runs against Postgres, MySQL, Mongo. Catches the MySQL collation-case-insensitivity trap; catches Mongo aggregation-pipeline edge cases.

**Seed budget:** 10,000 cases per engine on every PR; 100,000 nightly. Failing seeds become pinned regression tests.

### 12.3 Integration suite (per engine)

Federation suite: single-source SELECT, cross-source JOIN, same-source JOIN with opportunistic pushdown, single-source aggregation, cross-source aggregation, source-unreachable, pool-exhaustion, TLS-required-by-default, schema drift mid-query, `pushdown_summary` always present.

Sync suite: 100k-row initial snapshot, snapshot+stream, updates, deletes/tombstones, schema add column, schema add unrepresentable column, composite PK, no-PK rejection, `mode=both` resolution, `kyma_connector_health` heartbeat correctness.

Mongo-specific: polymorphic field stays in dynamic, field stabilizes mid-stream and promotes, nested-doc flattening at depth 2, BSON type coercion (ObjectId, Decimal128, Date, BSON Timestamp), array → dynamic.

### 12.4 Chaos scenarios (gating v1)

| Scenario | Setup | Fault | Assertion |
|---|---|---|---|
| Snapshot crash | 1M-row sync | `kill -9` at 5×10 random offsets | Snapshot completes; row count matches source exactly |
| Streaming crash | 1k inserts/sec from source | `kill -9` once/min for 30 min | After quiesce: row count + content equals source |
| Source restart | Streaming under load | Restart source container | Stream reopens at checkpoint; no dups, no gaps |
| Network partition | Streaming | iptables-drop source↔kyma 60s | Lag spike then recovery |
| Catalog flap | Streaming | Restart catalog Postgres | CAS retry absorbs; no data loss |
| Slot dropped externally | Streaming | `pg_drop_replication_slot` from another conn | Connector `disabled` with correct reason; no silent re-snapshot |
| Clock skew | Streaming | Skew source clock ±60s | Lag clamped; cursors still correct |
| CAS storm | 10 concurrent writers | Run 10 min | All writes land; cursors monotonic |

Each asserts: at the end, the synced kyma table is exactly equivalent to the source's state (modulo deletes-as-tombstones), and no row was written twice, lost, or corrupted.

### 12.5 Performance benchmarks (`criterion`, gated in CI)

Federation latency:
- Single-row PK lookup: p50 < 10ms, p99 < 50ms.
- 1k-row scan with filter pushdown: p50 < 100ms.
- Cross-source join (10k Postgres × 1M kyma): p50 < 1s.

Sync throughput:
- Snapshot rate: ≥ 50k rows/sec sustained per connector.
- Stream rate: ≥ 5k events/sec sustained with p99 lag < 2s.
- Lag under 5k events/sec for 30 min: p99 lag < 5s.

Regression > 25% on any benchmark fails CI; spec allows re-budgeting if justified.

### 12.6 Out of v1 testing scope

- Multi-region federation.
- Source DB version matrix beyond Postgres 15+16, MySQL 8.0, Mongo 7.0 (best-effort, documented).
- Encrypted/column-level security beyond `scope` include/exclude.
- Concurrent connector + manual `ALTER TABLE` on the kyma side (documented limitation; v1.5 introduces an exclusive lock).

### 12.7 Test infrastructure

- `kyma-connectors-testkit` — private dev crate housing the property-test query generator, fault injector, chaos test harness.
- CI matrix split — federation property tests + chaos run on a separate job with longer timeout, gated by `run-chaos` label on PRs (always on `main` and nightly).

---

## 13. Rollout

### 13.1 Milestones

Five milestones; each independently shippable.

| Milestone | Ships | Engine | Modes | Gate |
|---|---|---|---|---|
| **M0 — Foundation** | `ExternalSource` trait, `Capabilities`, `kyma-federation` skeleton, catalog migrations, admin API extensions, `SecretStore` resolver, `live(...)` table function, `pushdown_summary` plumbing, testkit | None | None | Unit tests green; existing kyma suite green |
| **M1 — Postgres** | `PostgresSource` (federation + sync + both); planner reaches "best practice" target; replication-slot snapshot; CDC streaming with exactly-once | Postgres 15/16 | all | Integration + property + chaos pass; perf within budget |
| **M2 — MySQL** | `MySqlSource` including collation-safety; reuses M0/M1 | MySQL 8.0 | all | M1 gate + collation property tests |
| **M3 — Mongo** | `MongoSource`; SchemaEvolver integration for nested/polymorphic; change-stream CDC with `startAtOperationTime` | Mongo 7.0 | all | M1 gate + Mongo-specific suite |
| **M4 — Hardening & docs** | Web UI for connector management, `/v1/connectors/:id/events`, formal docs site pages, agent-tool integration over external sources | All | all | Web UI usable end-to-end without curl; agent eval suite passes cross-source Qs |

### 13.2 Gating rules (apply to every milestone)

- No silent regressions in existing kyma surface. Catalog migrations are additive; existing `Connector` trait stays untouched alongside `CdcConnector`.
- No `unsafe` added.
- Property tests gate all pushdown changes.
- Chaos scenarios gate every engine slice (snapshot-crash, streaming-crash, CAS-storm).
- `pushdown_summary` and `kyma_connector_health` ship in M0 and stay always-on.

### 13.3 Cargo feature gating

`kyma-server` exposes a `federation` Cargo feature. M0 wires it; M1+M2+M3 are gated behind it. Drivers (`mongodb`, `mysql_async`, etc.) only compile when enabled.

### 13.4 Effort calibration (rough, not commitment)

| Milestone | Size | Risk |
|---|---|---|
| M0 | 2–3 weeks | Low |
| M1 | 4–6 weeks | Medium |
| M2 | 2–3 weeks | Low–Medium |
| M3 | 4–5 weeks | Medium–High |
| M4 | 2–3 weeks | Low |
| **Total** | **~14–20 weeks** | — |

### 13.5 Out-of-v1 follow-ups (named)

| Lands in | Item |
|---|---|
| v1.5 | JSONB / MySQL-JSON drill-in (recursive inference inside `dynamic`) |
| v1.5 | Typed Arrow `List` for arrays |
| v1.5 | Connector alerts/SLOs in config |
| v1.5 | First-class bytes columns |
| v1.5 | Concurrent-DDL exclusion lock on synced tables |
| v2 | More engines (SQLite, SQL Server, Oracle, DynamoDB, ClickHouse, Snowflake/BigQuery as sources) |
| v2 | Read-time consistency fences |
| v2 | Multi-region federation |
| Post-v1 | Window-function pushdown |
| Post-v1 | Auto-recreate dropped replication slot with operator sign-off |

### 13.6 Cross-cutting commitments enforced by spec

- Every PR adding pushdown-able behavior includes property-test cases proving it.
- Every PR adding a new failure surface includes the integration test exercising it.
- No engine-specific code in `kyma-federation` or `kyma-connectors::cdc`. Caught by code review and a small architectural test (similar to kyma's existing arch tests).
- Schema-mapping table per engine is owned by the engine impl module as data (`const TYPE_MAP: &[(SourceType, KymaType)]`) and re-exported into doc-site rendering. Tables are code, not drifting documentation.

---

## 14. Open questions for M0 spec review

These are small but real and should not be re-litigated mid-implementation. Answer them at M0 spec review:

1. Final shape of `Capabilities`. The struct must be expressive enough for opportunistic same-source join detection without becoming a query language of its own. First draft happens in M0; second-pass review against all three engines before M1 starts.
2. Final shape of `PushedPlan`. Specifically: do we keep `Sql(String)` or move to a structured AST that engines render? `String` is simpler; AST is safer if we ever add SQL Server/Oracle. Recommendation: keep `String` for v1 with a strict-bind discipline; revisit if v2 proves it brittle.
3. Whether `connector_cdc_state.checkpoint` should be opaque JSON or a per-engine typed enum stored as JSON. Recommendation: opaque JSON, with engine-side serde — avoids catalog-schema churn when an engine evolves its checkpoint shape.
