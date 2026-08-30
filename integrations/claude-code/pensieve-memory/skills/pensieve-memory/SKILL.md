---
name: kyma-memory
description: Recall durable context (prior decisions, preferences, conventions, learnings) and query the user's live data + knowledge graph; persist new durable memories. Recall BEFORE answering anything that may depend on past context or the user's systems; save AFTER the user states something worth keeping long-term. Backed by the Kyma context engine via the bundled `kyma` MCP server (memory + data + graph tools).
---

# Kyma — the user's context engine

This project is wired to **Kyma**: the user's durable memory **+** live data **+** the
knowledge graph that links them. The `kyma-memory` plugin bundles an MCP server named
`kyma` (memory + data + graph tools) and **hooks** that automatically capture each session
and inject the most relevant memories into every prompt — so recall is already happening;
use the tools below when you need *more*, or to write.

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

Two more tools sit on the unified `/v1/search` substrate:
- `search` — one hybrid lexical+vector query across all data sources in scope; returns ranked hits with `db.table` provenance + the row. Use for a fast broad sweep before narrowing with `run_kql`/`run_sql`.
- `graph_search` — find graph nodes by text/label across one named graph or every graph; returns nodes with `<db>/<graph>` provenance and ids you can feed to `graph_traverse`.

## Closing the loop

After you act on a memory `recall_memory`/`memory_search` surfaced, call `reinforce_memory`
once you know the outcome: `outcome="helpful"` if it was correct/useful, `outcome="not_helpful"`
if it was wrong, outdated, or irrelevant. This is telemetry about the memory, not a content
edit — it never requires approval, and it's how recall gets better at surfacing what actually
helps over time. Skip it when you genuinely don't know whether a memory mattered; don't guess.

## Enriching the graph

Call `ingest_entity` (MCP server `kyma`) to mint a **virtual resource/entity** — a service,
repo, table, person, file, config, or concept — and wire it to existing graph nodes
(`target_namespace` + node id, e.g. a data-source-ingested `repo:owner/name`) and to memories
(`memory:<uuid>`). Discover real node ids first with `find_references_to` / `graph_traverse` /
`recall_memory`; it's idempotent per `(realm, kind, name)`. The `/kyma-ingest` command guides
this and also triggers data source pulls.

## Don't

- Don't invent memories or claim to remember something `recall_memory` didn't return.
- Don't save secrets, tokens, or transient scratch state.
- Memory writes go to the user's own Kyma server; treat them as durable and shared.
