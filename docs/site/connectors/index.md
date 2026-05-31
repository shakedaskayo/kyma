---
title: Connectors
description: The pull-side surface — a generic Connector trait, the shipped Prometheus reference, and three operational-database engines (Postgres, MySQL, MongoDB) that federate live and sync via CDC.
---

# Connectors

Connectors are how kyma pulls from sources that don't push to it. The
trait is small, the registry is one map, and the runner is the same
group-commit loop the OTLP and Kafka frontends use — so a connector's
output lands on the same staging buffer, through the same snapshot CAS,
into the same kyma extents.

One framework. One reference engine in tree. Three operational-database
engines on the way, each shipping in its own milestone.

<div class="feature-grid">

<div class="feature-card">

### [Framework](/connectors/framework)

The `Connector` trait, the registry, the periodic scheduler, the
admin REST API at `POST /v1/connectors`, and the secret-by-reference
resolver. Generic to every connector type — read this once and the
five pages below are mostly config.

</div>

<div class="feature-card">

### [Prometheus](/connectors/prometheus)

The reference implementation. Scrapes `/metrics` on a schedule,
parses OpenMetrics, lands one row per sample in the configured
table. Shipped today; the smallest possible thing that proves the
framework.

</div>

<div class="feature-card">

### [GitHub](/connectors/github)

Repositories, branches, pulls, issues, contributors, plus an optional
parsed code graph (functions, classes, calls, imports via tree-sitter).
One CLI command stands it up — token discovered from
`$GITHUB_TOKEN`, `$GH_TOKEN`, or `gh auth token`.

</div>

<div class="feature-card">

### [GitLab](/connectors/gitlab)

Projects, branches, merge requests, issues, members. Self-hosted
GitLab supported via `--api-url`.

</div>

<div class="feature-card">

### [Bitbucket](/connectors/bitbucket)

Repositories, branches, pull requests, issues from Bitbucket Cloud.
App-password + username (`basic`) or PAT auth.

</div>

<div class="feature-card">

### [Postgres](/connectors/postgres) 🚧

Federation for live reads, replication-slot CDC for sync, both at
once. Pushdown of filters, projection, `LIMIT`, single-source
aggregation. Lands in DB-M1.

</div>

<div class="feature-card">

### [MySQL](/connectors/mysql) 🚧

Same shape as Postgres. Binlog row events with GTID checkpoints for
CDC; collation-safety rules in the planner so case-insensitive
columns never silently corrupt federated equality. Lands in DB-M2.

</div>

<div class="feature-card">

### [MongoDB](/connectors/mongo) 🚧

Change streams with `startAtOperationTime` for CDC. BSON type
coercion, nested-document flattening up to a configurable depth,
polymorphic-field demotion to `dynamic`. Lands in DB-M3.

</div>

<div class="feature-card">

### [Multi-source data](/connectors/multi-source-data) 🚧

The marquee surface — `live(...)` for federated reads, the
`pushdown_summary` on every federated response, `kyma_connector_health`
as a queryable kyma table. Cross-references the conceptual model at
[Multi-source data](/concepts/multi-source-data).

</div>

</div>

## What's the same across all connector types

- **One trait.** Implement `Connector::run_once`; the framework owns
  scheduling, retry classification, secret resolution, and the row sink.
- **One registry.** Connector types register at startup; the admin API
  validates `type` against it before persisting any row.
- **One write path.** Rows produced by a tick go through the same
  JSON-to-Arrow coercion, the same staging buffer, and the same snapshot
  CAS as REST/NDJSON ingest. See [Extents and snapshots](/concepts/extents-and-snapshots).
- **One health surface.** Per-tick metrics, `last_error`, `last_rows_ingested`
  on the connector row, plus the `kyma_connector_health` view (DB-M0+) for
  the database engines. See [Observability](/concepts/observability).
