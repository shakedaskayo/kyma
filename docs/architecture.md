# kyma — Architecture (living document)

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
   storage and everything else. `kyma-format-tlm` is one implementation of
   many possible formats.
5. **Parser is pluggable.** `QueryFrontend` is the trait boundary for query
   languages. `kyma-kql` is one frontend; SQL / PromQL / custom DSLs layer on
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
  of the engine by `kyma-exec::df_adapter`, so DataFusion version churn touches
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

## Slice roadmap

| Slice | Scope | Duration |
|---|---|---|
| 1 | Single-node, distribution-ready affordances | 12-24mo |
| 2 | Read scale-out | ~3-6mo |
| 3 | Ingest scale-out | ~6mo |
| 4 | Multi-region / multi-cluster federation | 12+mo |

Each subsequent slice gets its own plan once the previous one ships.
