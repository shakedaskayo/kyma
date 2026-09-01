---
title: Data sources
description: The pull-side surface — a generic DataSource trait, the shipped Prometheus reference, and three operational-database engines (Postgres, MySQL, MongoDB) that federate live and sync via CDC.
---

# Data sources

Data sources are how pensieve pulls from sources that don't push to it. The
trait is small, the registry is one map, and the runner is the same
group-commit loop the OTLP and Kafka frontends use — so a data source's
output lands on the same staging buffer, through the same snapshot CAS,
into the same pensieve extents.

One framework. One reference engine in tree. Three operational-database
engines on the way, each shipping in its own milestone.

![The add-data-source catalog in the pensieve UI](/screenshots/data-sources-catalog.png)
*The **Add data source** catalog — code, knowledge, project, and data sources side by side. Available sources are one click to connect; the grid is engine-driven, so a new data source type appears here automatically.*

<div class="feature-grid">

<div class="feature-card">

### [Framework](/data-sources/framework)

The `DataSource` trait, the registry, the periodic scheduler, the
admin REST API at `POST /v1/data-sources`, and the secret-by-reference
resolver. Generic to every data source type — read this once and the
five pages below are mostly config.

</div>

<div class="feature-card">

### [Prometheus](/data-sources/prometheus)

The reference implementation. Scrapes `/metrics` on a schedule,
parses OpenMetrics, lands one row per sample in the configured
table. Shipped today; the smallest possible thing that proves the
framework.

</div>

<div class="feature-card">

### [OAuth data sources](/data-sources/oauth)

SaaS sources behind a one-click **Connect** flow — [Notion](/data-sources/notion),
[Google Drive](/data-sources/googledrive), [Gmail](/data-sources/gmail),
[Slack](/data-sources/slack), [Jira](/data-sources/jira),
[Confluence](/data-sources/confluence). OAuth2 authorize → encrypted credential →
automatic token refresh. Operator-configured apps or bring-your-own.

</div>

<div class="feature-card">

### [GitHub](/data-sources/github)

Repositories, branches, pulls, issues, contributors, plus an optional
parsed code graph (functions, classes, calls, imports via tree-sitter).
One CLI command stands it up — token discovered from
`$GITHUB_TOKEN`, `$GH_TOKEN`, or `gh auth token`.

</div>

<div class="feature-card">

### [GitLab](/data-sources/gitlab)

Projects, branches, merge requests, issues, members. Self-hosted
GitLab supported via `--api-url`.

</div>

<div class="feature-card">

### [Bitbucket](/data-sources/bitbucket)

Repositories, branches, pull requests, issues from Bitbucket Cloud.
App-password + username (`basic`) or PAT auth.

</div>

<div class="feature-card">

### [Microsoft Fabric](/data-sources/msfabric)

The first **federated** data source: Lakehouse / Warehouse tables queried
live over the SQL analytics endpoint. Schemas cataloged in pensieve, data
stays in Fabric, whole subplans (filters, joins, aggregations) pushed
down as one T-SQL statement. Shipped today.

</div>

<div class="feature-card">

### [Postgres](/data-sources/postgres) 🚧

Federation for live reads, replication-slot CDC for sync, both at
once. Pushdown of filters, projection, `LIMIT`, single-source
aggregation. Lands in DB-M1.

</div>

<div class="feature-card">

### [MySQL](/data-sources/mysql) 🚧

Same shape as Postgres. Binlog row events with GTID checkpoints for
CDC; collation-safety rules in the planner so case-insensitive
columns never silently corrupt federated equality. Lands in DB-M2.

</div>

<div class="feature-card">

### [MongoDB](/data-sources/mongo) 🚧

Change streams with `startAtOperationTime` for CDC. BSON type
coercion, nested-document flattening up to a configurable depth,
polymorphic-field demotion to `dynamic`. Lands in DB-M3.

</div>

<div class="feature-card">

### [Multi-source data](/data-sources/multi-source-data) 🚧

The marquee surface — `live(...)` for federated reads, the
`pushdown_summary` on every federated response. Cross-references the
conceptual model at [Multi-source data](/concepts/multi-source-data).

</div>

</div>

## What's the same across all data source types

- **One trait.** Implement `DataSource::run_once`; the framework owns
  scheduling, retry classification, secret resolution, and the row sink.
- **One registry.** Data source types register at startup; the admin API
  validates `type` against it before persisting any row.
- **One write path.** Rows produced by a tick go through the same
  JSON-to-Arrow coercion, the same staging buffer, and the same snapshot
  CAS as REST/NDJSON ingest. See [Extents and snapshots](/concepts/extents-and-snapshots).
- **One health surface.** Per-tick Prometheus metrics (`pensieve_data_source_cursor_age_seconds`, `pensieve_data_source_rows_ingested_total`, etc.) plus `last_error`, `last_rows_ingested`, `last_success_at` on the data source row (`GET /v1/data-sources/:id`). See [Observability](/concepts/observability).

## What you get: a context graph

Graph data sources (GitHub, GitLab, Notion, Slack, Jira, …) don't just land rows —
they emit **nodes and edges** that register as a queryable property graph. A
GitHub data source produces `Repository`, `PullRequest`, `Issue`, `Branch`, and
`User` nodes linked by `AUTHORED`, `HAS_PULL_REQUEST`, `TARGETS_BRANCH`, and
more; a Notion data source produces `Page`/`Database`/`User` nodes linked by
`CONTAINS` and `RELATES_TO`. Every source feeds the same graph, so you can
traverse from a Slack message to the Jira issue it mentions to the GitHub PR
that closed it.

![The pensieve context graph rendered from data source data](/screenshots/context-graph.png)
*The **Graph** view rendering a GitHub data source's output — repositories, pull requests, issues, branches, and contributors as a navigable property graph. See [Query → Graph traversals](/query/) and the [graph concepts](/concepts/multi-source-data).*
