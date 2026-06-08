# Dreaming

**Dreaming** is scheduled, agentic memory housekeeping — off by default. It runs an autonomous agent (using your configured engine) that reviews recent memories, fills gaps with read-only connector access, and keeps the context graph healthy.

The always-on deterministic consolidation pipeline (cheap firehose rollups) keeps running regardless; dreaming is the intelligence layer on top.

---

## Enable dreaming

Dreaming is disabled until you flip `enabled`. Set it via the UI (**Settings → Memory → Dreaming**) or the API:

```bash
curl -X PUT $KYMA/v1/agent/memory/settings \
  -H 'content-type: application/json' \
  -d '{"dreaming":{"enabled":true}}'
```

All dreaming knobs live under the `dreaming` key in `/v1/agent/memory/settings`.

---

## Knobs

| Knob | Default | Controls |
|---|---|---|
| `enabled` | `false` | Master switch — dreaming never runs unless `true` |
| `interval_secs` | `86400` | Schedule cadence in seconds (default: daily) |
| `mode` | `"full"` | `full` \| `housekeeping_only` (no connector reads) \| `sources` |
| `realm_scope` | `[]` | Realms in scope; empty list = all realms |
| `max_tool_calls` | `100` | Agent-loop budget per run (adk engines) |
| `wall_clock_secs` | `600` | Hard wall-clock limit; runaway CLI agents are killed |
| `connector_read_budget` | `25` | Max connector reads per run |
| `connector_read_max_bytes` | `4194304` | Max bytes fetched across all connector reads (4 MiB) |
| `mutation_cap` | `60` | Max memory mutations per run (save / merge / archive / judge) |

---

## Trigger a run

Trigger on demand from the UI (**Memory → Dreaming → Dream now**) or via API:

```bash
curl -X POST $KYMA/v1/agent/memory/dreaming/run \
  -H 'content-type: application/json' \
  -d '{"mode":"full"}'
```

**Request body** (all optional):

| Field | Type | Description |
|---|---|---|
| `mode` | string | Override the scheduled mode for this run only |
| `focus` | string | Hint folded into the prompt (a realm name, a connector, …) |

**Responses:**

- `202 Accepted` — new run queued; body: `{"job_id": "<uuid>"}`
- `200 OK` — a run is already in flight; body: `{"deduped": true, "detail": "a dreaming run is already in flight"}`

Only one dreaming run is in flight at a time; concurrent triggers deduplicate.

**History and drilldown:**

```
GET /v1/agent/memory/dreaming/runs        # list runs
GET /v1/agent/memory/dreaming/runs/:id    # single run + activity feed
```

The web UI at `/memory/dreaming` shows the run list and a per-run activity feed (every tool call the agent made).

See [API reference](/reference/api) for full endpoint shapes.

---

## What a run does

A run works in four phases, bounded by the wall-clock and budget knobs above:

1. **Review** — surveys recent memories and the coding-agent activity firehose (`claude_code_events` table — events streamed from agents connected to your nodes) plus memory files synced from those agents.
2. **Gap-fill** — when a memory references something with missing context, the agent calls `list_connectors` + `connector_read` to fetch fresh, read-only context from your sources (a GitHub README/file/issue, a SELECT against a connected Postgres) — authenticated through the connector's stored credential, budgeted by `connector_read_budget` and `connector_read_max_bytes`. Skipped when `mode` is `housekeeping_only`.
3. **Graph wiring & entity maintenance** — the core: links memories to deterministic resources they're about (connector-ingested repos, tables, services), creates and maintains logical entities (`ingest_entity` is idempotent — entities are refreshed, not duplicated), relates entities with meaningful edge types, re-scores importance, merges duplicates, supersedes contradictions (bi-temporal — never a hard delete), and archives stale memories.
4. **Summary** — the final agent message becomes the run's report; outcome counters (memories created/merged/archived, connector reads, tool calls) are stored on the run record.

---

## Limits and safety

- **Connector access is read-only by construction** — per-kind operation allowlists; SELECT-only SQL with an injected LIMIT; call and byte budgets.
- **Memory mutations are bi-temporal** — merging archives the source and records a `MERGED_INTO` edge; superseding sets `invalid_at`; nothing is hard-deleted.
- **Every mutation is tallied** — visible in the run's stats and in the drilldown conversation.

---

## Execution: the worker fabric

Dreaming runs are `dreaming` **jobs** on the worker fabric. The scheduler only enqueues; a worker claims the job under a lease and executes it. The server's embedded worker handles them by default.

When the engine is the Claude CLI, the headless agent is sandboxed: it sees **only kyma's MCP server** (`--strict-mcp-config`), runs in a scratch directory, and is killed at `wall_clock_secs`.

> **Roadmap:** Remote execution of dreaming on daemon workers is planned. The fabric claim/lease surface already supports it; executor dispatch is the follow-up work.

See [Workers](/agent/workers) for the fabric that executes runs and [Memory settings](/agent/memory) for the full settings context.
