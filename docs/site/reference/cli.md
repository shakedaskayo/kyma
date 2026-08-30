---
title: CLI
description: pensieve CLI reference — local engine (mcp, serve, setup, sync, worker, service), client commands (connect, status, query, sessions, install-skill, data source, ingest, recall, remember, entity, distill, deploy), and admin commands (create-database, create-table, ...).
---

# CLI

`pensieve` is the binary. It has three mode groups:

- **Local engine** — runs an embedded context engine (SQLite catalog +
  local object store, zero infra). Includes `pensieve mcp`, `pensieve serve`,
  `pensieve setup`, `pensieve sync`, `pensieve worker`, `pensieve service`.
- **Client mode** — talks to a running Pensieve server over HTTP. Used for
  asking the agent questions, managing data sources, and wiring up coding
  agents.
- **Admin mode** — talks directly to the Postgres catalog. Used for
  provisioning databases, tables, and graphs from scripts.

All modes coexist in the same binary; subcommand selection picks the
mode.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash
pensieve version
```

The installer downloads the prebuilt binary to `~/.local/bin` (or `/usr/local/bin`).
Binary name is `pensieve` (not `pensieve-cli`).

From source (contributors): inside a checkout, run `cargo install --path crates/pensieve-cli`,
or use `… | bash -s -- --from-source`.

## Global flags

| Flag             | Env var              | Default                                                | Used by            |
| ---------------- | -------------------- | ------------------------------------------------------ | ------------------ |
| `--catalog-url`  | `PENSIEVE_CATALOG_URL`   | `postgres://pensieve:pensieve_dev@localhost:5433/pensieve`         | Admin subcommands. |

Client-mode subcommands read connection info from `~/.pensieve/config.json`
(written by `pensieve connect`). Two env vars override the file:

| Var               | Effect                                  |
| ----------------- | --------------------------------------- |
| `PENSIEVE_SERVER_URL` | Overrides the saved endpoint.           |
| `PENSIEVE_TOKEN`      | Overrides the saved bearer token.       |

---

## Local engine subcommands

These run the embedded context engine — no Postgres or S3 required.
Data lives under `~/.pensieve/` by default (override with `PENSIEVE_HOME`).

### `pensieve mcp`

Serve the Model Context Protocol over **stdio** — what a coding agent
spawns automatically via the MCP config written by `pensieve setup`. Starts
the embedded engine, runs an initial Claude Code file-memory sync, then
serves the full context-engine toolset (memory + data + graph) over the
MCP protocol.

```bash
pensieve mcp   # run directly to test; agents invoke this as the MCP server command
```

No flags. The agent connects via stdin/stdout. See the
[MCP reference](/reference/mcp) for the full tool list.

### `pensieve serve [--addr ADDR]`

Serve the web UI + HTTP API locally (query / catalog / graph / ingest /
MCP). Browse the graph and ingest on demand. Sign-in defaults:
`admin` / `admin`.

| Flag     | Env var               | Default             |
| -------- | --------------------- | ------------------- |
| `--addr` | `PENSIEVE_LOCAL_HTTP_ADDR` | `127.0.0.1:7777`   |

```bash
pensieve serve
pensieve serve --addr 0.0.0.0:8888
```

### `pensieve setup <agent|list> [--print]`

Wire a coding agent to `pensieve mcp` over stdio with a one-liner. Writes
(merging, never clobbering other servers) the agent's MCP config file.

Usage: `pensieve setup <agent>` where `<agent>` is one of:

| Agent key      | Config file written                         | Scope          |
| -------------- | ------------------------------------------- | -------------- |
| `claude-code`  | `.mcp.json`                                 | project (cwd)  |
| `cursor`       | `.cursor/mcp.json`                          | project (cwd)  |
| `windsurf`     | `~/.codeium/windsurf/mcp_config.json`       | global (home)  |

`pensieve setup list` prints the supported agent keys. Any unknown agent
key emits a generic stdio MCP snippet to paste manually.

`--print` previews the resulting config without writing it.

```bash
pensieve setup claude-code        # write .mcp.json in the current project
pensieve setup cursor --print     # preview only
pensieve setup list               # show supported agents
```

### `pensieve sync [--watch] [--dry-run] [--cc-only] [--cloud-only] [--project PATH]`

Sync memory with Claude Code's file memory (`~/.claude/projects/*/memory`,
always present) and bidirectionally with a control plane (when
`PENSIEVE_CLOUD_URL` is set).

The file phase ingests + embeds Claude Code memory files, promotes
high-value pensieve memories back as native files, and curates `MEMORY.md`.

| Flag            | Purpose                                                                 |
| --------------- | ----------------------------------------------------------------------- |
| `--watch`       | Keep running, re-syncing on an interval (`PENSIEVE_CC_SYNC_POLL_SECS`, default 30s). |
| `--dry-run`     | Plan + audit-log Claude Code file changes without writing them. Ingestion into the local store still runs. |
| `--cc-only`     | Only the Claude Code file phase (skip the control plane).               |
| `--cloud-only`  | Only the control-plane push/pull (skip Claude Code files).              |
| `--project PATH` | Limit the file phase to one project path.                              |

```bash
pensieve sync                       # one-shot sync
pensieve sync --watch               # background loop
pensieve sync --dry-run             # preview file changes
```

### `pensieve worker <action>`

**Two distinct worlds under one subcommand:**

#### Background sync service (`install | uninstall | status`)

Manage an OS user service (launchd on macOS, systemd --user on Linux)
running `pensieve sync --watch` so memory stays synced with no terminal open.

```bash
pensieve worker install [--interval SECS] [--cc-only] [--cloud-only]
pensieve worker uninstall
pensieve worker status
```

| Flag           | Purpose                                        |
| -------------- | ---------------------------------------------- |
| `--interval N` | Poll interval in seconds (default 30; sets `PENSIEVE_CC_SYNC_POLL_SECS`). |
| `--cc-only`    | Only Claude Code file phase.                   |
| `--cloud-only` | Only control-plane push/pull.                  |

#### Fabric node daemon + admin (`run | create | list | revoke`)

Register this machine as a worker node with the control plane, pull
and run jobs (data source syncs, dreaming tasks, etc.).

```bash
# Start the node daemon
pensieve worker run --server <URL> --token <TOKEN> \
  [--accept source_sync,dreaming] [--max-concurrent 2] [--name my-box]

# Admin: mint a new worker identity
pensieve worker create --name my-box [--capabilities sources,dreaming]

# Admin: list registered nodes
pensieve worker list

# Admin: revoke a node
pensieve worker revoke <worker-id>
```

`pensieve worker run` flags:

| Flag               | Env var              | Default           | Purpose                                          |
| ------------------ | -------------------- | ----------------- | ------------------------------------------------ |
| `--server URL`     | `PENSIEVE_SERVER_URL`    | _required_        | Control-plane URL.                               |
| `--token TOKEN`    | `PENSIEVE_WORKER_TOKEN`  | _required_        | Worker auth token from `pensieve worker create`.     |
| `--accept LIST`    | —                    | `source_sync`     | Comma-separated job kinds this node accepts.     |
| `--max-concurrent N` | —                  | `1`               | Max jobs running in parallel.                    |
| `--name NAME`      | —                    | `node@<hostname>` | Friendly node name shown in `pensieve worker list`.  |

### `pensieve service <action>`

Manage the local Pensieve server (`pensieve serve`) as an OS user service (launchd
on macOS, systemd --user on Linux): starts at login, restarts on crash.

```bash
pensieve service install [--addr ADDR] [--token TOKEN]
pensieve service uninstall
pensieve service status
```

| Flag       | Purpose                                                                   |
| ---------- | ------------------------------------------------------------------------- |
| `--addr`   | Listen address (default `127.0.0.1:7777`).                               |
| `--token`  | Static admin token (`PENSIEVE_AUTH_TOKENS=<token>:admin` in the service env). |

---

## Client subcommands

### `pensieve connect <url> [--token TOKEN]`

Save a connection to a running Pensieve server. Writes
`~/.pensieve/config.json` (mode `0600`).

```bash
pensieve connect http://localhost:8080 --token "$(curl -s -XPOST ... | jq -r .access_token)"
```

### `pensieve status`

Show the saved endpoint, whether a token is configured, and probe
`/health`.

```bash
pensieve status
# Endpoint:  http://localhost:8080
# Token:     configured
# Health:    {"status":"ok","version":"0.0.1"}
```

### `pensieve query "<question>" [--json] [--session ID] [--continue]`

Stream `/v1/agent/ask` to stdout. Without `--json`, prints the
`answer_delta` / `answer_final` text directly. With `--json`, emits
the raw SSE event stream as JSONL — useful for scripting.

`--session ID` resumes a specific conversation session; `--continue` resumes the
most recent one `query` used (pensieve records the last session id locally), so
follow-up questions keep context.

```bash
pensieve query "how many rows in github_nodes?"
pensieve query "any error logs from prod-api in the last 15 minutes?"
pensieve query "and how many of those were 500s?" --continue
pensieve query "list databases" --json | jq -c '.event,.data'
```

### `pensieve sessions <op>`

Inspect or manage the agent's conversation sessions (each `query` turn belongs to
one). Talks to `/v1/agent/sessions`.

```bash
pensieve sessions list              # recent sessions
pensieve sessions show <id>         # metadata + rolling summary
pensieve sessions turns <id>        # every turn in order
pensieve sessions delete <id>       # delete a session and all its turns
```

### `pensieve user <op>` — requires an admin token

Manage users over HTTP (`/v1/admin/users`). The configured token must have the
**admin** role. Passwords are read interactively unless `--password-stdin`.

```bash
pensieve user list
pensieve user create alice --role write             # role: read | write | admin (default read)
pensieve user create bot --role read --password-stdin <<<"$PW"
pensieve user passwd alice                          # reset a password
pensieve user set-role alice admin                  # change role
pensieve user delete alice --yes                    # --yes skips the confirm prompt
```

### `pensieve install-skill [--target DIR] [--also-link-claude] [--which pensieve|deploy|all]`

Write a `SKILL.md` that teaches Claude Code / Cursor / Aider / etc.
how to use the `pensieve` CLI as a data tool.

| Flag                  | Effect                                                                  |
| --------------------- | ----------------------------------------------------------------------- |
| `--target DIR`        | Where to write `SKILL.md`. Default `~/.pensieve/skills/<skill>/`.           |
| `--also-link-claude`  | Symlink `~/.claude/skills/pensieve` → the target dir (Unix only).           |
| `--which`             | `pensieve` (default) — the data/query skill; `deploy` — the production-deployment runbook; `all` — both. |

### `pensieve install-plugin [--target DIR] [--force]`

Install the [pensieve-memory Claude Code plugin](/agent/claude-code-plugin) —
hooks + a bundled MCP server + slash commands — into
`~/.claude/skills/pensieve-memory/`. Templates your saved server URL + token into
the plugin's `.mcp.json` so it works immediately. Restart Claude Code, then run
`/pensieve-status`.

| Flag           | Effect                                                              |
| -------------- | ------------------------------------------------------------------ |
| `--target DIR` | Install into `DIR` instead of `~/.claude/skills/pensieve-memory`.       |
| `--force`      | Overwrite an existing install without the warning.                 |

### `pensieve recall "<text>" [--realm R] [--limit N] [--json]`

Semantic recall from the Agentic Memory layer via the MCP `recall_memory` tool
(embedding + vector search, no agent turn). Prints a compact ranked list, or the
raw structured result with `--json`. Used by the plugin to inject context.

```bash
pensieve recall "how do we handle auth tokens?" --realm pensieve --limit 8
```

### `pensieve remember "<content>" [--type T] [--realm R] [--importance F] [--topic-key K]`

Save a durable memory via the MCP `save_memory` tool. `--type` is one of
`fact | decision | preference | learning | procedure` (default `fact`). A stable
`--topic-key` makes a re-save **update in place** (deterministic upsert, no duplicate).

```bash
pensieve remember "We deploy by tagging a release, then make ship" --type procedure
pensieve remember "Prefer KQL over SQL in examples" --type preference --realm global
pensieve remember "Auth model: short-lived JWT + refresh" --topic-key arch/auth
```

### `pensieve entity "<name>" [--kind K] [--realm R] [--prop k=v]… [--link spec]…`

Create or update a **virtual graph entity** (resource) via the MCP `ingest_entity` tool
and wire it to memories + existing graph nodes. Idempotent per `(realm, kind, name)`. Each
`--link` is `node_id[|namespace[|rel]]` — pipe-delimited.

```bash
pensieve entity "payments service" --kind service --prop owner=team-pay \
  --link "repo:owner/name|github|LIVES_IN" \
  --link "memory:<uuid>||DOCUMENTED_BY"
```

### `pensieve distill [--session ID] [--realm R]`

Read a session transcript on **stdin** and hand it to the pensieve agent (which owns
`save_memory`) to extract durable memories. Used by the plugin's SessionEnd hook.

```bash
tail -n 600 transcript.jsonl | pensieve distill --realm pensieve
```

### `pensieve datasource <op>`

Manage data sources. See [Data sources → GitHub](/data-sources/github),
[GitLab](/data-sources/gitlab), [Bitbucket](/data-sources/bitbucket) for
type-specific args.

```bash
pensieve datasource list
pensieve datasource add github shakedaskayo/pensieve --start
pensieve datasource add gitlab gitlab-org/gitlab --start
pensieve datasource add bitbucket atlassian/python-bitbucket --username me --app-password $BBPW --start
pensieve datasource show gh-shakedaskayo-pensieve
pensieve datasource pause gh-shakedaskayo-pensieve
pensieve datasource resume gh-shakedaskayo-pensieve
pensieve datasource trigger gh-shakedaskayo-pensieve
pensieve datasource remove gh-shakedaskayo-pensieve
```

`<name|id>` is interchangeable — pass either the human-readable name
or the UUID. `--start` triggers an immediate first run after creating
the data source and polls until the first tick completes (or 30 s).

#### `add` ingestion knobs

For git sources (github / gitlab / bitbucket), all modules are ON by
default, including **codebase** (the structural code graph). Disable
with `--no-<module>`:

```bash
# Metadata only — skip source parsing.
pensieve datasource add github my-org/big-repo --no-codebase --start

# Constrain code parsing to two languages, with a tighter file cap.
pensieve datasource add github my-org/big-repo \
  --languages rust,go \
  --max-files 1000 \
  --max-bytes 524288 \
  --exclude 'vendor/**,**/*_test.go,dist/**' \
  --start
```

| Flag                  | What it does                                                | Default                                  |
| --------------------- | ----------------------------------------------------------- | ---------------------------------------- |
| `--no-repos`          | Skip the repos / projects module.                           | enabled                                  |
| `--no-branches`       | Skip the branches module.                                   | enabled                                  |
| `--no-pulls`          | Skip the pulls / MR module.                                 | enabled                                  |
| `--no-issues`         | Skip the issues module.                                     | enabled                                  |
| `--no-contributors`   | Skip the contributors / members module.                     | enabled                                  |
| `--no-codebase`       | Skip the structural code-graph module.                      | **enabled**                              |
| `--languages a,b,c`   | Restrict code parsing to these languages.                   | rust, python, typescript, javascript, go |
| `--max-bytes N`       | Skip files larger than N bytes.                             | 1 MiB                                    |
| `--max-files N`       | Cap on files fetched + parsed per tick.                     | 300                                      |
| `--exclude 'a,b,c'`   | Glob patterns to skip.                                      | sensible vendor/generated defaults       |
| `--max-pages N`       | API pages per module per tick (100 items/page).             | 10                                       |
| `--schedule-ms N`     | Tick interval in milliseconds.                              | 300 000 (5 min)                          |

Token discovery for `datasource add` (in order):
`--token` → `--credential-id` → `$<KIND>_TOKEN` env → `gh auth token`
shell-out (github only).

::: tip OAuth data sources
`datasource add` covers the token-auth git sources. The OAuth data sources —
[Notion](/data-sources/notion), [Google Drive](/data-sources/googledrive),
[Gmail](/data-sources/gmail), [Slack](/data-sources/slack), [Jira](/data-sources/jira),
[Confluence](/data-sources/confluence) — authenticate through the browser, so add
them from the web UI's **Connect** flow. See [OAuth data sources](/data-sources/oauth).
:::

### `pensieve ingest <op>`

Inspect ingestion runs across one or all data sources.

```bash
pensieve ingest status                     # all data sources, last-run snapshot
pensieve ingest status --datasource gh-…    # one data source
pensieve ingest tail                       # poll forever
pensieve ingest tail --datasource gh-… --interval 5
```

#### `pensieve ingest push --table T [--db D] [--idempotency-key K]`

Stream NDJSON from **stdin** straight into a table via `POST /v1/ingest`
(auto-create + schema-evolve on). This is the firehose transport the
[pensieve-memory plugin](/agent/claude-code-plugin) uses to capture conversation events.

```bash
printf '%s\n' '{"ts":"2026-05-31T12:00:00Z","kind":"note","text":"hello"}' \
  | pensieve ingest push --table claude_code_events
```

### `pensieve deploy <op>`

One-command production (and local test-drive) deployment. Manages a
workspace under `~/.pensieve/deploy/<name>/` containing the materialized
IaC templates and a `deploy.json` state file.

Targets:

| Target  | What it provisions                                          |
| ------- | ----------------------------------------------------------- |
| `aws`   | ECS Fargate engine + S3 extents + Supabase (catalog + Auth) via embedded Terraform / Pulumi. |
| `local` | Supabase project via Management API + a local Docker container (test drive). |

#### `pensieve deploy init [--name NAME] [--target aws|local] [--tool terraform|pulumi] [flags]`

Interactive wizard: collect credentials + settings, materialize the
IaC workspace. Supabase access-token resolution order:
1. `SUPABASE_ACCESS_TOKEN` env var
2. `~/.supabase/access-token` (Supabase CLI login)
3. Browser OAuth (when `PENSIEVE_SUPABASE_OAUTH_CLIENT_ID` is set)
4. Guided manual paste

| Flag              | Default      | Purpose                                                     |
| ----------------- | ------------ | ----------------------------------------------------------- |
| `--name NAME`     | `prod`       | Workspace name (`~/.pensieve/deploy/<name>/`).                  |
| `--target`        | `aws`        | `aws` or `local`.                                           |
| `--tool`          | `terraform`  | IaC tool for the aws target: `terraform` or `pulumi`.       |
| `--region`        | `us-east-1`  | AWS region.                                                 |
| `--supabase-org`  | _interactive_ | Supabase organization id (skips the interactive picker).   |
| `--domain`        | _unset_      | Custom domain for the engine (aws target).                  |
| `--admin-email`   | _interactive_ | Email(s) granted the pensieve admin role (comma-separated).    |
| `--yes`           | off          | Answer prompts with defaults (requires `--supabase-org` + a token source). |
| `--print-only`    | off          | Render the workspace + print the planned commands, run nothing. |
| `--force`         | off          | Overwrite an existing workspace's rendered config.          |

#### `pensieve deploy up [--name NAME] [--auto-approve]`

Provision: `terraform apply` / `pulumi up` (aws) or `docker run` (local).

#### `pensieve deploy status [--name NAME]`

Show deployment outputs and probe the engine's `/health`.

#### `pensieve deploy destroy [--name NAME] [--yes]`

Tear everything down (terraform destroy / docker rm + Supabase project delete).

### `pensieve update [--check] [--version TAG] [--force] [--no-restart]`

Self-update to the latest GitHub release (binary + embedded web UI),
then restart the local server so the new UI is live immediately.

| Flag           | Purpose                                                          |
| -------------- | ---------------------------------------------------------------- |
| `--check`      | Only check whether a newer release exists; don't install.        |
| `--version TAG` | Install a specific release tag (e.g. `v0.0.3`).                |
| `--force`      | Reinstall even if this version is already current.              |
| `--no-restart` | Don't restart a running `pensieve serve` after updating.            |

```bash
pensieve update --check          # is there a newer version?
pensieve update                  # update + restart local server
pensieve update --version v0.0.3 # pin a version
```

---

## Admin subcommands

These talk directly to the Postgres catalog (no running server
needed). Set `PENSIEVE_CATALOG_URL` once at the top of your shell.

### `create-database <name>`

```bash
pensieve create-database analytics
# created database analytics (a3f1...)
```

### `create-table --db <name> --name <name> --schema <spec> [--retention-days N]`

`--schema` is `col:type[, col:type, ...]`.

Types: `bool`, `int` (Int32), `long` (Int64), `real` (Float64),
`string`, `timestamp` (nanoseconds UTC), `dynamic` (JSON in Binary),
`vector(N)` (FixedSizeList<Float32, N>, non-nullable).

```bash
pensieve create-table \
  --db analytics --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
```

### `alter-table --db <name> --table <name> --add-column <spec>`

Schema only widens; there is no drop / rename / narrow.

```bash
pensieve alter-table --db analytics --table pageviews --add-column "referrer:string"
```

### `list-tables --db <name>`

```bash
pensieve list-tables --db analytics
# pageviews  [_timestamp:Timestamp(Nanosecond, None), user_id:Utf8, path:Utf8, ms:Int32, referrer:Utf8]
```

### `create-graph --db <name> --name <name> --nodes <table> --edges <table>`

Register a property-graph over two existing tables. The default
column mapping is `id`, `labels`, `src`, `dst`, `type`. Override per
column with `--id-col`, `--label-col`, etc. `--realm-col` is optional
and adds a partition dimension.

```bash
pensieve create-graph --db github --name github --nodes github_nodes --edges github_edges
```

`schema` is a reserved name (used for the synthetic schema-graph) and
rejected.

### `list-graphs --db <name>` / `drop-graph --db <name> --name <name>`

```bash
pensieve list-graphs --db github
pensieve drop-graph --db github --name old-graph
```

### `version`

Prints the package version and exits.

---

## Typical onboarding flow

### Provision-by-script (admin only)

```bash
export PENSIEVE_CATALOG_URL=postgres://pensieve:pensieve_dev@localhost:5433/pensieve

pensieve create-database analytics
pensieve create-table --db analytics --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
pensieve list-tables --db analytics
```

After this, `POST /v1/ingest` with `X-Database: analytics` and
`X-Table: pageviews` lands rows. With auto-create on (the default)
you can also skip this whole step.

### Connect a coding agent

```bash
export PENSIEVE_TOKEN=$(curl -s -XPOST http://localhost:8080/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"dev"}' | jq -r .access_token)

pensieve connect http://localhost:8080 --token "$PENSIEVE_TOKEN"
pensieve install-skill --also-link-claude
pensieve query "what databases do we have?"
```

### Ingest a GitHub repo

```bash
export GITHUB_TOKEN=ghp_xxx     # or have `gh auth status` happy
pensieve create-database github     # one-time
pensieve datasource add github shakedaskayo/pensieve --start
pensieve ingest status
```
