---
title: Sync
description: How pensieve keeps memory coherent — bidirectional, incremental sync between local mode and a control plane, plus a Claude Code file-memory phase. Work offline, sync when you land; your laptop and your team converge.
---

# Sync

Memory you create on a laptop and memory on the team control plane should be the same
memory. `pensieve sync` keeps them coherent — **incrementally and in both directions** — so you
can work offline and converge when you reconnect.

## Two phases

A sync run does up to two independent things:

1. **Claude Code file memory ↔ pensieve.** Reconciles pensieve's store with Claude Code's own
   file memory (`~/.claude/projects/*/memory`, always present). What your agent wrote to
   disk becomes durable, queryable pensieve memory, and vice-versa.
2. **Local ↔ control plane.** When `PENSIEVE_CLOUD_URL` is set, pushes local memories up and
   pulls remote ones down — bidirectional and incremental, so only deltas move. The control
   plane receives and reconciles; bi-temporal validity means contradictions are superseded,
   not clobbered.

## The command

```bash
pensieve sync                       # one-shot: both phases
pensieve sync --watch               # keep running, re-syncing on an interval
pensieve sync --dry-run             # preview Claude Code file changes without writing
pensieve sync --cc-only             # only the Claude Code file phase
pensieve sync --cloud-only          # only the control-plane push/pull
```

Point it at a control plane with two environment variables:

```bash
PENSIEVE_CLOUD_URL=https://pensieve.your-co.dev \
PENSIEVE_CLOUD_TOKEN=<bearer> \
pensieve sync
```

| Variable | Default | Effect |
|---|---|---|
| `PENSIEVE_CLOUD_URL` | _unset_ | Control plane to push/pull to. Unset → local-only (Claude Code phase still runs). |
| `PENSIEVE_CLOUD_TOKEN` | _unset_ | Bearer token for the control plane. |
| `PENSIEVE_SYNC_REALM` | server default | Which memory realm to sync to/from. |
| `PENSIEVE_CC_SYNC_POLL_SECS` | `30` | Re-sync interval for `--watch` and the background service. |
| `PENSIEVE_CC_FILE_SYNC` | `1` | Set `0` to disable the Claude Code file phase entirely. |

See [Environment variables](/reference/env) for the full list and the
[CLI reference](/reference/cli) for every flag.

## When it runs for you

- **On MCP start** — the local engine fires a one-shot file sync when the MCP server
  initialises (`PENSIEVE_CC_SYNC_ON_MCP=1` by default), so a fresh session already sees prior
  memory.
- **Continuously** — `pensieve sync --watch`, or the background `pensieve serve` watcher
  (`PENSIEVE_CC_WATCH=1`), keeps reconciling on the poll interval.
- **On demand** — run `pensieve sync` whenever you want to converge, e.g. after working offline.

## Why it matters

This is what makes local mode and the [control plane](/concepts/local-and-cluster-mode)
one system instead of two. Your laptop stays fast and offline-capable; the control plane
stays the shared source of truth; sync makes them agree. Memory written on one machine shows
up on the others — and on the team's server — without anyone copying files around.

## Read next

- Where the two ends live: [Local & cluster mode](/concepts/local-and-cluster-mode).
- The whole picture: [How pensieve works](/concepts/how-pensieve-works).
- The connect paths that trigger sync: [Connect your agent](/agent/connect).
