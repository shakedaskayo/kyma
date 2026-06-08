---
title: CLI
description: kyma CLI reference — local engine (mcp, serve, setup, sync, worker, service), client commands (connect, status, query, sessions, install-skill, connector, ingest, recall, remember, entity, distill, deploy), and admin commands (create-database, create-table, ...).
---

# CLI

`kyma` is the binary. It has three mode groups:

- **Local engine** — runs an embedded context engine (SQLite catalog +
  local object store, zero infra). Includes `kyma mcp`, `kyma serve`,
  `kyma setup`, `kyma sync`, `kyma worker`, `kyma service`.
- **Client mode** — talks to a running Kyma server over HTTP. Used for
  asking the agent questions, managing connectors, and wiring up coding
  agents.
- **Admin mode** — talks directly to the Postgres catalog. Used for
  provisioning databases, tables, and graphs from scripts.

All modes coexist in the same binary; subcommand selection picks the
mode.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash
kyma version
```

The installer downloads the prebuilt binary to `~/.local/bin` (or `/usr/local/bin`).
Binary name is `kyma` (not `kyma-cli`).

From source (contributors): inside a checkout, run `cargo install --path crates/kyma-cli`,
or use `… | bash -s -- --from-source`.

## Global flags

| Flag             | Env var              | Default                                                | Used by            |
| ---------------- | -------------------- | ------------------------------------------------------ | ------------------ |
| `--catalog-url`  | `KYMA_CATALOG_URL`   | `postgres://kyma:kyma_dev@localhost:5433/kyma`         | Admin subcommands. |

Client-mode subcommands read connection info from `~/.kyma/config.json`
(written by `kyma connect`). Two env vars override the file:

| Var               | Effect                                  |
| ----------------- | --------------------------------------- |
| `KYMA_SERVER_URL` | Overrides the saved endpoint.           |
| `KYMA_TOKEN`      | Overrides the saved bearer token.       |

---

## Local engine subcommands

These run the embedded context engine — no Postgres or S3 required.
Data lives under `~/.kyma/` by default (override with `KYMA_HOME`).

### `kyma mcp`

Serve the Model Context Protocol over **stdio** — what a coding agent
spawns automatically via the MCP config written by `kyma setup`. Starts
the embedded engine, runs an initial Claude Code file-memory sync, then
serves the full context-engine toolset (memory + data + graph) over the
MCP protocol.

```bash
kyma mcp   # run directly to test; agents invoke this as the MCP server command
```

No flags. The agent connects via stdin/stdout. See /reference/mcp for the
full tool list.

### `kyma serve [--addr ADDR]`

Serve the web UI + HTTP API locally (query / catalog / graph / ingest /
MCP). Browse the graph and ingest on demand. Sign-in defaults:
`admin` / `admin`.

| Flag     | Env var               | Default             |
| -------- | --------------------- | ------------------- |
| `--addr` | `KYMA_LOCAL_HTTP_ADDR` | `127.0.0.1:7777`   |

```bash
kyma serve
kyma serve --addr 0.0.0.0:8888
```

### `kyma setup <agent|list> [--print]`

Wire a coding agent to `kyma mcp` over stdio with a one-liner. Writes
(merging, never clobbering other servers) the agent's MCP config file.

Usage: `kyma setup <agent>` where `<agent>` is one of:

| Agent key      | Config file written                         | Scope          |
| -------------- | ------------------------------------------- | -------------- |
| `claude-code`  | `.mcp.json`                                 | project (cwd)  |
| `cursor`       | `.cursor/mcp.json`                          | project (cwd)  |
| `windsurf`     | `~/.codeium/windsurf/mcp_config.json`       | global (home)  |

`kyma setup list` prints the supported agent keys. Any unknown agent
key emits a generic stdio MCP snippet to paste manually.

`--print` previews the resulting config without writing it.

```bash
kyma setup claude-code        # write .mcp.json in the current project
kyma setup cursor --print     # preview only
kyma setup list               # show supported agents
```

### `kyma sync [--watch] [--dry-run] [--cc-only] [--cloud-only] [--project PATH]`

Sync memory with Claude Code's file memory (`~/.claude/projects/*/memory`,
always present) and bidirectionally with a control plane (when
`KYMA_CLOUD_URL` is set).

The file phase ingests + embeds Claude Code memory files, promotes
high-value kyma memories back as native files, and curates `MEMORY.md`.

| Flag            | Purpose                                                                 |
| --------------- | ----------------------------------------------------------------------- |
| `--watch`       | Keep running, re-syncing on an interval (`KYMA_CC_SYNC_POLL_SECS`, default 30s). |
| `--dry-run`     | Plan + audit-log Claude Code file changes without writing them. Ingestion into the local store still runs. |
| `--cc-only`     | Only the Claude Code file phase (skip the control plane).               |
| `--cloud-only`  | Only the control-plane push/pull (skip Claude Code files).              |
| `--project PATH` | Limit the file phase to one project path.                              |

```bash
kyma sync                       # one-shot sync
kyma sync --watch               # background loop
kyma sync --dry-run             # preview file changes
```

### `kyma worker <action>`

**Two distinct worlds under one subcommand:**

#### Background sync service (`install | uninstall | status`)

Manage an OS user service (launchd on macOS, systemd --user on Linux)
running `kyma sync --watch` so memory stays synced with no terminal open.

```bash
kyma worker install [--interval SECS] [--cc-only] [--cloud-only]
kyma worker uninstall
kyma worker status
```

| Flag           | Purpose                                        |
| -------------- | ---------------------------------------------- |
| `--interval N` | Poll interval in seconds (default 30; sets `KYMA_CC_SYNC_POLL_SECS`). |
| `--cc-only`    | Only Claude Code file phase.                   |
| `--cloud-only` | Only control-plane push/pull.                  |

#### Fabric node daemon + admin (`run | create | list | revoke`)

Register this machine as a worker node with the control plane, pull
and run jobs (connector syncs, dreaming tasks, etc.).

```bash
# Start the node daemon
kyma worker run --server <URL> --token <TOKEN> \
  [--accept source_sync,dreaming] [--max-concurrent 2] [--name my-box]

# Admin: mint a new worker identity
kyma worker create --name my-box [--capabilities sources,dreaming]

# Admin: list registered nodes
kyma worker list

# Admin: revoke a node
kyma worker revoke <worker-id>
```

`kyma worker run` flags:

| Flag               | Env var              | Default           | Purpose                                          |
| ------------------ | -------------------- | ----------------- | ------------------------------------------------ |
| `--server URL`     | `KYMA_SERVER_URL`    | _required_        | Control-plane URL.                               |
| `--token TOKEN`    | `KYMA_WORKER_TOKEN`  | _required_        | Worker auth token from `kyma worker create`.     |
| `--accept LIST`    | —                    | `source_sync`     | Comma-separated job kinds this node accepts.     |
| `--max-concurrent N` | —                  | `1`               | Max jobs running in parallel.                    |
| `--name NAME`      | —                    | `node@<hostname>` | Friendly node name shown in `kyma worker list`.  |

### `kyma service <action>`

Manage the local Kyma server (`kyma serve`) as an OS user service (launchd
on macOS, systemd --user on Linux): starts at login, restarts on crash.

```bash
kyma service install [--addr ADDR] [--token TOKEN]
kyma service uninstall
kyma service status
```

| Flag       | Purpose                                                                   |
| ---------- | ------------------------------------------------------------------------- |
| `--addr`   | Listen address (default `127.0.0.1:7777`).                               |
| `--token`  | Static admin token (`KYMA_AUTH_TOKENS=<token>:admin` in the service env). |

---

## Client subcommands

### `kyma connect <url> [--token TOKEN]`

Save a connection to a running Kyma server. Writes
`~/.kyma/config.json` (mode `0600`).

```bash
kyma connect http://localhost:8080 --token "$(curl -s -XPOST ... | jq -r .access_token)"
```

### `kyma status`

Show the saved endpoint, whether a token is configured, and probe
`/health`.

```bash
kyma status
# Endpoint:  http://localhost:8080
# Token:     configured
# Health:    {"status":"ok","version":"0.0.1"}
```

### `kyma query "<question>" [--json] [--session ID] [--continue]`

Stream `/v1/agent/ask` to stdout. Without `--json`, prints the
`answer_delta` / `answer_final` text directly. With `--json`, emits
the raw SSE event stream as JSONL — useful for scripting.

`--session ID` resumes a specific conversation session; `--continue` resumes the
most recent one `query` used (kyma records the last session id locally), so
follow-up questions keep context.

```bash
kyma query "how many rows in github_nodes?"
kyma query "any error logs from prod-api in the last 15 minutes?"
kyma query "and how many of those were 500s?" --continue
kyma query "list databases" --json | jq -c '.event,.data'
```

### `kyma sessions <op>`

Inspect or manage the agent's conversation sessions (each `query` turn belongs to
one). Talks to `/v1/agent/sessions`.

```bash
kyma sessions list              # recent sessions
kyma sessions show <id>         # metadata + rolling summary
kyma sessions turns <id>        # every turn in order
kyma sessions delete <id>       # delete a session and all its turns
```

### `kyma user <op>` — requires an admin token

Manage users over HTTP (`/v1/admin/users`). The configured token must have the
**admin** role. Passwords are read interactively unless `--password-stdin`.

```bash
kyma user list
kyma user create alice --role write             # role: read | write | admin (default read)
kyma user create bot --role read --password-stdin <<<"$PW"
kyma user passwd alice                          # reset a password
kyma user set-role alice admin                  # change role
kyma user delete alice --yes                    # --yes skips the confirm prompt
```

### `kyma install-skill [--target DIR] [--also-link-claude] [--which kyma|deploy|all]`

Write a `SKILL.md` that teaches Claude Code / Cursor / Aider / etc.
how to use the `kyma` CLI as a data tool.

| Flag                  | Effect                                                                  |
| --------------------- | ----------------------------------------------------------------------- |
| `--target DIR`        | Where to write `SKILL.md`. Default `~/.kyma/skills/<skill>/`.           |
| `--also-link-claude`  | Symlink `~/.claude/skills/kyma` → the target dir (Unix only).           |
| `--which`             | `kyma` (default) — the data/query skill; `deploy` — the production-deployment runbook; `all` — both. |

### `kyma install-plugin [--target DIR] [--force]`

Install the [kyma-memory Claude Code plugin](/agent/claude-code-plugin) —
hooks + a bundled MCP server + slash commands — into
`~/.claude/skills/kyma-memory/`. Templates your saved server URL + token into
the plugin's `.mcp.json` so it works immediately. Restart Claude Code, then run
`/kyma-status`.

| Flag           | Effect                                                              |
| -------------- | ------------------------------------------------------------------ |
| `--target DIR` | Install into `DIR` instead of `~/.claude/skills/kyma-memory`.       |
| `--force`      | Overwrite an existing install without the warning.                 |

### `kyma recall "<text>" [--realm R] [--limit N] [--json]`

Semantic recall from the Agentic Memory layer via the MCP `recall_memory` tool
(embedding + vector search, no agent turn). Prints a compact ranked list, or the
raw structured result with `--json`. Used by the plugin to inject context.

```bash
kyma recall "how do we handle auth tokens?" --realm kyma --limit 8
```

### `kyma remember "<content>" [--type T] [--realm R] [--importance F] [--topic-key K]`

Save a durable memory via the MCP `save_memory` tool. `--type` is one of
`fact | decision | preference | learning | procedure` (default `fact`). A stable
`--topic-key` makes a re-save **update in place** (deterministic upsert, no duplicate).

```bash
kyma remember "We deploy by tagging a release, then make ship" --type procedure
kyma remember "Prefer KQL over SQL in examples" --type preference --realm global
kyma remember "Auth model: short-lived JWT + refresh" --topic-key arch/auth
```

### `kyma entity "<name>" [--kind K] [--realm R] [--prop k=v]… [--link spec]…`

Create or update a **virtual graph entity** (resource) via the MCP `ingest_entity` tool
and wire it to memories + existing graph nodes. Idempotent per `(realm, kind, name)`. Each
`--link` is `node_id[|namespace[|rel]]` — pipe-delimited.

```bash
kyma entity "payments service" --kind service --prop owner=team-pay \
  --link "repo:owner/name|github|LIVES_IN" \
  --link "memory:<uuid>||DOCUMENTED_BY"
```

### `kyma distill [--session ID] [--realm R]`

Read a session transcript on **stdin** and hand it to the kyma agent (which owns
`save_memory`) to extract durable memories. Used by the plugin's SessionEnd hook.

```bash
tail -n 600 transcript.jsonl | kyma distill --realm kyma
```

### `kyma connector <op>`

Manage connectors. See [Connectors → GitHub](/connectors/github),
[GitLab](/connectors/gitlab), [Bitbucket](/connectors/bitbucket) for
type-specific args.

```bash
kyma connector list
kyma connector add github shakedaskayo/kyma --start
kyma connector add gitlab gitlab-org/gitlab --start
kyma connector add bitbucket atlassian/python-bitbucket --username me --app-password $BBPW --start
kyma connector show gh-shakedaskayo-kyma
kyma connector pause gh-shakedaskayo-kyma
kyma connector resume gh-shakedaskayo-kyma
kyma connector trigger gh-shakedaskayo-kyma
kyma connector remove gh-shakedaskayo-kyma
```

`<name|id>` is interchangeable — pass either the human-readable name
or the UUID. `--start` triggers an immediate first run after creating
the connector and polls until the first tick completes (or 30 s).

#### `add` ingestion knobs

For git sources (github / gitlab / bitbucket), all modules are ON by
default, including **codebase** (the structural code graph). Disable
with `--no-<module>`:

```bash
# Metadata only — skip source parsing.
kyma connector add github my-org/big-repo --no-codebase --start

# Constrain code parsing to two languages, with a tighter file cap.
kyma connector add github my-org/big-repo \
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

Token discovery for `connector add` (in order):
`--token` → `--credential-id` → `$<KIND>_TOKEN` env → `gh auth token`
shell-out (github only).

::: tip OAuth connectors
`connector add` covers the token-auth git sources. The OAuth connectors —
[Notion](/connectors/notion), [Google Drive](/connectors/googledrive),
[Gmail](/connectors/gmail), [Slack](/connectors/slack), [Jira](/connectors/jira),
[Confluence](/connectors/confluence) — authenticate through the browser, so add
them from the web UI's **Connect** flow. See [OAuth connectors](/connectors/oauth).
:::

### `kyma ingest <op>`

Inspect ingestion runs across one or all connectors.

```bash
kyma ingest status                     # all connectors, last-run snapshot
kyma ingest status --connector gh-…    # one connector
kyma ingest tail                       # poll forever
kyma ingest tail --connector gh-… --interval 5
```

#### `kyma ingest push --table T [--db D] [--idempotency-key K]`

Stream NDJSON from **stdin** straight into a table via `POST /v1/ingest`
(auto-create + schema-evolve on). This is the firehose transport the
[kyma-memory plugin](/agent/claude-code-plugin) uses to capture conversation events.

```bash
printf '%s\n' '{"ts":"2026-05-31T12:00:00Z","kind":"note","text":"hello"}' \
  | kyma ingest push --table claude_code_events
```

### `kyma deploy <op>`

One-command production (and local test-drive) deployment. Manages a
workspace under `~/.kyma/deploy/<name>/` containing the materialized
IaC templates and a `deploy.json` state file.

Targets:

| Target  | What it provisions                                          |
| ------- | ----------------------------------------------------------- |
| `aws`   | ECS Fargate engine + S3 extents + Supabase (catalog + Auth) via embedded Terraform / Pulumi. |
| `local` | Supabase project via Management API + a local Docker container (test drive). |

#### `kyma deploy init [--name NAME] [--target aws|local] [--tool terraform|pulumi] [flags]`

Interactive wizard: collect credentials + settings, materialize the
IaC workspace. Supabase access-token resolution order:
1. `SUPABASE_ACCESS_TOKEN` env var
2. `~/.supabase/access-token` (Supabase CLI login)
3. Browser OAuth (when `KYMA_SUPABASE_OAUTH_CLIENT_ID` is set)
4. Guided manual paste

| Flag              | Default      | Purpose                                                     |
| ----------------- | ------------ | ----------------------------------------------------------- |
| `--name NAME`     | `prod`       | Workspace name (`~/.kyma/deploy/<name>/`).                  |
| `--target`        | `aws`        | `aws` or `local`.                                           |
| `--tool`          | `terraform`  | IaC tool for the aws target: `terraform` or `pulumi`.       |
| `--region`        | `us-east-1`  | AWS region.                                                 |
| `--supabase-org`  | _interactive_ | Supabase organization id (skips the interactive picker).   |
| `--domain`        | _unset_      | Custom domain for the engine (aws target).                  |
| `--admin-email`   | _interactive_ | Email(s) granted the kyma admin role (comma-separated).    |
| `--yes`           | off          | Answer prompts with defaults (requires `--supabase-org` + a token source). |
| `--print-only`    | off          | Render the workspace + print the planned commands, run nothing. |
| `--force`         | off          | Overwrite an existing workspace's rendered config.          |

#### `kyma deploy up [--name NAME] [--auto-approve]`

Provision: `terraform apply` / `pulumi up` (aws) or `docker run` (local).

#### `kyma deploy status [--name NAME]`

Show deployment outputs and probe the engine's `/health`.

#### `kyma deploy destroy [--name NAME] [--yes]`

Tear everything down (terraform destroy / docker rm + Supabase project delete).

### `kyma update [--check] [--version TAG] [--force] [--no-restart]`

Self-update to the latest GitHub release (binary + embedded web UI),
then restart the local server so the new UI is live immediately.

| Flag           | Purpose                                                          |
| -------------- | ---------------------------------------------------------------- |
| `--check`      | Only check whether a newer release exists; don't install.        |
| `--version TAG` | Install a specific release tag (e.g. `v0.0.3`).                |
| `--force`      | Reinstall even if this version is already current.              |
| `--no-restart` | Don't restart a running `kyma serve` after updating.            |

```bash
kyma update --check          # is there a newer version?
kyma update                  # update + restart local server
kyma update --version v0.0.3 # pin a version
```

---

## Admin subcommands

These talk directly to the Postgres catalog (no running server
needed). Set `KYMA_CATALOG_URL` once at the top of your shell.

### `create-database <name>`

```bash
kyma create-database analytics
# created database analytics (a3f1...)
```

### `create-table --db <name> --name <name> --schema <spec> [--retention-days N]`

`--schema` is `col:type[, col:type, ...]`.

Types: `bool`, `int` (Int32), `long` (Int64), `real` (Float64),
`string`, `timestamp` (nanoseconds UTC), `dynamic` (JSON in Binary),
`vector(N)` (FixedSizeList<Float32, N>, non-nullable).

```bash
kyma create-table \
  --db analytics --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
```

### `alter-table --db <name> --table <name> --add-column <spec>`

Schema only widens; there is no drop / rename / narrow.

```bash
kyma alter-table --db analytics --table pageviews --add-column "referrer:string"
```

### `list-tables --db <name>`

```bash
kyma list-tables --db analytics
# pageviews  [_timestamp:Timestamp(Nanosecond, None), user_id:Utf8, path:Utf8, ms:Int32, referrer:Utf8]
```

### `create-graph --db <name> --name <name> --nodes <table> --edges <table>`

Register a property-graph over two existing tables. The default
column mapping is `id`, `labels`, `src`, `dst`, `type`. Override per
column with `--id-col`, `--label-col`, etc. `--realm-col` is optional
and adds a partition dimension.

```bash
kyma create-graph --db github --name github --nodes github_nodes --edges github_edges
```

`schema` is a reserved name (used for the synthetic schema-graph) and
rejected.

### `list-graphs --db <name>` / `drop-graph --db <name> --name <name>`

```bash
kyma list-graphs --db github
kyma drop-graph --db github --name old-graph
```

### `version`

Prints the package version and exits.

---

## Typical onboarding flow

### Provision-by-script (admin only)

```bash
export KYMA_CATALOG_URL=postgres://kyma:kyma_dev@localhost:5433/kyma

kyma create-database analytics
kyma create-table --db analytics --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
kyma list-tables --db analytics
```

After this, `POST /v1/ingest` with `X-Database: analytics` and
`X-Table: pageviews` lands rows. With auto-create on (the default)
you can also skip this whole step.

### Connect a coding agent

```bash
export KYMA_TOKEN=$(curl -s -XPOST http://localhost:8080/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"dev"}' | jq -r .access_token)

kyma connect http://localhost:8080 --token "$KYMA_TOKEN"
kyma install-skill --also-link-claude
kyma query "what databases do we have?"
```

### Ingest a GitHub repo

```bash
export GITHUB_TOKEN=ghp_xxx     # or have `gh auth status` happy
kyma create-database github     # one-time
kyma connector add github shakedaskayo/kyma --start
kyma ingest status
```
