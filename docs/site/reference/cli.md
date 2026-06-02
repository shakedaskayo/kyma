---
title: CLI
description: kyma CLI reference — client commands (connect, status, query, install-skill, connector, ingest) and admin commands (create-database, create-table, ...).
---

# CLI

`kyma` is the binary. It has two modes:

- **Client mode** — talks to a running Kyma server over HTTP. Used for
  asking the agent questions, managing connectors, and wiring up
  coding agents.
- **Admin mode** — talks directly to the Postgres catalog. Used for
  provisioning databases, tables, and graphs from scripts.

Both modes coexist in the same binary; subcommand selection picks the
mode.

## Install

```bash
cargo install --path crates/kyma-cli
kyma version
```

Binary name is `kyma` (not `kyma-cli`).

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

### `kyma install-skill [--target DIR] [--also-link-claude]`

Write a `SKILL.md` that teaches Claude Code / Cursor / Aider / etc.
how to use the `kyma` CLI as a data tool.

| Flag                  | Effect                                                                  |
| --------------------- | ----------------------------------------------------------------------- |
| `--target DIR`        | Where to write `SKILL.md`. Default `~/.kyma/skills/kyma/`.               |
| `--also-link-claude`  | Symlink `~/.claude/skills/kyma` → the target dir (Unix only).           |

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
