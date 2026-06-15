# kyma — Retrieval & scale subsystems

Developer/operator reference for the retrieval path (vector ANN, BM25 full-text,
hybrid search) and the scale-out machinery (Parquet at rest, staged ingest +
committer) shipped by the scale program. Documents what is **wired and active**
today; the architecture roadmap in `architecture.md` tracks what is foundation
only.

Every index is a **format-agnostic sidecar**: a separate object keyed by extent
id, registered in the catalog, built from `RecordBatch`es. Sidecars work
identically over TLM and Parquet extents and never change the extent bytes.

---

## Index sidecars

`crates/kyma-core/src/index_sidecar.rs` defines the contract:

- `SidecarKind` — `IvfRabitq` (code `"ivf_rabitq"`) and `TantivyFts` (code
  `"tantivy_fts"`).
- `RowAddress { block: BlockId, row: u32 }` — addresses a single row inside an
  extent, stable across format.
- `IndexSidecarDescriptor { id, extent_id, table_id, column, kind, object_path,
  byte_size, params, embedding_model_id, created_at }` — the catalog row.

Sidecars are registered in the **`extent_indexes`** catalog table
(`crates/kyma-catalog/migrations/028_extent_indexes.sql`, mirrored in the SQLite
catalog for local mode). The descriptor's `embedding_model_id` pins the
embedding model generation: query-time code never mixes vectors from different
models.

Sidecar blobs are cached on disk under `KYMA_CACHE_DIR` (default
`~/.kyma/cache`) by `crates/kyma-storage/src/sidecar_cache.rs`.

---

## Vector ANN

Crate `kyma-index-vector` builds a per-extent **IVF + 1-bit RaBitQ** sidecar:

```
extent embedding column
  → L2-normalize + derive RowAddress per vector
  → mini-batch k-means (nlist = clamp(√n_rows, 16, 256))
  → RaBitQ 1-bit quantization (+ correction factors)
  → KYIV sidecar blob (footer + per-partition layout, range-GET friendly)
```

Query side — `kyma_exec::ann::ann_topk` (`crates/kyma-exec/src/ann.rs`):

1. adaptive-threshold extent prune,
2. sidecar probe (`nprobe` partitions),
3. RaBitQ scan to a candidate pool,
4. **exact cosine rerank** of candidates (the brute-force UDF is the recall
   oracle and the rerank kernel — it is never removed).

`ann_topk` is wired into the vector leg of unified search
(`crates/kyma-server/src/search/mod.rs`) and agent memory retrieval
(`crates/kyma-server/src/agent/memory_retrieve.rs`). The SQL/UDF cosine path
remains as the fallback for unindexed extents and as the CI recall oracle.

For server-mode scale-out across many extents, a **global centroid tree**
(SPANN-style, `kyma-index-vector::global_tree` + the `ann_tree` catalog table)
fixes the per-extent fan-out: `ann_topk` routes the query through the tree's
selected partitions (`ann_tree::select`) when a fresh tree exists, and falls
back to per-extent fan-out otherwise — so the tree is a pure latency
optimization, never load-bearing for correctness. Local mode (SQLite catalog)
builds and queries per-extent ANN sidecars identically; the global tree is the
one server-only piece.

---

## Full-text (BM25)

Crate `kyma-index-fts` builds a per-extent **Tantivy** index as a sidecar.
Query side — `kyma_exec::fts::bm25_topk` (`crates/kyma-exec/src/fts.rs`) —
replaces the catalog's coarse `LIKE`/token-set prune for the lexical leg with
real BM25 ranking, and is wired into the same two call sites (unified search +
memory retrieval). The `LIKE` path remains the fallback for unindexed extents.

---

## Hybrid search

Unified search runs the vector leg (`ann_topk`) and the lexical leg
(`bm25_topk`) and fuses them with Reciprocal Rank Fusion (RRF, k=60). Both legs
degrade to their exact/`LIKE` fallbacks when a sidecar is missing, so results
are always correct — only latency differs.

---

## Storage format (Parquet at rest)

`SegmentFormat` is the storage trait boundary. Two implementations ship:

- `kyma-format-tlm` — the original Arrow-IPC-derived format (default).
- `kyma-format-parquet` — ZSTD-compressed Parquet, one row group per appended
  batch.

`FormatRegistry` (`crates/kyma-core/src/segment_format.rs`) holds one **write**
format plus a list of **reader** formats and dispatches reads by sniffing each
extent's magic bytes — so a table can hold a mix of TLM and Parquet extents and
old extents stay readable forever.

| Env | Default | Effect |
|---|---|---|
| `KYMA_WRITE_FORMAT` | `tlm` | `parquet` makes new extents Parquet; TLM stays registered as a reader. |

Compaction is the organic migration vehicle: rewriting an extent re-emits it in
the current write format.

---

## Ingest: synchronous vs staged + committer

Default ingest is synchronous (router commits inline; read-your-writes).
Setting `KYMA_INGEST_MODE=staged` splits the path:

- **Routers** flush a batch to object storage, record its manifest in the
  **`staged_extents`** catalog table, and ack — object storage is the WAL.
- A **committer** (`crates/kyma-ingest-core/src/committer.rs`) drains staged
  rows, groups them by table, and commits each group via
  `Catalog::commit_staged_group` in one transaction (snapshot CAS + staged-row
  delete), so a row becomes visible atomically.

| Env | Default | Effect |
|---|---|---|
| `KYMA_INGEST_MODE` | (unset → synchronous) | `staged` enables the router/committer split. |
| `KYMA_ROLE` | `all_in_one` | Role of this node; only committer-eligible roles run the committer loop. |
| `KYMA_COMMIT_WINDOW_MS` | `1000` | Committer drain interval. |

Local mode keeps the in-process synchronous path (staged mode is a server/PG
feature).

---

## Catalog & query-path config

| Env | Default | Effect |
|---|---|---|
| `KYMA_PG_MAX_CONNS` | `16` | Postgres catalog pool size (min 1). |
| `KYMA_CACHE_DIR` | `~/.kyma/cache` | On-disk sidecar/extent cache root. |
| `KYMA_QUERY_MAX_CONCURRENT` | `0` (unlimited) | Per-process cap on in-flight `/v1/query` + `/v1/search`; over the cap returns `429` + `Retry-After`. |
| `KYMA_QUERY_MAX_CONCURRENT_PER_TENANT` | `0` (unlimited) | Per-tenant concurrency cap — each tenant gets its own semaphore, so one tenant saturating its budget can't starve another. |
| `KYMA_QUERY_RETRY_AFTER_SECS` | `1` | `Retry-After` value sent with the 429. |
| `KYMA_AGENT_MAX_CONCURRENT` / `_PER_TENANT` | `0` (unlimited) | Same, for the heavier agent-run path. |
| `KYMA_INGEST_RATE_RPS` | `0` (unlimited) | Token-bucket ingest rate limit (refill/sec), keyed by database; empty bucket → `429` + `Retry-After`. |
| `KYMA_INGEST_RATE_BURST` | `2×rps` | Ingest token-bucket burst cap. |
| `KYMA_QUOTA_REFRESH_SECS` | `30` | How often the per-tenant quota cache reloads from the `tenant_quotas` catalog table. |

Query/agent admission lives in `crates/kyma-server/src/concurrency.rs` (per-
process and per-tenant semaphores); ingest rate limiting lives in
`crates/kyma-ingest-rest/src/rate_limit.rs` — all wired and off by default.

**Per-tenant configurable quotas.** Beyond the env-global defaults above, an
operator can set DIFFERENT query/agent concurrency limits per tenant via the
`tenant_quotas` catalog table (PG migration 032 + SQLite mirror), managed by an
admin endpoint:

```
PUT /v1/admin/tenant-quotas/<tenant-uuid>   {"max_query_concurrent": 10, "max_agent_concurrent": 2}
GET /v1/admin/tenant-quotas
```

Both require an admin token. A configured value overrides the env default for
that tenant; an unset field (or no row) falls back to the env default. The
limiter reads an in-RAM cache (`quota_cache.rs`) refreshed every
`KYMA_QUOTA_REFRESH_SECS`, so there is no per-request catalog hit; an upsert
also refreshes the cache immediately. Ingest rate limiting stays per-database
(the ingest path carries a database, not a tenant).

---

## Graph writes (Cypher CREATE / MERGE)

`/v1/query` with `Content-Type: application/x-cypher` is write-capable for
`CREATE` and `MERGE`. The graph is a read-only view over append-only tables, so
these clauses map to **appends**; `SET` / `DELETE` / `REMOVE` are rejected by
design (no in-place mutation).

- `parse_cypher_write` (`crates/kyma-kql/src/cypher.rs`) returns `Some(write)`
  for a `CREATE`/`MERGE` statement and `None` for a read query (which falls
  through to the read path).
- `handle_cypher_write` (`crates/kyma-server/src/lib.rs`) enforces the write
  role, then ingests node/edge rows via the `WritePath`. `MERGE` runs an
  existence check first and appends only when absent.

Supported: `CREATE`/`MERGE` of nodes with inline properties and of fully
inline-specified edges (`(a {id})-[:T {p}]->(b {id})`). Both endpoints must
carry an inline `id`. Ack:

```json
{ "created": { "nodes": N, "edges": M }, "merged_existing": K, "request_id": "…" }
```

---

## Test gates

The retrieval/scale work is guarded by the gauntlet
(`scripts/gauntlet.sh --tier=pr|nightly|weekly`) and two fresh-install
acceptance gates:

- **Local single-binary** — `scripts/fresh-install-validate.sh` (SQLite + local
  FS, `cargo build`), wired into CI as `fresh-install.yml`.
- **Docker container** — `scripts/compose-smoke.sh` builds the image from the
  Dockerfile and brings up the full `docker-compose` stack (Postgres + MinIO +
  engine) on an isolated project/ports (`scripts/compose.smoke-override.yml`, so
  it never collides with a running default-named dev stack), then asserts
  health/login/ingest/query/search and tears down. Wired into CI as
  `compose-smoke.yml` (runs on push to main + on demand).
- **Kubernetes / Helm** — `scripts/fresh-install-validate-k8s.sh` spins a
  throwaway `kind` cluster, deploys in-cluster Postgres + MinIO, helm-installs
  the chart (`deploy/helm/kyma-engine`) from the built image, waits for the pod
  rollout, then asserts health/login/ingest/query/search over a port-forward and
  deletes the cluster. Wired into CI as `k8s-smoke.yml` (push to main + on
  demand).

All three fresh-install gates, plus the gauntlet, are wired into CI
(`.github/workflows/gauntlet-*.yml`, `fresh-install.yml`, `compose-smoke.yml`,
`k8s-smoke.yml`). See `benchmarks.md` for the measured numbers and the
deterministic correctness oracles (recall-vs-exact, differential-vs-petgraph).
