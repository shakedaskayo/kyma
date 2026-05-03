---
title: Concepts
description: The mental model behind kyma — five invariants, three pruning levels, one agent loop, and the storage shape that ties them together.
---

# Concepts

Kyma is opinionated about a small number of things. Almost every
load-bearing decision in the engine traces back to one of those
opinions. The pages below are the mental model — read in order if
you're starting; read by topic if you're deep on one.

If you only have time for two, start with
[**The five invariants**](/concepts/the-five-invariants) and
[**The pruning cascade**](/concepts/the-pruning-cascade). Everything
else is an instance of those two ideas.

<div class="feature-grid">

<div class="feature-card">

### [What kyma is](/concepts/what-is-kyma)

The ten-minute version of the value prop. What the engine ingests, how
it stores, how it answers, who it's for, and — explicitly — what it
isn't.

</div>

<div class="feature-card">

### [The five invariants](/concepts/the-five-invariants)

Five non-negotiable architectural properties — object storage as the
only source of truth, stateless compute, externalized catalog,
pluggable format, pluggable parser. Encoded as architectural tests;
regressions block merge.

</div>

<div class="feature-card">

### [The pruning cascade](/concepts/the-pruning-cascade)

Three levels of elimination — catalog, extent footer, block index —
that skip 99 % of bytes on 99 % of queries. Why a query without a
time bound plans like a full scan, and what to do about it.

</div>

<div class="feature-card">

### [Extents and snapshots](/concepts/extents-and-snapshots)

Append-only columnar files on object storage. CAS-committed snapshots.
Iceberg-style manifests. The shape that makes ingest exactly-once and
queries predictable.

</div>

<div class="feature-card">

### [Schema model](/concepts/schema-model)

Eight column types, the schema-only-widens rule, mid-batch evolution.
When to use `int` vs `long`, `string` vs `dynamic`, `vector(N)` vs
external embedding tables.

</div>

<div class="feature-card">

### [Dynamic and vectors](/concepts/dynamic-and-vectors)

The two non-relational column types. CBOR-encoded `dynamic` with token
+ path indices for arbitrary structured data. Fixed-dimension
`vector(N)` with cosine / L2 / inner-product UDFs.

</div>

<div class="feature-card">

### [The agent loop](/concepts/the-agent-loop)

`/v1/agent/ask`. Natural-language question in, Server-Sent Events out.
Schema RAG via pgvector keeps the agent's mental model accurate as
schemas evolve. Read-only by design.

</div>

<div class="feature-card">

### [Multi-source data](/concepts/multi-source-data)

How kyma joins your operational databases — Postgres, MySQL, MongoDB —
with its own tables. Federation for live reads; CDC sync for fast
historical queries; both at once via `live(table)`.

</div>

<div class="feature-card">

### [Retention and compaction](/concepts/retention-and-compaction)

Per-table retention policies. Compaction that merges small extents
into fewer fat ones for better pruning. Tombstone collapse on synced
tables. All as work-unit rows; adding capacity is starting another
node.

</div>

<div class="feature-card">

### [Observability](/concepts/observability)

How to tell what kyma is doing. Prometheus `/metrics`, the agent run
trace, `/v1/connectors/:id/status`, the queryable
`kyma_connector_health` table, and `pushdown_summary` — the trust
mechanism for federation.

</div>

</div>

## How to read this section

- **First time?** Read in order. Each page builds on the previous one.
  Quick path: invariants → cascade → extents → schema → agent loop.
- **Debugging a slow query?** Start with
  [the pruning cascade](/concepts/the-pruning-cascade), then
  [pruning and performance](/query/pruning-and-performance) for the
  practical companion.
- **Choosing a column type?** [Schema model](/concepts/schema-model)
  + [dynamic and vectors](/concepts/dynamic-and-vectors) cover it.
- **Integrating with another database?**
  [Multi-source data](/concepts/multi-source-data), then the
  [Connectors section](/connectors/) for the operational details.
- **Architecting a deployment?** Start with
  [the five invariants](/concepts/the-five-invariants), then
  [Architecture](/architecture/) for the slice roadmap.
