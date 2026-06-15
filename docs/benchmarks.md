# kyma — Industry benchmark comparison

_Last measured: 2026-04-19. Hardware: 1× macOS laptop, Docker-hosted Postgres + MinIO on loopback. Dev build (`cargo build`, not `--release`). Phase-A format (Arrow IPC wrapper)._

> **Update — Scale & Production-Readiness program (2026-06-15).** The "Phase D" /
> "Phase B follow-on" work referenced throughout sections 1–7 below has since
> **shipped** and supersedes those "future work" notes: a **Parquet** segment
> format (ZSTD + encodings + blooms + page index; `KYMA_WRITE_FORMAT`, per-extent
> dispatch), **per-extent IVF + RaBitQ ANN** + **Tantivy BM25** sidecars + a
> cross-encoder reranker (so "vector search = stub" is obsolete), **staged
> ingest** (router → object store → lease-elected committer), the **Helm role
> split** (edge / committer / worker), a **CSR graph engine** with a full Cypher
> dialect (read + CREATE/MERGE writes), and per-tenant **rate-limit / query-admission**
> ops hardening. The section-1 ingest numbers below predate this and are **not**
> re-measured here. Release-tier throughput/latency at 100M-row / 1B-edge /
> multi-node / soak scale is pending representative infrastructure; the
> deterministic **correctness** gates (ANN-recall-vs-oracle, graph
> differential-vs-petgraph) and the CI-tier **graph-traversal** perf below were
> validated in-session. **Multi-node read scale-out (S2.4) correctness was
> re-validated in-session on an isolated throwaway stack: two stateless query
> nodes over one shared catalog + object store, 20 split ingests → both nodes
> agree `COUNT(*)=100` / `DISTINCT req_id=20` (no loss, no duplication).**
>
> **Graph traversal engine (S3.1/S3.2) — `kyma-graph-bench`, CI tier, dev laptop
> under load (orders of magnitude, not absolute ceilings):**
>
> | Operation | 10k edges | 100k edges |
> |---|---|---|
> | CSR build (power-law) | ~5 ms | ~47 ms |
> | k-hop (k=3) forward | ~5 µs | ~2.5 µs |
> | k-hop (k=3) backward (hub fan-in, worst case) | ~4 ms | ~35 ms |
> | unweighted shortest-path | ~1.2 µs | ~1.2 µs |
>
> Forward traversal + shortest-path are microseconds and scale-independent
> (bounded fan-out from the seed over the immutable CSR); only worst-case
> backward hub fan-in grows with graph size. Correctness is gated against a
> petgraph oracle (`kyma-graph-testkit` / `kyma-graph-topo`, all green).

Our stated goal (plan §1) is to beat existing unified-observability solutions on a real production workload. This doc quantifies where we are **today**, compares honestly to published numbers for the closest industry peers, and lists the specific engineering work that closes each remaining gap.

---

## 1. Where we are today

Measured via `scripts/load-test.sh` and ad-hoc one-shot requests.

| Metric | Value | Notes |
|---|---|---|
| **Sustained ingest** (8 workers × 500 rows/batch) | **66,450 rows/sec** | 10s window, 0 errors, p99 = 52 ms |
| **Single-request throughput** (5 000-row batch) | **~200,000 rows/sec** | One-shot via curl, 25 ms end-to-end |
| **Query latency, small result** | 1–3 ms | `SELECT COUNT(*)`, 3 rows |
| **Query latency, 50k-row scan** | ~150 ms | cold cache, full table |
| **Time-range pushdown** | **10×–N× fewer extents touched** | verified by `test-pushdown.sh`, 10 extents → 1 extent scanned |
| **Crash durability** | 100% (2 hard kills tested) | `test-chaos-test.sh` — zero data loss or corruption |
| **Cluster-node consistency** | Zero loss at 100 concurrent ingests across 2 nodes | `test-a-two-nodes.sh` — 207 CAS conflicts resolved correctly |
| **Build** | `cargo build` dev profile | release profile + LTO typically +30–50% throughput |

### What's bottlenecked right now

The **66K rows/sec** ceiling is dominated by three things:

1. **One extent per HTTP request.** Every `POST /v1/ingest` writes a new extent + commits a new snapshot. Postgres CAS is the serialization point. High-throughput systems batch N requests into one extent and amortize the commit — we will too (Phase B-follow-on: `kyma-ingest-core::StagingBuffer`).
2. **Phase-A Arrow IPC format.** Written with default Arrow IPC compression; no per-type encoders (Gorilla floats, delta-of-delta timestamps, dictionary-coded strings). Real format lands in **Phase D**.
3. **Dev build, no LTO, single machine, all over loopback.** Representative for relative comparisons but _not_ an absolute ceiling.

---

## 2. Industry numbers — the honest field

Numbers below are **per-node, sustained** per each vendor's published documentation, blog posts, or benchmark suites. Workloads differ; row size, schema, and indexing matter a lot. Use for orientation, not apples-to-apples.

| System | Published sustained ingest (1 node) | Query on indexed field (1 TB, p99) | Source |
|---|---|---|---|
| **Loki** (Grafana) | 50–150 MB/s (~500k–1M lines/s) | ~5 s on "LogQL filter" over 1 TB | Grafana docs + Loki performance guide |
| **ClickHouse** | 500k–1M rows/s/core, 5M+ rows/s/node with big batches | 100 ms – 1 s depending on MV | ClickHouse docs, Altinity benchmarks |
| **InfluxDB 3.0** (IOx, Arrow/Parquet/DF-based) | ~1M points/s per writer | sub-s on time-range + tag filter | InfluxData engineering blog 2023–2024 |
| **Elasticsearch** | 10–50k docs/s/node (1 primary shard) | 50–500 ms "grep through logs" | Elastic Co benchmarks, community reports |
| **Splunk** | 30–100k events/s per indexer | 1–10 s over 1 TB | Splunk sizing docs |
| **Azure Data Explorer** | 100–500k events/s per ingest node, bursty to 5 M/s | sub-s over 1 B rows with inverted indices | Microsoft ADX perf docs |
| **Parquet + DataFusion** (self-hosted) | 500k–2M rows/s bulk-load, limited by Parquet encoder | 100 ms – few s, depends heavily on RG size + filters | DataFusion benchmarks, `datafusion-benchmarks` repo |
| **kyma today** (phase A) | **66 k rows/s sustained, 200 k peak** | 1–150 ms small-to-50k | this doc |

### What this means

- **Against Elasticsearch and Splunk, we are already competitive** on ingest (we hit higher rows/sec than ES's typical node at single-digit workers).
- **Against Loki we are in the same ballpark** on ingest; their number includes log-line parsing + label-indexing overhead we don't have yet, so at equivalent feature parity we should be similar.
- **Against ClickHouse, InfluxDB 3.0, and ADX we are ~10× behind on raw ingest throughput** — those systems batch aggressively and have been years in custom-format work. This is the gap Phase D + B-follow-on closes.
- **On durability/correctness** we are equal or ahead — `Test A` concurrency, catalog-CAS snapshot isolation, and hard-kill survival are all demonstrated.

---

## 3. Where we're architecturally ahead (today, already)

These are real advantages, not aspirations.

| Dimension | kyma | Typical competitor |
|---|---|---|
| **Unified** (logs + metrics + traces + wide-table + vector-memory in one engine) | **Yes** by design (pluggable `SegmentFormat` trait) | Most orgs run 4–6 separate systems (Loki + Prom + Tempo + ES + …) |
| **Open object storage** (MinIO/S3) with pluggable format | **Yes** (Iceberg-mirroring catalog; format swap is a trait impl) | Proprietary file formats (Splunk, ADX); tight coupling to one FS (Elastic shard files) |
| **Ingest idempotency** | `X-Idempotency-Key` + 24h TTL ledger | Splunk has HEC idempotency; most competitors don't natively |
| **Multi-node CAS** | Demonstrated: 207 conflicts retried correctly under 100-way contention | Often requires leader election (Elastic) or serialization (ADX ingest node) |
| **Per-request budget enforcement** | Wall-clock + memory budget per query, 429 + Retry-After | Elastic circuit breakers, ClickHouse quotas — similar capability |
| **Crash-safety** | Hard-kill → no loss, snapshot chain intact, no rewinds | ES has at-least-once durability; Splunk has similar; Loki depends on WAL config |
| **Schema evolution without rewrite** | ALTER TABLE via immutable schema snapshots + read-time promotion | Iceberg/Delta: yes. ES: forced remapping. ADX: yes but column-level. |

---

## 4. Where we are behind — and what closes each gap

| Gap | Industry best | Current | Closes with | Expected impact |
|---|---|---|---|---|
| **Sustained ingest: 10×** | 500k–1M rows/s (ClickHouse, ADX) | 66k rows/s | **Staging buffer + batched commit** in `kyma-ingest-core`: accumulate N requests into one extent, one snapshot commit per flush. Single change, weeks of work. | **5–10×** ingest throughput. Closes most of the gap. |
| **Query latency: needle-in-haystack** | sub-100 ms (ADX + inverted index on 1 TB) | 150 ms on 50 k rows no index | **Inverted index** on text columns + **per-block min/max** (Phase D). | **100×+** on text filters; 10× on any column-bounded predicate. |
| **Storage efficiency** | Parquet+ZSTD ≈ 5–10× compression on logs | Arrow IPC ≈ 2–3× | **Per-type encoders** (Gorilla floats, delta timestamps, dictionary strings, LZ4→ZSTD at compaction) in Phase D format. | Disk + I/O 2–3×. |
| **High-cardinality aggregation** | ClickHouse: native `count-distinct` sketches | Uses DF default HashAggregate | **HyperLogLog** integration + DF UDAF registration. Couple days. | ~10× on `dcount` at high cardinality. |
| **Vector / semantic search** (for agent memory) | Qdrant: sub-10 ms on 1 M vectors with HNSW | Stub format, no real impl | **Real `VectorFormat`** impl with HNSW. Phase D-follow-on, ~2 months. | Closes the agent-memory promise. |
| **Load-distribution** | ADX auto-splits via cluster mgmt | Single-node per cluster (slice 1) | **Slice 2** of plan: multi-node read scale-out (already architected for — `node_id` per-scan, catalog-CAS, gRPC-in-process). | Linear scaling with nodes added. |
| **Arrow Flight gRPC** | InfluxDB 3.0, DuckDB all support | HTTP-NDJSON only | **gRPC + Arrow Flight** query surface. Hours-to-days to stand up. | 2–5× on client-side serialization; enables zero-copy clients. |

### Batched ingest — landed 2026-04-19

Implemented as `kyma_ingest_core::StagingBuffer` with pipelined per-table flushes. Workload-shape matters more than I expected; here's what actually happens:

| Workload shape | Staging OFF | Staging ON | Extent count ratio |
|---|---|---|---|
| 500 rows × 8 workers, 10s | **69,200 rows/s**, p99=52ms, 1370 extents | 41,800 rows/s, p99=100ms, 139 extents | **~10× fewer** extents |
| 500 rows × 16 workers (batchy) | ~69k rows/s (MinIO-PUT-bound) | 37,600 rows/s, p99=280ms, **33 extents** | **41× fewer** extents |
| 50 rows × 32 workers, high-concurrency | 8,535 rows/s, **p99=2,015ms**, 1693 extents, 1 error | 9,915 rows/s, **p99=185ms**, 150 extents, 0 errors | 11× fewer + **11× lower p99** |

The **real** wins are the ones we cared about, just not the ones the raw rows/sec number captures:

1. **10–40× fewer extents.** Catalog is 10–40× less loaded; small-file problem disappears; queries that don't use time-range pushdown speed up proportionally.
2. **10× lower p99 under high concurrency.** 2015ms → 185ms at 32 concurrent workers. This is the number that matters for user-facing SLOs.
3. **Stability.** Direct mode showed errors under 32-way contention; staging shows zero.
4. **Lower Postgres load.** 10–40× fewer CAS commits per unit of work.

The **raw rows/sec** improvement only materializes at workload shapes we can't easily drive from a bash script — hundreds-to-thousands of concurrent clients with small per-request payloads (the real log-shipper / OTLP-collector scenario). The bash benchmark is MinIO-PUT-bound at low concurrency, so direct mode's 8-way parallel upload beats batched on rows/sec in that regime.

Future tuning knobs (env vars):
- `KYMA_FLUSH_MAX_ROWS` (default 8000) — flush when buffer reaches N rows
- `KYMA_FLUSH_MAX_BYTES` (default 16 MiB)
- `KYMA_FLUSH_MAX_AGE_MS` (default 50) — safety valve for latency

`KYMA_STAGING_DISABLED=1` restores one-extent-per-request behaviour for comparison tests.

### Remaining lever for raw rows/sec — landed 2026-04-19

Implemented as `kyma_ingest_core::CommitCoordinator`. **Group commit squared:** concurrent staging flushes no longer compete on `tables.current_snapshot_id`; they hand their extent manifests to a central coordinator that bundles arrivals within a short window (default 5 ms) into a single snapshot row containing many extents.

Measured after coordinator landed (same hardware, dev build):

| Shape | Baseline (direct) | Staging alone | **Staging + Coordinator** | Extents ratio |
|---|---|---|---|---|
| 500 × 16 (MinIO-saturated) | ~69k rows/s, 1300+ extents | 37k rows/s, 160 extents | **70.4k rows/s, 160 extents** | **8.5× fewer** |
| 500 × 32 + tuned | — | — | **69.4k rows/s, 66 extents** | **21× fewer** |
| 100 × 64 | ~20k rows/s | 20k rows/s | **20.5k rows/s, 76 extents** | **20× fewer** |
| 50 × 32 | 8.5k, 1693 extents, 1 err | 9.9k, 150 extents | 7-9k, **31 extents** | **55× fewer** |

What the coordinator did:
- **500 × 16 case (most important):** matched the direct-mode raw throughput ceiling *while* producing 8.5× fewer extents. This is "no regression on throughput, structural win on storage."
- **500 × 32 case:** 21× fewer extents at the same throughput.
- **50 × 32 case:** 55× fewer extents — each extent now carries ~50 rows from ~50 ingest requests, vs. one request per extent before.

The per-table CAS is no longer the bottleneck. The new bottleneck is **MinIO-PUT parallelism** — each flush still produces one extent (one object), and each object still takes one HTTP PUT. Concurrent uploads saturate MinIO at ~16-32 parallel on loopback.

### Next lever: combined extents at the staging layer

Rather than "one flush = one extent," combine contents of multiple triggered flushes into fewer, larger extents. That reduces object count further and uses MinIO's per-request overhead more efficiently. Also meaningfully reduces query-time I/O because fewer footers to fetch. Estimated ~2× on hot-table throughput + 5-10× fewer objects. Phase D work.

## Query-side wins

### Equality-index pushdown — landed 2026-04-20

Per-extent distinct-value sets for string / int32 / int64 columns (cap 1000/column). Stored in each extent's `column_stats` JSONB. The catalog prunes extents that can't contain any matching value before any object-store I/O fires.

Measured on 10 extents, one region each (`WHERE region = 'X'` should scan exactly 1):

| Query shape | Extents listed | Ratio vs unfiltered |
|---|---|---|
| `SELECT COUNT(*) FROM events` | 10 | baseline |
| `WHERE region = 'us-east'` | **1** | **10× less I/O** |
| `WHERE shard = 7` (int) | **1** | **10× less I/O** |
| `WHERE region IN ('us-east','eu-west')` | **2** | **5× less I/O** |
| `WHERE region = 'moon'` (no match) | **0** | **∞× less I/O** |
| `WHERE region = 'us-east' AND timestamp BETWEEN …` | 1 | equality + time-range compose |

Falls back gracefully when the distinct set overflows the cap: column_stats records `null`, catalog returns all extents for that column, and DataFusion filters rows above the scan as before.

### What this means for real workloads

A typical telemetry query looks like `WHERE service = 'X' AND timestamp > ago(1h)`. Previously we pruned extents only by time range; now we also prune by service (and any other categorical column with bounded cardinality). Real gain depends on cardinality:
- 50 services × 1 hour window on 30-day-retention data: **~50× less I/O** vs time-range alone.
- High-cardinality (`user_id`) columns: pruning kicks in up to 1000 distinct/extent, then degrades to time-range-only.

### Text-search (word) index — landed 2026-04-20

Per-extent word-level token sets for string columns (cap 10k tokens/column, overflows to `null`). Stored in `column_stats.{col}.tokens` as sorted JSONB arrays.

Tokenization matches on both the writer + reader sides: lowercase, split on non-alphanumeric ASCII, keep tokens ≥ 2 chars. `WHERE col LIKE '%word%'` and KQL `col contains "word"` both route through the same extractor, tokenize the needle the same way, emit `ColumnPrune::ContainsTokens(Vec<String>)`, and the catalog runs `tokens @> ["word"]` JSONB containment.

Measured (10 extents, each tagged with a unique word):

| Query | Extents scanned | Ratio |
|---|---|---|
| `LIKE '%alpha%'` (word in 1 extent) | **1** | **10×** |
| `LIKE '%xylophone%'` (no match) | **0** | **∞×** |
| KQL `message contains "gamma"` | **1** | **10×** |
| `LIKE '%retry%'` (word in all 10) | 10 | baseline (correct) |
| `LIKE '%alpha%retry%'` (multi-token, only 1 extent has both) | **1** | **10×** |

Multi-token phrases prune to the intersection — an extent must contain **all** tokens in the needle to be scanned. Row-level filtering still happens in DataFusion above the scan (so semantic correctness of phrase search is preserved).

**Substring matching inside a token is over-pruned at the extent level.** Query `LIKE '%oom%'` will miss extents that only contain the word `OutOfMemoryError` — because "oom" is not a whole word in our tokenizer. This is the same trade-off word-level indexes in ES / Splunk / ADX make; users who need substring match use `contains_cs` or regex, and those fall back to full scan. N-gram indexing is the opt-in answer (future task).

### Combined: equality + time-range + text-search prune compose

All three prune mechanisms AND together at the catalog level. A real log query like:
```sql
WHERE service = 'api-gateway'
  AND timestamp BETWEEN '2026-04-20 10:00' AND '2026-04-20 10:05'
  AND message LIKE '%OutOfMemoryError%'
```
would (on a 30-day retention table with 100 services) prune to roughly **1 extent** — the one with `api-gateway` + the 5-minute window + the OOM token — out of potentially **thousands**. That is the "much better than market" claim made real.

### Environment variables

| Var | Default | Effect |
|---|---|---|
| `KYMA_STAGING_DISABLED` | unset (staging on) | Skip staging; one extent per request |
| `KYMA_FLUSH_MAX_ROWS` | 8 000 | Flush triggers when buffer hits this |
| `KYMA_FLUSH_MAX_BYTES` | 16 MiB | Flush triggers when buffer hits this |
| `KYMA_FLUSH_MAX_AGE_MS` | 50 | Force flush if oldest waiter is older |
| `KYMA_COMMIT_WINDOW_MS` | 5 | Coordinator batch-arrival window |
| `KYMA_COMMIT_MAX_EXTENTS` | 128 | Max extents bundled per snapshot |

---

## 5. Concrete "path to best-in-class" — 6 months of work to be competitive

In rough order of impact/effort:

1. **Staging buffer + batched ingest** (Phase B follow-on, 2–3 weeks). +5–10× ingest.
2. **Per-block min/max + bloom per extent** (Phase D, 2–3 weeks). 2–10× query speedup on filtered scans.
3. **Inverted index on text columns** (Phase D, 4–6 weeks — the single biggest engineering piece). 100×+ on needle-in-haystack log search, which is where Loki/ES/Splunk make their money.
4. **Per-type column encoders** — Gorilla floats + delta-of-delta timestamps + dictionary strings + ZSTD at compaction (Phase D, 3–4 weeks). 2–3× storage + proportional scan reduction.
5. **Dynamic column (CBOR + path-presence bitmap)** (Phase D, 3–4 weeks). Unblocks "any data" workloads without schema migration.
6. **Arrow Flight gRPC query surface** (Phase E, 1 week). 2–5× client-side query throughput.
7. **Multi-node read scale-out** (Slice 2, 3–6 months). Linear scaling with node count.

After items 1–4 land, we expect to be:
- **Comparable to ClickHouse** on bulk ingest
- **Comparable to ADX** on needle-in-haystack log search
- **Ahead of everyone** on the unified (logs+metrics+traces+vector) story
- **Ahead of everyone** on open storage + schema evolution

---

## 6. How to reproduce the measurements

```bash
# 1. Stack up.
docker-compose up -d

# 2. Build (ideally `--release`).
cargo build --release --bin kyma --bin kyma-cli

# 3. Load test.
LOAD_DURATION_SECS=30 LOAD_PARALLELISM=8 LOAD_ROWS_PER_REQ=500 ./scripts/load-test.sh

# 4. Chaos test.
./scripts/chaos-test.sh

# 5. Pushdown benchmark.
./scripts/test-pushdown.sh  # reports extents scanned with/without filter

# 6. Full regression sweep.
for t in e2e-test test-a-two-nodes test-compaction test-retention \
         test-alter-table test-auth test-kql test-pushdown chaos-test; do
    ./scripts/$t.sh || echo "FAILED: $t"
done
```

Raw metrics (Prometheus-format) are on `GET /metrics`. Key series:
- `kyma_ingest_rows_total`, `kyma_ingest_bytes_total`, `kyma_ingest_duration_seconds`
- `kyma_query_duration_seconds`, `kyma_scan_extents_listed_total`
- `kyma_catalog_cas_conflicts_total`, `kyma_http_errors_total`
- `kyma_compaction_tasks_total`, `kyma_retention_extents_soft_deleted_total`

---

## 7. Honest summary

- **Today**, on a dev-build laptop, we are **competitive with Elasticsearch-class systems** on ingest and **ahead on unification, open-storage, and correctness guarantees**.
- We are **10× behind ClickHouse/ADX/InfluxDB 3.0** on raw ingest throughput — that gap is closed by a single well-understood change (batched ingest) and a multi-month format effort (Phase D).
- We are not yet making the "much more efficient than the market" claim real on raw numbers — the architecture is positioned for it, the implementation isn't there yet. **Phase D is what makes the claim real.**

The plan closes every identified gap with specific, scoped work items, not hand-waving.
