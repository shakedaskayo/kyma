---
title: How kyma works
description: The whole system in one page — the memory → data → graph loop, how your agent connects (plugin/CLI/MCP), the columnar engine underneath, continuous ingestion, background consolidation and dreaming, the two runtime modes, and sync.
---

# How kyma works

kyma has a lot of surface area, but the mental model is small. This page is the map:
how the pieces fit, in the order you meet them.

## The loop: memory → data → graph

Everything kyma does serves one loop — give a coding agent **the right context, at the
right time, grounded in your real systems**:

1. **Memory.** Your agent captures and recalls durable facts across sessions. Recall is
   graph-aware hybrid retrieval (vector + keyword + graph), so it returns not just a
   matching note but the subgraph around it.
2. **Live data.** Those memories sit next to the logs, traces, and code they're about —
   queryable in KQL or SQL — so the agent can check what *actually* happened, not just
   what it was told.
3. **The graph.** Memories and live resources are nodes in one property graph. A memory
   about `payments-svc` links to the real service, its repo, and the traces it emits —
   so recall comes back wired to the systems it concerns.

```
  capture ──▶  MEMORY ──┐
                        ├──▶  the GRAPH that links them  ──▶  recalled into every prompt
  ingest  ──▶  LIVE DATA┘
```

Each layer is the same engine doing more. You meet **memory** first (it's the fastest,
most useful thing), then **live data**, then watch the **graph** tie them together.

## How your agent connects

MCP is the cross-agent standard, but it's *pull-only* — the agent only sees what it
explicitly asks for. kyma meets your agent where it is, with three paths that share one
engine:

| Path | Best for | What's special |
|---|---|---|
| **Claude Code plugin** | Claude Code | **Automatic** — hooks capture each turn and inject the most relevant memories into *every* prompt, no tool call; plus slash commands. |
| **CLI + skill** | Cursor, Aider, Continue, any shell-tool agent | The agent shells out to `kyma recall` / `kyma query`; a skill teaches it *when*. |
| **MCP** | Any MCP client | The full tool surface (memory · graph · live data · curation) over stdio or HTTP. |

See [Connect your agent](/agent/connect) for the one-command setup.

## The engine underneath

"Live data" is a real columnar engine, not a key-value store — which is what lets an
agent ask twenty exploratory questions per prompt:

- **Columnar Arrow on object storage you own**, with per-extent stats + token indices, so
  a [three-level pruning cascade](/concepts/the-pruning-cascade) skips 99 %+ of data.
- **KQL, SQL, or PromQL** over Arrow Flight gRPC — exact rows, streamed zero-copy.
- **Five invariants** keep it swappable and scalable: object storage is the only source of
  truth, compute is stateless, the catalog is externalized, format and parser are
  pluggable. See [The five invariants](/concepts/the-five-invariants).

## Getting data in — continuously

Two directions, same group-commit write path and the same extents underneath:

- **Push** — your stack sends to kyma: OTLP, REST/NDJSON, Kafka, or file-drop. See [Ingest](/ingest/).
- **Pull** — **data sources** fetch from sources on a **schedule**: GitHub/GitLab/Bitbucket,
  Prometheus, Postgres/MySQL/MongoDB, Notion/Slack/Jira/Confluence/Gmail/Drive. A data source
  runs periodically and lands rows continuously, so the picture stays current without you
  re-running anything. See [Data sources](/data-sources/).

## Keeping memory sharp — consolidation & dreaming

Memory doesn't just accumulate; kyma maintains it:

- **Deterministic consolidation** (always on) — cheap firehose rollups that keep the
  memory tables tidy. No LLM, runs regardless.
- **[Dreaming](/agent/dreaming)** (opt-in) — a scheduled, autonomous agent that reviews
  recent memories, fills gaps with read-only data source access, wires memories to the
  resources they're about, merges duplicates, and supersedes contradictions (bi-temporal —
  never a hard delete). It's the intelligence layer on top of consolidation, bounded by
  call/byte/wall-clock budgets.

## Where it runs — two modes

The same context engine runs two ways, and grows from one to the other without a rewrite:

| | **Local mode** (`kyma`) | **Control plane** (kyma server) |
|---|---|---|
| Infra | embedded SQLite + local files — zero infra | Postgres + object store (S3/MinIO) |
| Who | one developer, offline, instant | a team, always-on, shared |
| Extras | on-demand ingest + query | scheduled data sources, dreaming, the full web app |

A read-scale **cluster** is the same server with multiple stateless nodes over shared
object storage. See [Local & cluster mode](/concepts/local-and-cluster-mode).

## Staying coherent — sync

Memory created locally and memory on the control plane stay consistent through
**[sync](/concepts/sync)** — an incremental, bidirectional push/pull. Work offline on a
plane, sync when you land; your team's shared memory and your laptop's memory converge.

## Read next

- Start hands-on: [Give your agent memory](/quickstart/give-your-agent-memory).
- The runtime topology: [Local & cluster mode](/concepts/local-and-cluster-mode).
- Keeping machines coherent: [Sync](/concepts/sync).
- The recall internals: [Agentic Memory](/agent/memory).
