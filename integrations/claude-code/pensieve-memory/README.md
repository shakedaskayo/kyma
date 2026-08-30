# pensieve-memory — realtime memory for Claude Code

Turns Claude Code into a live source **and** consumer of your [Pensieve](https://github.com/agentcylabs/pensieve)
memory layer. As a session unfolds it:

- **Captures** the meaningful points of the conversation (prompts, tool actions,
  assistant turns) into a KQL/SQL-queryable firehose table, in realtime.
- **Recalls** relevant durable memories on every prompt and injects them as context.
- **Distills** durable memories (decisions, preferences, learnings) at session end, so
  later sessions — in any tool wired to the same Pensieve — start with the right context.

It bundles a Pensieve **MCP server**, so the model can also `recall_memory`, `save_memory`,
and run `run_kql` / `run_sql` against your data directly.

## Install

You need the `pensieve` CLI on your PATH and a connection to your Pensieve server:

```bash
pensieve connect https://your-pensieve.example.com --token <token>
pensieve install-plugin            # writes the plugin into ~/.claude/skills/pensieve-memory
```

Restart Claude Code. Run `/pensieve-status` to confirm it's connected and capturing.

`pensieve install-plugin` materializes this plugin (templating your server URL + token into
`.mcp.json`) so both the hooks and the bundled MCP server work out of the box.

## What gets captured

The firehose lands in `default.claude_code_events` (auto-created, schema-evolving):

| column | meaning |
|---|---|
| `ts`, `session_id`, `realm` | when / which session / which project |
| `kind` | `session_start` · `user_prompt` · `tool_use` · `assistant_turn` · `session_end` |
| `role`, `text`, `text_len` | message content (redacted; omitted in `metadata` mode) |
| `tool_name`, `tool_input`, `tool_response_preview` | for `tool_use` events |
| `cwd`, `source` | working dir, session source |

Query it like any Pensieve table — it also shows up in Discover and the graph:

```kql
claude_code_events
| where realm == "pensieve"
| summarize events = count() by kind
| order by events desc
```

## File-memory sync (Claude Code's `memory/` dirs)

Claude Code keeps its own file-based memory per project
(`~/.claude/projects/<slug>/memory/`: a `MEMORY.md` index + one `.md` per
memory). Pensieve syncs with it in both directions — automatically, at session
start/end and at `pensieve mcp` startup (plus `pensieve sync --watch` for a resident
loop):

- **Ingest** (files are source of truth): every memory file is embedded and
  upserted into pensieve's memory graph (`[[wikilinks]]` become edges); edits
  become new versions, deletions archive the node, renames are tracked.
- **Promote**: high-value pensieve memories (importance ≥ 0.6, capped at 15
  index entries with hysteresis) are written back as native `pensieve-*.md`
  files Claude Code loads — frontmatter-stamped so they never re-ingest.
- **Curate**: superseded/duplicate/stale files are *archived* to
  `memory/archive/` with a tombstone (never deleted); `MEMORY.md` gets a
  surgical `<!-- BEGIN/END pensieve-managed -->` region — your own entries are
  never touched, and files you list yourself are never double-indexed.
- **Your edits win**: hand-edit a pensieve-promoted file and pensieve pulls the edit
  back into the store and stops rewriting that file.

Writeback is atomic (temp+rename), lock-guarded, defers while a session is
active, and audit-logs every action to `~/.pensieve/cc-curation.log`. Try
`pensieve sync --cc-only --dry-run` to see the plan without writing.

Want it running with no session or terminal at all? Install the optional
background worker — an OS user service (launchd on macOS, systemd `--user`
on Linux) running the same sync loop:

```bash
pensieve worker install            # or: --interval 60 --cc-only
pensieve worker status             # installed? running? where it logs
pensieve worker uninstall          # fully reversible
```

## Slash commands

- `/pensieve-recall <query>` — semantic recall from your durable memory.
- `/pensieve-remember [note]` — save a memory, or distill the recent conversation.
- `/pensieve-ask <question>` — ask Pensieve about your data (logs/traces/tables/graph) in KQL/SQL.
- `/pensieve-ingest [source | resources]` — pull from a data source on demand, or create virtual
  graph entities wired to memory and existing resources.
- `/pensieve-status` — connection, capture mode, and this project's event breakdown.

## Configuration (environment variables)

All optional — sensible defaults make it work with zero config.

| var | default | meaning |
|---|---|---|
| `PENSIEVE_CC_CAPTURE` | `full` | `off` · `metadata` (no message/tool bodies) · `full` |
| `PENSIEVE_CC_AUTO_RECALL` | `1` | inject recalled memories as context each prompt |
| `PENSIEVE_CC_DISTILL` | `1` | distill durable memories at session end |
| `PENSIEVE_CC_REALM` | project dir basename | memory/firehose namespace |
| `PENSIEVE_CC_DB` / `PENSIEVE_CC_TABLE` | `default` / `claude_code_events` | firehose target |
| `PENSIEVE_CC_RECALL_LIMIT` | `5` | memories injected per recall |
| `PENSIEVE_CC_MAXLEN` | `4000` | max bytes of any captured text field |
| `PENSIEVE_CC_NO_REDACT` | unset | set to `1` to disable secret redaction (not recommended) |
| `PENSIEVE_CC_FILE_SYNC` | `1` | sync Claude Code's `memory/` file dirs |
| `PENSIEVE_CC_CURATE` | `1` | curation + writeback (archive/promote/index) |
| `PENSIEVE_CC_PROMOTE` | `1` | promote high-value pensieve memories to native files |
| `PENSIEVE_CC_PROMOTE_MAX` | `15` | hard cap on pensieve-managed `MEMORY.md` entries |
| `PENSIEVE_CC_PROMOTE_MIN_IMPORTANCE` | `0.6` | promotion importance floor |
| `PENSIEVE_CC_STALE_DAYS` | `90` | LLM stale-review age gate (needs a usable engine) |
| `PENSIEVE_CC_DUP_COSINE` | `0.97` | duplicate-merge similarity threshold |
| `PENSIEVE_CC_QUIET_WINDOW` | `300` | seconds a project session blocks writeback |
| `PENSIEVE_CC_SYNC_POLL_SECS` | `30` | `pensieve sync --watch` poll interval |
| `PENSIEVE_CC_SYNC_ON_MCP` | `1` | opportunistic file sync at `pensieve mcp` startup |
| `PENSIEVE_CC_HOME` | `~/.claude` | Claude Code home override |

## Privacy

In `full` mode, conversation content (prompts, tool I/O previews, assistant replies) is
sent to **your own** Pensieve server. Common secret shapes (API keys, tokens, JWTs, private
keys) are redacted before egress, but redaction is best-effort — use `PENSIEVE_CC_CAPTURE=metadata`
if you want event shape without bodies, or `off` to disable capture entirely. All hooks
fail open: if Pensieve is unreachable, the session is never blocked.

## Uninstall

```bash
rm -rf ~/.claude/skills/pensieve-memory
```

(or wherever you installed it via `--target`), then restart Claude Code.
