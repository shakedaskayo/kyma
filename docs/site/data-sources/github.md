---
title: GitHub
description: Ingest GitHub repos, pulls, issues, contributors, and a structural code graph — set up from the web UI or with one CLI command.
---

# GitHub data source

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

Toggle modules per-data-source via `--modules` (CLI) or the catalog
config field (UI).

## Setup — UI

1. Open `/data-sources` → "Add data source" → pick **GitHub**.
2. Paste a Personal Access Token (or pick an existing PAT credential
   from the picker).
3. Enter one or more `owner/repo` paths.
4. Optionally enable the **Code graph** module (off by default — it
   downloads source files and parses them with tree-sitter, which
   takes longer and burns more API quota).
5. Save. The data source ticks on its schedule (default 5 min). Click
   **Trigger now** for an immediate first run.

## Setup — CLI

Codebase parsing (the tree-sitter structural code graph) is **ON by
default**. Pass `--no-codebase` for metadata-only ingestion.

```bash
# One repo. Token auto-discovered from $GITHUB_TOKEN, $GH_TOKEN,
# or `gh auth token` (in that order).
kyma datasource add github shakedaskayo/kyma --start

# Multiple repos under one data source.
kyma datasource add github \
  "anthropics/claude-code,anthropics/anthropic-sdk-python" \
  --start

# Metadata only (no source parsing) — faster, less API quota.
kyma datasource add github shakedaskayo/kyma --no-codebase --start

# Restrict code parsing to specific languages + tune caps.
kyma datasource add github my-org/big-repo \
  --languages rust,go \
  --max-files 1000 \
  --max-bytes 524288 \
  --exclude 'vendor/**,**/*_test.go,dist/**' \
  --max-pages 5 \
  --start

# Explicit token, custom name and database.
kyma datasource add github octocat/Hello-World \
  --token ghp_xxx \
  --name oss-demo \
  --db demo \
  --start

# Reuse a credential you've already stored.
kyma datasource add github my-org/my-repo \
  --credential-id 6c6c0a52-… \
  --start
```

`--start` triggers an immediate first tick and polls until it lands
(or 30 s). Without `--start`, the data source waits for its first
scheduled tick.

### Ingestion knobs

All optional; defaults match server-side defaults.

| Flag                  | What it does                                                     | Default                                 |
| --------------------- | ---------------------------------------------------------------- | --------------------------------------- |
| `--no-repos`          | Skip the repos module.                                           | enabled                                 |
| `--no-branches`       | Skip the branches module.                                        | enabled                                 |
| `--no-pulls`          | Skip the PR module.                                              | enabled                                 |
| `--no-issues`         | Skip the issues module.                                          | enabled                                 |
| `--no-contributors`   | Skip the contributors module.                                    | enabled                                 |
| `--no-codebase`       | Skip structural code parsing.                                    | **enabled**                             |
| `--languages a,b,c`   | Restrict code parsing to these languages.                        | rust, python, typescript, javascript, go |
| `--max-bytes N`       | Skip files larger than N bytes.                                  | 1 048 576 (1 MiB)                       |
| `--max-files N`       | Hard cap on files fetched + parsed per tick.                     | 300                                     |
| `--exclude 'a,b,c'`   | Comma-separated glob patterns to skip.                           | sensible vendor/generated defaults      |
| `--max-pages N`       | Cap on API pages per module per tick (100 items/page).           | 10                                      |
| `--schedule-ms N`     | Tick interval in milliseconds.                                   | 300 000 (5 min)                         |

## Token sources, in order

1. `--token <pat>` — explicit flag.
2. `--credential-id <uuid>` — reuse an existing credential.
3. `$GITHUB_TOKEN` env var.
4. `$GH_TOKEN` env var.
5. `gh auth token` shell-out (if [GitHub CLI](https://cli.github.com)
   is installed and authenticated). Best-effort, silent fallback.

If nothing resolves, `kyma datasource add github` errors out with a
clear hint.

### Same knobs on gitlab / bitbucket

`kyma datasource add gitlab …` and `kyma datasource add bitbucket …`
accept the same `--no-codebase`, `--languages`, `--max-bytes`,
`--max-files`, `--exclude` flags. The server-side codebase parsing
for those providers is on the roadmap — the CLI surface is reserved
so today's invocations keep working when it lands.

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
- If the GitHub API rate-limits us, the data source pauses until the
  reset and logs `last_error = "rate limited until X"`.

## Web visualization

`/graph` (the **Graph Explorer**) renders the unified cross-database
graph — every repo, branch, pull, contributor and their relationships
on one canvas. Force / tree / radial / grid layouts; hover dims
non-neighbors, click to focus, sidebar drives namespace and label
filters.

See the README for a screenshot.

## Limits

- One PAT per data source. To ingest from multiple orgs with different
  permissions, create one data source per org.
- The `codebase` module currently parses Python, TypeScript,
  JavaScript, and Go. Other languages land in the next slice.
- No incremental fetches yet — every tick walks the whole repo
  metadata. For high-churn repos, tune `schedule_ms` up.
