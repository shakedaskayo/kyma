# Workers & nodes

kyma's background work — connector syncs, dreaming runs, local source syncs —
rides a **worker fabric**: jobs in the control plane, claimed under leases by
workers. Locally you get this for free: the server runs an *embedded worker*
in-process. In production you can register additional workers on any compute.

A worker on a developer machine is more than compute: it is a **node of the
distributed context engine**. It owns local sources — a coding agent's memory
files and transcripts — that only it can read, reports *presence* (which
coding-agent sessions are active there), and syncs that raw material into the
shared engine, so memories captured on one machine are recallable from every
other.

Kyma is engine-agnostic: a small **detector registry** decides which coding
agents live on a node. Each detector is a cheap filesystem probe that reports
the agent's roots, realms, and active sessions; detectors that find nothing
report nothing. Claude Code (`~/.claude/projects`) ships with a full ingestion
pipeline today; Cursor, Windsurf, and Codex are detected (so they show up in
node inventory and capabilities) ahead of their pipelines landing. Adding an
agent is one entry in the registry — see `kyma-local::agent_sources`.

## Quick start

Mint a worker identity on the control plane (admin):

```bash
kyma worker create --name laptop-rosa
# → worker_id + a kyw_… token (shown once)
```

Run the node daemon on that machine:

```bash
kyma worker run --server https://kyma.example.com --token kyw_…
```

By default the daemon is **low-impact**: it accepts only `source_sync` jobs
(reading the machine's own coding-agent files — one per detected agent),
self-scheduled and pinned to itself. One job at a time; a 30s heartbeat carries
presence + its source inventory. The daemon advertises the generic `sources`
capability plus a per-agent `source:<kind>` (e.g. `source:claude-code`) for
each agent it detects.

See every node, its sources, presence, and liveness:

```bash
kyma worker list            # or GET /v1/workers, or the Nodes strip in the UI
```

Revoke a node's token:

```bash
kyma worker revoke <worker_id>
```

## Job kinds

| Kind | What | Where it runs |
|---|---|---|
| `connector_sync` | Scheduled connector ingestion | Embedded worker (any `connector`-capable worker) |
| `dreaming` | Agentic memory housekeeping | Embedded worker (capability-routed) |
| `source_sync` | A node's local coding-agent files → the engine (one per detected agent) | Pinned to the owning node |

Remote execution of `dreaming` and `connector_sync` on daemons is a planned
follow-up (the claim/lease surface already supports it).

## Security notes

- Worker tokens (`kyw_…`) are 256-bit secrets stored as SHA-256 hashes; the
  daemon refuses plain `http://` to non-local control planes unless
  `KYMA_WORKER_INSECURE=1`.
- Dead workers release their jobs via lease expiry; the control plane sweeps
  stale leases and marks silent workers offline.
