# pensieve — Architecture (living document)

This document is the source of truth for the engine's architecture. The
companion **slice-1 implementation plan** lives at
`~/.claude/plans/reverse-engineer-end-to-glistening-chipmunk.md` and is mirrored
in this repository's commit history as the design evolves.

---

## Five non-negotiable invariants

These five are encoded in CI via the architectural tests in
`benches/distribution/`. Regressions block merge.

1. **Object storage is the only source of truth.** Local disk is cache, never
   master. Losing every compute node means zero data loss.
2. **Query nodes are stateless.** A node's state is its config + its cache.
   Any durable state lives in the catalog or on object storage.
3. **Catalog is externalized from byte one.** Single-node dev still runs
   Postgres in a separate process. There is no embedded-catalog code path to
   delete later.
4. **Format is pluggable.** `SegmentFormat` is the trait boundary between
   storage and everything else. `pensieve-format-tlm` is one implementation of
   many possible formats.
5. **Parser is pluggable.** `QueryFrontend` is the trait boundary for query
   languages. `pensieve-kql` is one frontend; SQL / PromQL / custom DSLs layer on
   as peers.

Violating any of these = distribution becomes a rewrite.

---

## Workspace layout

See the top-level `README.md` for a concise map. Each crate's own `lib.rs`
documents its responsibility and the trait surface it implements or consumes.

---

## Three external dependencies (slice 1)

- **Postgres** — the catalog. Pluggable behind `Catalog` trait.
- **MinIO / S3-compatible object storage** — the source of truth. Pluggable
  behind `object_store::ObjectStore`.
- **Apache DataFusion** — the query execution runtime. Isolated from the rest
  of the engine by `pensieve-exec::df_adapter`, so DataFusion version churn touches
  one file.

All three replaceable behind traits. Swapping catalogs (to FoundationDB /
Raft) or execution runtimes is a bounded change, not a rewrite.

---

## The three-level pruning cascade (where "blazing" lives)

```
         Incoming query
              |
              v
   +----------------------+
   | 1. Catalog pruning   |  Postgres query using manifest stats:
   |                      |  time range, min/max per column, present_paths
   +----------+-----------+  -> Candidate extents (often 99%+ eliminated)
              |
              v
   +----------------------+
   | 2. Extent pruning    |  Range-GET extent footers (cached):
   |                      |  bloom filters, per-column stats, path bitmaps
   +----------+-----------+  -> Candidate blocks
              |
              v
   +----------------------+
   | 3. Block / index     |  For each candidate block:
   |    pruning           |  - Block footer (min/max) confirms
   +----------+-----------+  - Inverted index posting list for text predicates
              |              -> Exact rows to decode
              v
   +----------------------+
   | 4. Decode & execute  |  Arrow RecordBatches -> DataFusion pipeline
   +----------------------+
```

---

## Distribution readiness (slice 1 affordances)

1. Node identity + heartbeat — `nodes` catalog table.
2. All internal communication is gRPC, even in-process — loopback now, remote
   endpoints later, zero call-site changes.
3. Work-unit abstraction — every background task is a row in a catalog table
   pulled with `FOR UPDATE SKIP LOCKED`.
4. Ingest partitioning stub — `IngestRouter` trait has `LocalRouter` today,
   `ConsistentHashRouter` tomorrow.
5. Query fan-out structure — planner emits per-extent scans with
   `node=local` today, remote assignment tomorrow.

---

## Self-telemetry

Pensieve instruments itself via OpenTelemetry and writes traces into its own store.

### OTLP receiver signals

| Signal  | Table               | gRPC method                       |
|---------|---------------------|-----------------------------------|
| Logs    | `otel.otel_logs`    | `opentelemetry.proto.collector.logs.v1.LogsService/Export` |
| Traces  | `otel.otel_traces`  | `opentelemetry.proto.collector.trace.v1.TraceService/Export` |

Both listeners share port 4317 (configurable via `PENSIEVE_OTLP_ADDR`; disabled by default — set to `0.0.0.0:4317` to enable).

### `otel.otel_traces` schema

| Column           | Type                        | Notes |
|------------------|-----------------------------|-------|
| `start_time`     | Timestamp(ns)               | span start |
| `end_time`       | Timestamp(ns)               | span end |
| `duration_ns`    | Int64                       | `end_time - start_time` |
| `trace_id`       | Utf8                        | hex-encoded |
| `span_id`        | Utf8                        | hex-encoded |
| `parent_span_id` | Utf8 (nullable)             | null for root spans |
| `name`           | Utf8                        | span name |
| `kind`           | Utf8                        | SERVER / CLIENT / INTERNAL / … |
| `status_code`    | Utf8                        | OK / ERROR / UNSET |
| `status_message` | Utf8 (nullable)             | error description |
| `service_name`   | Utf8 (nullable)             | promoted from `service.name` resource attr |
| `subject`        | Utf8 (nullable)             | promoted from `pensieve.subject` span attr |
| `tenant`         | Utf8 (nullable)             | promoted from `pensieve.tenant` span attr |
| `attributes_json`| Utf8                        | remaining span attributes as JSON |
| `resource_json`  | Utf8                        | remaining resource attributes as JSON |

### Self-instrumentation (`pensieve_telemetry` target)

The server instruments its own API operations by emitting tracing spans with `target: "pensieve_telemetry"`. A layered tracing registry filters the OTel exporter layer to this target only — spans outside `pensieve_telemetry` go to the log formatter but not to storage, preventing recursion.

Instrumented operations:

| Span name        | Handler                  | Key attributes |
|------------------|--------------------------|----------------|
| `request`        | auth middleware          | `http.method`, `http.route`, `pensieve.tenant`, `pensieve.subject`, `http.status` |
| `memory.recall`  | memory query handler     | `memory.query`, `memory.results`, `memory.took_ms` |
| `memory.import`  | memory import handler    | `memory.imported` |
| `memory.export`  | memory export handler    | `memory.exported` |
| `agent.query`    | agent ask handler        | `agent.question` |
| `ingest.batch`   | REST ingest handler      | `ingest.table`, `ingest.rows` |

The `SelfTraceExporter` is an in-process `opentelemetry_sdk::SpanExporter`. It starts unwired (drops all spans) and is connected to storage via `OnceLock` once the server's `WritePath` is ready. There is no loopback gRPC — spans go directly to `WritePath::ingest`.

Spans are excluded from self-tracing for: `GET /health`, `GET /metrics*`, and `GET /v1/explore/live*` (long-lived WebSocket).

### Capture-health monitoring

The Claude Code integration hooks write `~/.pensieve/capture-health.json` on every failed ingest attempt (and delete it on success). `pensieve status` reads this file and surfaces capture failures alongside the auth probe result, so a silent 401 streak no longer goes undetected.

---

## Slice roadmap

| Slice | Scope | Status |
|---|---|---|
| 1 | Single-node, distribution-ready affordances | shipped |
| 2 | Read scale-out (rendezvous read router, sidecar caching) | partial — see scale program |
| 3 | Ingest scale-out (staged ingest + committer split) | partial — see scale program |
| 4 | Multi-region / multi-cluster federation | future |

---

## Scale program status

The scale program (S-series) evolves the engine toward petabyte scale as trait
impls and sidecars, never a rewrite. `retrieval.md` is the developer reference
for the retrieval + scale subsystems and their config. Honest status:

**Wired and active**

- **Vector ANN** — per-extent IVF + 1-bit RaBitQ sidecars (`pensieve-index-vector`,
  `pensieve_exec::ann::ann_topk`), wired into unified search + agent memory
  retrieval, with exact-cosine rerank and the brute-force UDF as fallback/oracle.
- **Global centroid tree (SPANN, server mode)** — `pensieve-index-vector::global_tree`
  + the `ann_tree` catalog table (migration 030); `ann_topk` routes through the
  tree (`ann_tree::select`) when a fresh one exists and falls back to per-extent
  fan-out otherwise, so it is a pure latency optimization, never load-bearing.
- **BM25 full-text** — per-extent Tantivy sidecars (`pensieve-index-fts`,
  `pensieve_exec::fts::bm25_topk`) replacing the `LIKE`/token-set lexical leg.
- **Index sidecar contract** — `index_sidecar.rs` + `extent_indexes` catalog
  table (+ SQLite mirror); format-agnostic, embedding-model-pinned.
- **Parquet at rest** — `pensieve-format-parquet` + `FormatRegistry` magic-byte
  read dispatch; `PENSIEVE_WRITE_FORMAT` selects the write format; mixed-format
  tables stay readable.
- **Staged ingest + committer** — `PENSIEVE_INGEST_MODE=staged` + `staged_extents`
  + `committer.rs`; object storage as the WAL, atomic group commit.
- **Graph traversal** — `pensieve-graph-topo` CSR is used in the live retrieval
  path; the Cypher dialect (`pensieve-kql`) supports multi-hop chains,
  variable-length `-[*M..N]->`, and `shortestPath`, plus `CREATE`/`MERGE`
  writes (write-role gated). The memory-expansion default is capped at
  `MAX_HOPS = 2` — a product choice for agent recall, not an engine limit.
- **Ops hardening** — per-process **and** per-tenant query/agent concurrency
  isolation (`concurrency.rs`, each tenant its own semaphore) + token-bucket
  **ingest** rate limiting (`pensieve-ingest-rest::rate_limit`), all with `429` +
  `Retry-After`. Off by default (env-gated). Per-tenant limits are also
  **catalog-configurable** via the `tenant_quotas` table (migration 032 +
  SQLite mirror) and the admin `PUT/GET /v1/admin/tenant-quotas` endpoint, read
  through an in-RAM cache (`quota_cache.rs`) — a configured value overrides the
  env default for that tenant; unset falls back to the env default.

**Remaining (foundation present; would extend, not block, the above)**

- **Persisted graph snapshots** — the CSR is built from a table scan rather
  than persisted as a `graph_snapshots` artifact with an incremental refresh
  job; this is a build-latency optimization, not a correctness gap.

**Infra-gated (validated by design + small-scale tests; full numbers require
representative hardware)** — 100M/1B-scale ANN + graph tiers, multi-machine
cluster failover, 1-hour soak, and LLM-judged memory benchmarks run from
`scripts/` on real infra, not in CI.

Each subsequent slice gets its own plan once the previous one ships.
