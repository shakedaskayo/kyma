---
title: PromQL
description: PromQL is on the kyma roadmap. The frontend will plug into the same QueryFrontend trait KQL and SQL use, parsing PromQL into the same logical plan that runs through the pruning cascade.
---

# PromQL

> 🚧 **Roadmap.** PromQL is not yet shipped. The MIME type
> `application/promql` is reserved for it. Until the frontend lands,
> requests with that `Content-Type` fall through to the SQL parser and
> error with `400 sql_parse_error` — i.e. the response is honest about
> what hasn't been implemented yet, but the surface isn't usable. Track
> progress in the [README roadmap](https://github.com/shaked/engine#roadmap).

## Why PromQL fits

kyma's query path is structured around a single trait, `QueryFrontend`.
A frontend is a parser: it takes a source string and returns a logical
plan that the rest of the engine — DataFusion execution, the three-level
pruning cascade, Arrow Flight transport — already knows how to run.

Today there are two implementations: KQL (`kyma-kql`) and SQL (DataFusion's
own parser). PromQL becomes a third. Once the parser lands, every PromQL
query benefits from the same machinery as the other two:

- Catalog pruning by time range and per-column min/max.
- Block-level inverted-index pruning for label predicates.
- Zero-copy Arrow Flight transport for results.
- Multi-node read fan-out via the same read-router.

The trait is in `kyma-core/src/query_frontend.rs`. Frontend authors
implement `parse(source, ctx) -> Arc<dyn Any>`, and the registry in
`kyma-plan` downcasts the payload to the concrete `LogicalPlan`. There's
no special path for PromQL queries — they're just another frontend.

## What's reserved today

| Field        | Value                       |
| ------------ | --------------------------- |
| MIME type    | `application/promql`        |
| Endpoint     | `POST /v1/query`            |
| Status       | Not implemented             |

The MIME type is reserved so existing client code can be written against
the eventual surface today. When the frontend ships, the same request
shape becomes valid — no new endpoint, no new auth model, no new
configuration knob.

## Migration story

Existing Prometheus dashboards (Grafana, custom UIs, anything that speaks
PromQL HTTP) point at kyma's query endpoint with no other changes. The
query string is still PromQL; the response is still result rows. What
changes underneath is that the query runs against years of history pruned
to milliseconds, instead of a Prometheus TSDB sized for a few weeks.

Long-retention metrics, joins between metrics and logs in the same
query, and federated queries against a synced Postgres table all work
the same way they do for KQL and SQL today — see [Multi-source
data](/concepts/multi-source-data).

## Where to track progress

- The [README roadmap](https://github.com/shaked/engine#roadmap) — PromQL
  is in the "next" tier.
- The [`QueryFrontend`
  trait](https://github.com/shaked/engine/blob/main/crates/kyma-core/src/query_frontend.rs)
  is the contract a future implementation will satisfy.
- Discussions and the eventual spec land under
  [`docs/superpowers/specs/`](https://github.com/shaked/engine/tree/main/docs/superpowers/specs).

## What to use today

- [SQL](/query/sql) for ad-hoc analytical queries — DataFusion's full
  surface plus federation.
- [Arrow Flight](/query/arrow-flight) for zero-copy result transport
  when the NDJSON HTTP path is the bottleneck.
- [The agent endpoint](/query/agent-endpoint) for natural-language
  questions that compile to KQL or SQL.
