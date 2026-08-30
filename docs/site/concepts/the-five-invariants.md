---
title: The five invariants
description: Five non-negotiable architectural properties that turn distribution from a rewrite into a deployment change. Encoded as architectural tests; regressions block merge.
---

# The five invariants

Every load-bearing decision in pensieve traces back to one of five invariants.
Together, they're the difference between "this scales when we need it" and
"we have to rewrite the engine to scale."

They're encoded in CI as architectural tests under `benches/distribution/`.
Regressions block merge.

## 1. Object storage is the only source of truth

Local disk is cache, never master. Losing every compute node means zero
data loss; the catalog and object store are enough to bring the engine back
up on a fresh fleet.

**What this rules out.** Per-node state machines. Quorum protocols between
compute nodes. Any code path that treats a local path as authoritative.

**What it enables.** Stateless compute is the natural shape. Adding a node
means starting a binary; removing one means killing it. Multi-region read
fan-out becomes possible because every region sees the same source of truth.

## 2. Query nodes are stateless

A node's state is its config plus its cache. Any durable state lives in the
catalog or on object storage.

**What this rules out.** Sticky sessions. Per-node ingest checkpoints.
"Schedule this query on the node that has it cached" hacks (cache locality
becomes a planner hint, not a correctness requirement).

**What it enables.** Crash recovery is a process restart. Rolling deploys
are uneventful. The same binary runs alone, in three replicas, or in three
hundred — no flag flips, no migrations.

## 3. Catalog is externalized from byte one

Single-node dev still runs Postgres in a separate process. There is no
embedded-catalog code path to delete later.

**What this rules out.** "Quick mode" with a local SQLite. A shipped binary
that "graduates" to Postgres later. Special-cased single-node logic in any
catalog operation.

**What it enables.** Iceberg-style manifests, CAS commits, per-column stats,
schema history — all already work the way they need to in a distributed
system, because they were never written for a non-distributed one.

## 4. Format is pluggable

`SegmentFormat` is the trait boundary between storage and the rest of the
engine. The shipping implementation, `pensieve-format-tlm`, is one of many
possible formats.

**What this rules out.** Format-specific code in the planner, the executor,
the catalog, or any ingest frontend. Anything outside the trait that
"just happens to work for tlm."

**What it enables.** Future formats — a wider-column format for analytical
shapes, a smaller-column format for very long retention, a Parquet shim for
interop — land as new trait implementations, not as engine-wide rewrites.

## 5. Parser is pluggable

`QueryFrontend` is the trait boundary for query languages. `pensieve-kql` is one
frontend; SQL, PromQL, and custom DSLs layer on as peers.

**What this rules out.** Hard-coding KQL semantics into the optimizer. A
SQL-only fast path. Any "the parser knows the planner" coupling.

**What it enables.** Adding a new language is implementing one trait, not
rewriting query handling. Domain-specific frontends — a cleaner-than-KQL DSL
for ML feature lookup, say — coexist without forking.

## What this means in practice

Violating any one of these turns distribution from a deployment change into
a rewrite. They're cheap to maintain when respected at design time and
expensive past Slice 2 to retrofit.

The architecture document — [Architecture](/architecture/architecture) —
spells out the slice roadmap and the affordances each slice picks up. The
short version: invariants 1–5 are what makes Slices 2, 3, and 4 *bounded*
projects rather than rewrites.
