---
title: What kyma is
description: A unified columnar query engine for every signal your stack emits — logs, traces, metrics, agent telemetry, and your own databases — answered in KQL or SQL at sub-second latency over years of history.
---

# What kyma is

A single data engine that turns the whole production picture into something
agents can query.

Point every OTLP emitter in your stack — services, Kubernetes, CI, queues,
frontend RUM, your agents themselves — at one engine. Then let an agent ask
it anything, in KQL or SQL, at sub-second latency over a decade of history.
No dashboards to scrape. No vendor APIs to juggle. No rate limits.

## What it does

- **Ingests every signal your stack already emits** — logs, traces, metrics,
  spans, tool calls, prompt and response bodies, deploy events, config diffs,
  audit trails — through one OTLP pipe, plus REST, Kafka, and file-drop for
  non-OTLP sources.
- **Stores it as columnar Arrow** on object storage you own (S3, MinIO, any
  `object_store` impl), with per-extent column statistics and token indices
  that make 99 %+ of queries skip 99 %+ of data.
- **Answers in KQL, SQL, or PromQL** over Arrow Flight gRPC — exact rows,
  streamed zero-copy — so an agent can ask twenty exploratory questions per
  user prompt without melting a credit card.
- **Federates with your operational databases** — Postgres, MySQL, MongoDB
  register as catalogs, and DataFusion joins them with kyma's own tables in
  a single query. Live or synced; you pick per source.
- **Scales from one binary to many nodes** without a rewrite. The catalog
  is externalized from byte one, compute is stateless, object storage is the
  source of truth.

## What it is not

- Not a metrics database. Metrics are one signal among many; treating them
  specially fragments the query surface that agents need to reason across.
- Not a log search appliance. Full-text on the `dynamic` column works, but
  the engine is column-aware first; a search-shaped query plans like a
  filter, not like a Lucene query.
- Not an OLAP warehouse for business data. The data model is built around
  append-only, time-partitioned, token-indexed columnar storage. Use it for
  observability and operational state. Use it for joining production telemetry
  with your customer table. Don't use it as your primary BI cube.
- Not a managed service. kyma is a binary you run. It expects an object
  store and a Postgres catalog you own.

## Who it's for

The agent that needs production awareness across the whole stack.

Concretely: anyone whose tools answer questions like "what changed?", "where
did this break?", "which customer is affected?", "is this regression real or
last-week's-rollout's tail?" — and who's tired of stitching three vendors
together to answer them.

## Where to go next

- The mental model: [The five invariants](/concepts/the-five-invariants).
- How a query stays fast: [The pruning cascade](/concepts/the-pruning-cascade).
- Multi-source data: [Multi-source data](/concepts/multi-source-data).
- Hands-on in five minutes: [Quickstart](/quickstart/).
