---
title: Query
description: Six surfaces — KQL, SQL, PromQL (roadmap), Arrow Flight, the agent endpoint, and the pruning rules they all share. Same logical plan, same DataFusion exec, same Arrow result.
---

# Query

Three query languages, two transports, one agent endpoint, and the
performance rules that apply to all of them. They share more than they
differ: every surface produces the same kyma logical plan, runs through
the same [pruning cascade](/concepts/the-pruning-cascade), executes on
the same DataFusion engine, and returns the same Arrow result.

Pick the surface that matches how you're calling kyma — not what the
data looks like, because that's the same on the other side.

<div class="feature-grid">

<div class="feature-card">

### [KQL](/query/kql)

Pipe-style language — read left-to-right, one operator at a time.
The default for ad-hoc operator queries and what
[the agent](/concepts/the-agent-loop) generates under the hood.
`Content-Type: application/x-kql`.

</div>

<div class="feature-card">

### [SQL](/query/sql)

Standard SQL via DataFusion. Same logical plan as KQL underneath, just
a different surface. Use it for cross-source joins, complex
sub-queries, or when SQL is what your tool already speaks.
`Content-Type: application/sql`.

</div>

<div class="feature-card">

### [PromQL](/query/promql) 🚧

Reserved for `Content-Type: application/promql`; frontend lands in a
later milestone. When it ships, existing Prometheus dashboards point
at kyma's endpoint with no other change.

</div>

<div class="feature-card">

### [Arrow Flight](/query/arrow-flight)

gRPC at `:9090` — zero-copy Arrow batches over the wire. Use it when
the NDJSON HTTP response is the bottleneck: streaming millions of
rows to a Polars DataFrame, a Spark connector, an Arrow-native client.

</div>

<div class="feature-card">

### [Agent endpoint](/query/agent-endpoint)

`POST /v1/agent/ask`. Natural-language question in, Server-Sent Events
out. The agent picks the right table, writes the KQL, runs it,
streams back tokens. See
[The agent loop](/concepts/the-agent-loop) for the conceptual model.

</div>

<div class="feature-card">

### [Pruning and performance](/query/pruning-and-performance)

The practical companion to
[The pruning cascade](/concepts/the-pruning-cascade) — what makes a
query fast or slow, how to read `pushdown_summary` and the metrics,
and a short cookbook of the patterns that prune well.

</div>

</div>

## What's the same across all surfaces

- **One logical plan.** Every frontend parses to the unified IR in
  `kyma-plan`. KQL and SQL are peer trait implementations; the planner
  doesn't know which one fed it.
- **One execution engine.** DataFusion runs vectorized over Arrow.
  Cross-source joins are the same exec path as native joins.
- **One pruning cascade.** Catalog → extent footer → block index →
  decode. A query without a time bound plans like a full scan
  regardless of which language wrote it.
- **One result shape.** Arrow `RecordBatch`es, served as NDJSON over
  HTTP or as Flight `RecordBatch` streams over gRPC.
