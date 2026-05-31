---
title: GitHub
description: Ingest GitHub repos, pulls, issues, contributors, and a structural code graph — set up from the web UI or with one CLI command.
---

# GitHub connector

Ingests repository metadata + (optionally) a parsed code graph into
two tables: `github_nodes` and `github_edges`. Every repo, branch,
pull, issue, contributor, file, function, class, and call becomes a
node; the edges link them into a single property graph you can query
with `graph-match` / `graph-traverse`.

## What gets ingested

| Module         | Default | What it pulls                                                            |
| -------------- | ------- | ------------------------------------------------------------------------ |
| `repos`        | on      | Repo nodes (id, owner, name, description, primary language, stars, …).   |
| `branches`     | on      | Branch nodes + `BELONGS_TO_REPO` edges.                                  |
| `pulls`        | on      | Pull-request nodes (state, head, base, title, body) + `OPENED_BY`/`MERGED_INTO`. |
| `issues`       | on      | Issue nodes + `OPENED_BY`/`CLOSED_BY`/`LABELED_AS`.                       |
| `contributors` | on      | User nodes + `CONTRIBUTED_TO` edges with commit counts.                  |
| `codebase`     | off     | File/function/class/call/import nodes parsed via tree-sitter. **Slower**; rate-limit sensitive. |

Toggle modules per-connector via `--modules` (CLI) or the catalog
config field (UI).

## Setup — UI

1. Open `/connectors` → "Add connector" → pick **GitHub**.
2. Paste a Personal Access Token (or pick an existing PAT credential
   from the picker).
3. Enter one or more `owner/repo` paths.
4. Optionally enable the **Code graph** module (off by default — it
   downloads source files and parses them with tree-sitter, which
   takes longer and burns more API quota).
5. Save. The connector ticks on its schedule (default 5 min). Click
   **Trigger now** for an immediate first run.

## Setup — CLI

```bash
# One repo. Token auto-discovered from $GITHUB_TOKEN, $GH_TOKEN,
# or `gh auth token` (in that order).
kyma connector add github shakedaskayo/kyma --start

# Multiple repos under one connector. Codebase parsing on.
kyma connector add github \
  "anthropics/claude-code,anthropics/anthropic-sdk-python" \
  --codebase --start

# Explicit token, custom name and database.
kyma connector add github octocat/Hello-World \
  --token ghp_xxx \
  --name oss-demo \
  --db demo \
  --start

# Reuse a credential you've already stored.
kyma connector add github my-org/my-repo \
  --credential-id 6c6c0a52-… \
  --start
```

`--start` triggers an immediate first tick and polls until it lands
(or 30 s). Without `--start`, the connector waits for its first
scheduled tick.

## Token sources, in order

1. `--token <pat>` — explicit flag.
2. `--credential-id <uuid>` — reuse an existing credential.
3. `$GITHUB_TOKEN` env var.
4. `$GH_TOKEN` env var.
5. `gh auth token` shell-out (if [GitHub CLI](https://cli.github.com)
   is installed and authenticated). Best-effort, silent fallback.

If nothing resolves, `kyma connector add github` errors out with a
clear hint.

## What you get

Two tables in your target database (default `github`):

```text
github_nodes
  at         timestamp     when the node was ingested
  id         string        composite id, e.g. "repo:owner/name", "user:login"
  label      string        node type, e.g. "Repository", "User", "File"
  labels     string        comma-separated extra labels
  name       string        human-readable label
  body       string        description / readme excerpt / file path
  props      binary        JSON-encoded properties (stars, language, …)

github_edges
  at         timestamp
  id         string
  src        string        node id
  dst        string        node id
  type       string        e.g. "CONTRIBUTED_TO", "BELONGS_TO_REPO"
  label      string
  body       string
  props      binary
```

A graph called `github` is auto-registered over these tables, so
`graph-match` works out of the box:

```kql
github_nodes
| graph-match (u:User)-[:CONTRIBUTED_TO]->(r:Repository)
| project u.name, r.name
| summarize commits=count() by u.name
| top 10 by commits desc
```

## Schedules + idempotency

- Default schedule: every **5 minutes**.
- A tick is idempotent — re-running it doesn't duplicate rows. Each
  ingested entity carries its GitHub id; the dedup key is `(table,
  id)`.
- If the GitHub API rate-limits us, the connector pauses until the
  reset and logs `last_error = "rate limited until X"`.

## Web visualization

`/graph` (the **Graph Explorer**) renders the unified cross-database
graph — every repo, branch, pull, contributor and their relationships
on one canvas. Force / tree / radial / grid layouts; hover dims
non-neighbors, click to focus, sidebar drives namespace and label
filters.

See the README for a screenshot.

## Limits

- One PAT per connector. To ingest from multiple orgs with different
  permissions, create one connector per org.
- The `codebase` module currently parses Python, TypeScript,
  JavaScript, and Go. Other languages land in the next slice.
- No incremental fetches yet — every tick walks the whole repo
  metadata. For high-churn repos, tune `schedule_ms` up.
