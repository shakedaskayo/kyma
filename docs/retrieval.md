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

Local mode (SQLite catalog) builds and queries per-extent ANN sidecars
identically; only the cross-extent global centroid tree is server-only (and is
roadmap, not yet wired — see `architecture.md`).

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
| `KYMA_QUERY_RETRY_AFTER_SECS` | `1` | `Retry-After` value sent with the 429. |

Admission control lives in `crates/kyma-server/src/concurrency.rs` and is wired
into the query/search handlers. (Per-tenant token-bucket quotas are roadmap —
the current limiter is per-process, not per-tenant.)

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
(`scripts/gauntlet.sh --tier=pr|nightly|weekly`) and the fresh-install
acceptance gate (`scripts/fresh-install-validate.sh`), both wired into CI
(`.github/workflows/gauntlet-*.yml`, `.github/workflows/fresh-install.yml`).
See `benchmarks.md` for the measured numbers and the deterministic correctness
oracles (recall-vs-exact, differential-vs-petgraph).
