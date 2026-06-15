---
title: Slice roadmap
description: The four-slice plan. Slice 1 ships single-node with distribution-ready affordances; Slices 2–4 layer read scale-out, ingest scale-out, and multi-region federation as bounded changes — not rewrites.
---

# Slice roadmap

kyma ships in four slices. Slice 1 is the engine that runs today: one
binary, one Postgres, one bucket, with the trait surfaces and catalog
shapes that distribution will eventually use already in place.
Slices 2, 3, and 4 add read scale-out, ingest scale-out, and
multi-region federation — each as a bounded project against the
existing trait boundaries, not a rewrite.

The plan is committed direction. Every affordance Slice 2+ depends on
exists in Slice 1, listed below. The reason the
[five invariants](/concepts/the-five-invariants) are non-negotiable
is precisely this: violating one in Slice 1 makes the corresponding
later slice a rewrite instead of a deployment change.

## Slice 1 — single-node, distribution-ready

**Status:** shipped. This is what runs today.

**Scope.** One binary, one Postgres catalog, one S3-compatible bucket.
End-to-end ingest through REST, OTLP, Kafka, and file-drop frontends.
Query in KQL, SQL, or PromQL over Arrow Flight. The full
[three-level pruning cascade](/concepts/the-pruning-cascade) is
active. Compaction and per-table retention run as background work
units. No clustering, no fan-out, no remote nodes.

**Distribution-ready affordances already in Slice 1.**

1. **Node identity and heartbeat.** The catalog has a `nodes` table.
   The single live node writes a heartbeat row.
2. **gRPC for all internal communication.** Even in-process call
   sites speak gRPC over loopback. Adding remote endpoints in Slice 2
   does not touch call sites.
3. **Work-unit abstraction.** Every background task — compaction,
   retention, GC, file-drop scans — is a row in a catalog table
   pulled with `FOR UPDATE SKIP LOCKED`. Adding a worker is starting
   another binary.
4. **Ingest-router trait.** `IngestRouter` has `LocalRouter` today;
   `ConsistentHashRouter` is the Slice 3 swap.
5. **Query fan-out structure.** The planner already emits per-extent
   scans tagged `node=local`. Slice 2 changes the tag, not the shape.

**What's still being decided in Slice 1.** Compaction policy
(specifically: target extent size and merge-tree fan-in) tunes
against the benchmark suite as it grows. A denser **Parquet** segment
format has since landed as a second `SegmentFormat` alongside the
Arrow-IPC TLM format — see [Storage format](/architecture/storage-format) —
selected with `KYMA_WRITE_FORMAT` and dispatched per extent, so the
trait contract and the catalog shape are unchanged.

## Slice 2 — read scale-out

**Status:** partially landed. Stateless query nodes ship today via the
[Helm role split](/deploy/helm#scaling-out-the-role-split) (`KYMA_ROLE=edge`)
behind one catalog and bucket, with snapshot/footer caching. The remaining work
is cross-node *partial-plan* fan-out (scan + partial-aggregate below the
exchange boundary) and its multi-node validation at scale.

**Scope.** Multiple stateless query nodes behind a single catalog
and bucket. The planner assigns per-extent scans across the live
node set; partial results stream back over Arrow Flight and merge at
the coordinator. Cache locality becomes a planner hint, never a
correctness requirement (Invariant 2).

**What Slice 1 introduced for it.**

- The `nodes` heartbeat table is the live-node membership view the
  Slice 2 planner reads.
- gRPC-everywhere means remote scan dispatch is "set the endpoint";
  no new call sites.
- The per-extent scan shape — `(extent_id, byte_range, projection,
  predicate)` — already passes through the planner. Slice 2 adds the
  `node` assignment.
- Stateless compute is structurally true: a node is its config plus
  its caches. Adding nodes is starting a binary.

**Blockers and decisions ahead.**

- **Work-stealing vs. static assignment.** The work-unit pattern from
  Slice 1 generalizes; the question is whether the planner pre-assigns
  each scan or workers pull from a shared queue. The benchmark suite
  drives the call.
- **Footer cache coherence.** Each node range-GETs and caches extent
  footers. Compaction invalidates them. Slice 2 needs an invalidation
  signal — a catalog watermark per table is the leading design.
- **Result merge.** Streaming partial Flight results through a
  merge-aware exec node is implemented for single-node already; the
  Slice 2 work is making it tolerate node loss mid-query.

## Slice 3 — ingest scale-out

**Status:** partially landed. Staged ingest ships today — set
`KYMA_INGEST_MODE=staged` and any node stages writes to object storage and acks
immediately, while a lease-elected **committer** drains the staged extents into
the catalog in one transaction (deployed as a dedicated role via the
[Helm role split](/deploy/helm#scaling-out-the-role-split)). The remaining work
is the multi-node consistent-hash router below and its at-scale validation.

**Scope.** Multiple ingest nodes sharing the same catalog and bucket.
A consistent-hash router maps each incoming write to the node
holding the relevant table's staging buffer. Group-commit still
happens per (table, node); CAS still serializes the catalog publish.
Per-table write ordering is preserved without serializing across
tables.

**What Slice 1 introduced for it.**

- `IngestRouter` is a trait. `LocalRouter` is the Slice 1 impl;
  `ConsistentHashRouter` slots in as a peer in Slice 3.
- Idempotency is already at the right layer — REST keys, file-drop
  SHA256, Kafka offsets. Replays survive node churn because the
  catalog is the dedup boundary, not any one node.
- The staging buffer and commit coordinator are per-table, not
  global. Sharding them across nodes is a routing question, not a
  correctness one.

**Blockers and decisions ahead.**

- **Hash-ring rebalance.** When a node joins or leaves, in-flight
  buffers on the old owner have to flush before the new owner takes
  writes. The work-unit table is the natural coordination point; the
  fence semantics need writing down.
- **Hot-table backpressure.** A single very-hot table can bottleneck
  on its owner's CPU. The escape hatch — sharding one table's writes
  across owners and merging at compaction — needs the catalog to
  represent multiple staging buffers per table. Spec'd, not built.
- **Cross-frontend ordering.** REST, OTLP, and Kafka can all write
  the same table. Per-frontend ordering is meaningful; cross-frontend
  ordering already isn't (and won't be) — Slice 3 documents this
  rather than fixing it.

## Slice 4 — multi-region / multi-cluster federation

**Status:** committed direction. Catalog shape compatible.

**Scope.** Multiple kyma clusters — typically one per region —
federate at query time. A single Flight query can fan out across
clusters and merge results. Each cluster owns its own bucket and
catalog; cross-cluster reads are explicit, not transparent
replication.

**What Slice 1 introduced for it.**

- Object storage as the only source of truth (Invariant 1) means
  cross-region reads are a function of bucket policy and Flight
  endpoints, not of replication state.
- The query frontend is pluggable (Invariant 5), so a federation
  frontend that accepts a multi-cluster KQL query and dispatches
  per-cluster sub-queries is a `QueryFrontend` peer, not a fork.
- Per-table retention, already per-cluster, gives operators the knob
  for "keep audit logs in EU for seven years, traces globally for
  seven days" without per-region forks.

**Blockers and decisions ahead.**

- **Federation catalog.** A "table that spans clusters" has to live
  somewhere. The leading design is a thin federation catalog that
  maps logical tables to per-cluster physical tables; queries plan
  against the federation and fan out per cluster.
- **Auth and tenant boundary.** Cross-cluster Flight has to carry
  the original principal, not a service-to-service token. The Slice
  4 work depends on the auth design landing in Slice 2.
- **Cost surfaces.** Cross-region egress is the largest cost in any
  federated query. The planner needs cost-model inputs to prefer
  local scans over remote ones when both can answer the same
  predicate; the manifest stats already carry the fields it needs.

## Where each slice lives in the codebase

The trait surfaces are in `crates/kyma-core/src/`. The Slice 1
implementations are in their own crates — `kyma-format-tlm`,
`kyma-kql`, `kyma-cat-pg`. The architectural tests in
`benches/distribution/` are what fail in CI when a change crosses a
trait boundary the wrong way.

For the affordance details, see
[Architecture overview](/architecture/architecture). For the
storage-side spec, see [Storage format](/architecture/storage-format).
