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

![The Add-connector catalog in the kyma UI](/screenshots/connectors-catalog.png)
*The **Add connector** catalog — code, knowledge, project, and data sources side by side. Available sources are one click to connect; the grid is engine-driven, so a new connector type appears here automatically.*

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

### [OAuth connectors](/connectors/oauth)

SaaS sources behind a one-click **Connect** flow — [Notion](/connectors/notion),
[Google Drive](/connectors/googledrive), [Gmail](/connectors/gmail),
[Slack](/connectors/slack), [Jira](/connectors/jira),
[Confluence](/connectors/confluence). OAuth2 authorize → encrypted credential →
automatic token refresh. Operator-configured apps or bring-your-own.

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
`pushdown_summary` on every federated response. Cross-references the
conceptual model at [Multi-source data](/concepts/multi-source-data).

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
- **One health surface.** Per-tick Prometheus metrics (`kyma_connector_cursor_age_seconds`, `kyma_connector_rows_ingested_total`, etc.) plus `last_error`, `last_rows_ingested`, `last_success_at` on the connector row (`GET /v1/connectors/:id`). See [Observability](/concepts/observability).

## What you get: a context graph

Graph connectors (GitHub, GitLab, Notion, Slack, Jira, …) don't just land rows —
they emit **nodes and edges** that register as a queryable property graph. A
GitHub connector produces `Repository`, `PullRequest`, `Issue`, `Branch`, and
`User` nodes linked by `AUTHORED`, `HAS_PULL_REQUEST`, `TARGETS_BRANCH`, and
more; a Notion connector produces `Page`/`Database`/`User` nodes linked by
`CONTAINS` and `RELATES_TO`. Every source feeds the same graph, so you can
traverse from a Slack message to the Jira issue it mentions to the GitHub PR
that closed it.

![The kyma context graph rendered from connector data](/screenshots/context-graph.png)
*The **Graph** view rendering a GitHub connector's output — repositories, pull requests, issues, branches, and contributors as a navigable property graph. See [Query → Graph traversals](/query/) and the [graph concepts](/concepts/multi-source-data).*
