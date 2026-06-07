# Dreaming

Dreaming is kyma's scheduled, agentic memory housekeeping: an autonomous
agent run — using your configured engine (Claude Code CLI, Anthropic, OpenAI,
or Ollama) — that reviews recent activity, fills gaps with **read-only**
connector access, and keeps the context graph healthy.

It is **off by default**. The always-on, deterministic consolidation pipeline
(cheap firehose rollups) keeps running either way; dreaming is the
intelligence layer on top.

## What a dreaming run does

A run works in phases:

1. **Review** — surveys recent memories and the coding-agent activity
   firehose (the `claude_code_events` table — events streamed from the coding
   agents connected to your nodes) plus memory files synced from those agents.
2. **Gap-fill** — when a memory references something with missing context,
   the agent uses `list_connectors` + `connector_read` to fetch fresh,
   read-only context from your sources (a GitHub README/file/issue, a
   SELECT against a connected Postgres) — authenticated through the
   connector's stored credential, budgeted per run.
3. **Graph wiring & entity maintenance** — the core: it links memories to
   the deterministic resources they're about (connector-ingested repos,
   tables, services), creates and *maintains* logical entities (a service,
   a person, a concept) with `ingest_entity` (idempotent — entities are
   refreshed, not duplicated), relates entities with meaningful edge types,
   re-scores importance, merges duplicates, supersedes contradictions
   (bi-temporal — never a hard delete), and archives stale memories.
4. **Summary** — the final message becomes the run's report in the UI.

Every run records its full conversation — including each tool call — so you
can drill into exactly what the agent did from **Memory → Dreaming**.

## Enabling

Settings → Memory → Dreaming (or `PUT /v1/agent/memory/settings`):

| Knob | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Master switch |
| `interval_secs` | `86400` | Schedule cadence |
| `mode` | `full` | `full` \| `housekeeping_only` (no connector reads) \| `sources` |
| `max_tool_calls` | `100` | Agent-loop budget (adk engines) |
| `wall_clock_secs` | `600` | Hard wall-clock; runaway CLI agents are killed |
| `connector_read_budget` | `25` | Max source reads per run |
| `mutation_cap` | `60` | Max memory mutations per run |

Trigger a run on demand with **Dream now**, or:

```bash
curl -X POST $KYMA/v1/agent/memory/dreaming/run \
  -H 'content-type: application/json' -d '{"mode":"full"}'
```

Only one dreaming run is in flight at a time; extra triggers dedupe.

## Execution: the worker fabric

Dreaming runs are `dreaming` **jobs** on the worker fabric. The scheduler
only enqueues; a worker claims the job under a lease and executes it. The
server's embedded worker handles them by default; remote workers that
advertise the right capabilities can take them too.

When the engine is the Claude CLI, the headless agent is sandboxed: it sees
**only kyma's MCP server** (`--strict-mcp-config`), runs in a scratch
directory, and is killed at the wall-clock budget.

## Safety

- Connector access is read-only by construction (per-kind operation
  allowlists; SELECT-only SQL with an injected LIMIT; call + byte budgets).
- Memory mutations are bi-temporal: merging archives the source and records
  a `MERGED_INTO` edge; superseding sets `invalid_at`; nothing is deleted.
- Every mutation is tallied into the run's stats and visible in the
  drilldown conversation.
