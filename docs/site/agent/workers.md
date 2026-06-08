# Workers & nodes

## Run a node

**Prerequisites:** a worker identity from the control plane (see [Register a worker](#register-a-worker)).

```bash
kyma worker run \
  --server https://kyma.example.com \
  --token  kyw_…
```

Environment variables are accepted in place of flags:

| Flag | Env fallback | Required |
|---|---|---|
| `--server` | `KYMA_SERVER_URL` | Yes |
| `--token` | `KYMA_WORKER_TOKEN` | Yes |
| `--name` | — | No (inherits registered name) |
| `--accept <kinds>` | — | No (default: `source_sync`) |
| `--max-concurrent <n>` | — | No |

Set `KYMA_WORKER_INSECURE=1` to allow plain `http://` to non-local control planes (not recommended in production). See [worker-daemon env vars](/reference/env#worker-daemon--node).

Once running, the daemon sends a heartbeat every 30 s. The heartbeat carries **presence** (which coding-agent sessions are active on this machine) and a **source inventory** (which agents were detected). It then claims and runs `source_sync` jobs for each detected agent.

## Register a worker

These commands require admin credentials on the control plane.

```bash
# Create a new worker identity — prints worker_id + token (shown once)
kyma worker create --name laptop-rosa

# List all registered workers with status, capabilities, and liveness
kyma worker list

# Revoke a node's token (outstanding jobs are re-queued on next sweep)
kyma worker revoke <worker_id>
```

The token prefix is `kyw_`. Tokens are stored as SHA-256 hashes server-side; there is no way to recover a lost token — revoke and re-create. See [worker-fabric HTTP endpoints](/reference/api#worker-fabric) for the raw API surface.

## What a node does

A worker on a developer machine is more than compute — it is a **node of the distributed context engine**. It:

1. Detects which coding agents are installed via a small **detector registry** (filesystem probes). Each probe reports the agent's roots, realms, and active sessions. Probes that find nothing report nothing.
2. Reports *presence* — which agent sessions are active — on every heartbeat.
3. Runs `source_sync` jobs: reads local coding-agent files and pushes them into the shared engine so memories from this machine are recallable everywhere.

Kyma is engine-agnostic. Claude Code (`~/.claude/projects`) ships with a full ingestion pipeline today. Cursor, Windsurf, and Codex are detected and appear in node inventory and capabilities; their ingestion pipelines are queued. Adding a new agent is one entry in the detector registry — see [coding agents](/agent/coding-agents).

## Job kinds

| Kind | What runs | Where it runs |
|---|---|---|
| `source_sync` | Reads local coding-agent files → pushes to engine (one job per detected agent) | Pinned to the owning node |
| `connector_sync` | Scheduled connector ingestion | Embedded worker (or any `connector`-capable worker) |
| `dreaming` | Agentic memory housekeeping | Embedded worker (capability-routed) |

Default `--accept` is `source_sync`. Pass `--accept dreaming,source_sync` to widen what a node will claim.

**Locally you get this for free:** the server runs an embedded worker in-process, so `connector_sync` and `dreaming` jobs never need an external daemon unless you want to offload them.

**Roadmap (deferred):** remote execution of `dreaming` and `connector_sync` on external daemons — the claim/lease surface already supports it; executor dispatch is the follow-up. See [Dreaming](/agent/dreaming) for the current in-process dreaming model.

## Security

- Worker tokens (`kyw_…`) are 256-bit secrets, stored as SHA-256 hashes; the plain token is shown once at creation time.
- The daemon refuses plain `http://` to non-local control planes unless `KYMA_WORKER_INSECURE=1`.
- Dead workers release their jobs via lease expiry; the control plane sweeps stale leases and marks silent workers offline (configurable via [`KYMA_FABRIC_OFFLINE_SECS`](/reference/env#worker-fabric)).
