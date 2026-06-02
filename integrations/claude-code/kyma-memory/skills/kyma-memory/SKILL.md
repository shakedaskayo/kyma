---
name: kyma-memory
description: Use to recall durable context (prior decisions, preferences, conventions, learnings) or to persist new durable memories for this user/project. Recall BEFORE answering questions that may depend on past context; save AFTER the user states something worth remembering long-term. Backed by the Kyma memory layer via the bundled `kyma` MCP server.
---

# Kyma Memory

This project is wired to **Kyma**, the user's unified realtime memory layer. The
`kyma-memory` plugin bundles an MCP server named `kyma` that exposes memory + query
tools, and hooks that continuously capture this session into Kyma.

## When to recall

Call `recall_memory` (MCP server `kyma`) before answering anything that could depend
on prior context: the user's preferences, past decisions, project conventions,
architecture choices, or "how we did X last time". Pass the user's request as `query`,
set `realms` to the current project (the working directory's basename) plus `global`,
and `limit` ~8. Recalled memories also arrive automatically as injected context on each
prompt — use `recall_memory` when you need more, a different query, or a specific realm.

## When to save

Call `save_memory` when the user states something durable and reusable:
- **decision** — an architecture/approach choice and its rationale
- **preference** — how the user likes things done (style, tools, workflow)
- **fact** — a non-obvious, load-bearing fact about the system or domain
- **learning** — something discovered this session worth keeping

Set `realm` to the project (or `global` for cross-project truths) and `importance`
0.3–0.9. Keep each memory self-contained (one idea, understandable without the chat).
Recall first to avoid duplicates; prefer updating/merging over re-saving.

## Querying the user's data

The same `kyma` MCP server exposes `explore_schema`, `describe_table`, `run_kql`,
`run_sql`, `sample_rows`, `find_references_to`, and `graph_traverse`. Use them to answer
questions about the user's logs, traces, tables, and code/graph — including the
`claude_code_events` table, which is this plugin's realtime capture of the conversation.

## Enriching the graph

Call `ingest_entity` (MCP server `kyma`) to mint a **virtual resource/entity** — a service,
repo, table, person, file, config, or concept — and wire it to existing graph nodes
(`target_namespace` + node id, e.g. a connector-ingested `repo:owner/name`) and to memories
(`memory:<uuid>`). Discover real node ids first with `find_references_to` / `graph_traverse` /
`recall_memory`; it's idempotent per `(realm, kind, name)`. The `/kyma-ingest` command guides
this and also triggers connector pulls.

## Don't

- Don't invent memories or claim to remember something `recall_memory` didn't return.
- Don't save secrets, tokens, or transient scratch state.
- Memory writes go to the user's own Kyma server; treat them as durable and shared.
