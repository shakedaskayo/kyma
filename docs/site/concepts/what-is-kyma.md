---
title: What kyma is
description: kyma is the context engine for coding agents — durable graph-aware memory, live data (logs, traces, code) in KQL/SQL, and the graph that links them, over one surface. Runs as a local single binary; scales to a team control plane.
---

# What kyma is

**The context engine for coding agents.** One place your agent recalls durable
**memory**, queries **live data** (logs, traces, code, connectors) in KQL or SQL,
and walks the **graph** that ties a decision to the resources it's about.

A memory store remembers what you told it. A context engine also knows your live
systems and how everything connects — and hands all of it to your agent through one
surface (a Claude Code plugin, a CLI, or MCP).

## The three layers

kyma is one engine that does three things, in order of how you'll meet them:

- **Memory** — durable, graph-aware recall across sessions and machines. Capture →
  LLM extraction with bi-temporal conflict resolution → near-realtime hybrid recall
  (vector + keyword + graph). The "it remembers" layer your agent feels first.
- **Live data** — every signal your stack emits (logs, traces, metrics, tool calls,
  prompt/response bodies, deploy events, config diffs) ingested through OTLP, REST,
  Kafka, or file-drop, plus scheduled connectors — answered in KQL or SQL at
  sub-second latency over years of history.
- **The graph** — entities and relationships as a first-class layer. Memories resolve
  to the *real* nodes they're about (a repo, a service, a table, a trace), so recall
  comes back wired to the systems it concerns.

## The engine underneath

The reason "live data" means a decade of history answered in milliseconds — not a
toy key-value store:

- **Columnar Arrow on object storage you own** (S3, MinIO, any `object_store` impl),
  with per-extent column statistics and token indices that make 99 %+ of queries skip
  99 %+ of data.
- **KQL, SQL, or PromQL** over Arrow Flight gRPC — exact rows, streamed zero-copy — so
  an agent can ask twenty exploratory questions per prompt without melting a credit card.
- **Federates with your operational databases** — Postgres, MySQL, MongoDB register as
  catalogs and DataFusion joins them with kyma's own tables in one query.
- **Scales from one binary to many nodes** without a rewrite: the catalog is externalized
  from byte one, compute is stateless, object storage is the source of truth. See
  [Local & cluster mode](/concepts/local-and-cluster-mode).

## What it is not

- Not *just* a memory layer. Memory SaaS gives the agent facts; kyma gives it the facts,
  the live logs/traces/code those facts are about, and the graph linking them — one query
  surface, one MCP server.
- Not a metrics database. Metrics are one signal among many; treating them specially
  fragments the surface an agent needs to reason across.
- Not a log search appliance. Full-text on the `dynamic` column works, but the engine is
  column-aware first; a search-shaped query plans like a filter, not like a Lucene query.
- Not an OLAP warehouse for business data. Use it for memory, observability, and
  operational state — and for joining production telemetry with your customer table — not
  as your primary BI cube.

## Who it's for

The coding agent that needs to remember, and to reason across the whole stack — and the
developer tired of an agent that forgets every session and can't see production.

## Where to go next

- The whole picture, end to end: [How kyma works](/concepts/how-kyma-works).
- Give your agent memory in five minutes: [Quickstart](/quickstart/give-your-agent-memory).
- How recall and the graph work: [Agentic Memory](/agent/memory).
- How a query stays fast: [The pruning cascade](/concepts/the-pruning-cascade).
