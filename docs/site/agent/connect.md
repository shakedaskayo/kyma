---
title: Connect your agent
description: Wire any coding agent to pensieve's context engine — durable memory + live data + the graph that links them. Connect via a Claude Code plugin (automatic capture + recall), a CLI any agent shells out to, or MCP (stdio + HTTP).
---

# Connect your agent

pensieve is a **context engine**: one place your coding agent recalls durable, graph-aware
**memory**, queries **live data** (logs, traces, code, data sources) in KQL/SQL, and walks
the **graph** that links them. Getting an agent connected takes one command.

MCP is the cross-agent standard — but it's **pull-only** (the agent only sees what it
explicitly asks for). pensieve meets your agent where it is, with three first-class paths that
share the same engine:

| Path | Best for | What's special |
| --- | --- | --- |
| **Claude Code plugin** | Claude Code | **Automatic** — captures each turn and injects the most relevant memories into *every* prompt; plus slash commands. |
| **CLI + skill** | Cursor, Aider, Continue, any shell-tool agent | The agent shells out to `pensieve query` / `pensieve recall`; a skill teaches it *when*. |
| **MCP** | Any MCP client (Claude Code, Cursor, Windsurf, …) | The full tool surface over stdio or HTTP. |

Pick one (they compose — the plugin uses both hooks *and* MCP).

## Fastest path — MCP, zero infra

The local single binary needs no Postgres and no Docker (embedded SQLite + local files):

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash
pensieve setup claude-code      # or: cursor · windsurf
```

`setup` writes the agent's MCP config to launch `pensieve mcp` over stdio; data lives
under `~/.pensieve`. Restart the agent and it has the full toolset. `pensieve setup list`
shows the supported agents; `--print` previews the config without writing it; any other
agent gets a paste-able stdio snippet.

## Automatic capture + recall — the Claude Code plugin

The plugin is the "it just remembers" path: **hooks** capture each session into the
firehose and inject recalled context into every prompt with no tool call, plus slash
commands. It shells out to the `pensieve` CLI and points at a pensieve endpoint (a server, or a
local `pensieve serve`):

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash   # the `pensieve` CLI — the plugin's hooks use it
pensieve connect <pensieve-url>                # a pensieve server, or a local `pensieve serve`
pensieve install-plugin                    # installs hooks + slash commands into ~/.claude
```

You get `/pensieve-recall`, `/pensieve-remember`, `/pensieve-ask`, `/pensieve-ingest`, and `/pensieve-status`
on top of the automatic capture/recall. See **[Claude Code plugin](./claude-code-plugin)**
for the hook wiring and configuration.

## Any agent — the CLI + a skill

Any coding tool that can shell out becomes context-aware:

```bash
pensieve connect <pensieve-url> --token <bearer>
pensieve install-skill --also-link-claude   # writes a SKILL.md that teaches the agent when to use pensieve
pensieve recall   "how do we run database migrations?"          # recall before answering
pensieve remember "Prefer KQL over SQL in examples" --type preference   # save durable memory
pensieve query    "what error logs from prod-api in the last 15 minutes?"   # ask live data
pensieve entity   "payments service" --kind service --link "repo:owner/name|github|LIVES_IN"
```

The installed skill teaches the agent the full loop — **recall** before answering,
**remember** what's durable, **query** live data, **enrich** the graph.

See **[Connect a coding agent (CLI)](./connect-from-cli)** for the full CLI surface.

## MCP details

The MCP server exposes the **same toolset** over two transports:

- **stdio** — `pensieve mcp` (what `setup` wires). Zero infra, local.
- **HTTP** — `POST /mcp/v1` on any running pensieve server (or `pensieve serve`), bearer-auth.

Either way an MCP client gets the full context-engine surface — memory
(`memory_search` · `recall_memory` · `save_memory` · `list_memories`), graph
(`ingest_entity` · `link_memory_to_entity` · `graph_traverse` · `find_references_to`),
live data (`run_kql` · `run_sql` · `explore_schema` · `describe_table` · `sample_rows` ·
`list_databases`), and curation (`update_memory_status/importance` · `memory_compare` ·
`memory_judge` · `memory_session_summary`). Call `memory_search` first; follow `linked`
resources with `graph_traverse`.

## Local ↔ server: two tiers, one memory

The same context engine runs two ways, and memory stays coherent across them:

- **`pensieve` (local mode)** — one binary per developer: stdio MCP (`pensieve mcp`), an optional
  local web UI (`pensieve serve`), and on-demand ingest. Zero infra.
- **pensieve server** — the team control plane: Postgres + object store, scheduled
  **data sources**, background consolidation, the full web app.

Keep a machine in sync with the control plane (push local changes + pull remote ones,
incrementally):

```bash
PENSIEVE_CLOUD_URL=https://pensieve.your-co.dev PENSIEVE_CLOUD_TOKEN=… pensieve sync
```

## Next

- **[Agentic Memory](./memory)** — how recall, the graph, and bi-temporal validity work.
- **[Claude Code plugin](./claude-code-plugin)** — hooks, slash commands, configuration.
- **[Engines](./engines)** — pick the LLM the pensieve agent itself uses (Anthropic / OpenAI /
  Ollama / Claude Code OAuth).
