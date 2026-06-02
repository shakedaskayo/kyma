---
title: Connect your agent
description: Wire any coding agent to kyma's context engine — durable memory + live data + the graph that links them. Connect via a Claude Code plugin (automatic capture + recall), a CLI any agent shells out to, or MCP (stdio + HTTP).
---

# Connect your agent

kyma is a **context engine**: one place your coding agent recalls durable, graph-aware
**memory**, queries **live data** (logs, traces, code, connectors) in KQL/SQL, and walks
the **graph** that links them. Getting an agent connected takes one command.

MCP is the cross-agent standard — but it's **pull-only** (the agent only sees what it
explicitly asks for). kyma meets your agent where it is, with three first-class paths that
share the same engine:

| Path | Best for | What's special |
| --- | --- | --- |
| **Claude Code plugin** | Claude Code | **Automatic** — captures each turn and injects the most relevant memories into *every* prompt; plus slash commands. |
| **CLI + skill** | Cursor, Aider, Continue, any shell-tool agent | The agent shells out to `kyma query` / `kyma recall`; a skill teaches it *when*. |
| **MCP** | Any MCP client (Claude Code, Cursor, Windsurf, …) | The full 19-tool surface over stdio or HTTP. |

Pick one (they compose — the plugin uses both hooks *and* MCP).

## Fastest path — MCP, zero infra

The local single binary needs no Postgres and no Docker (embedded SQLite + local files):

```bash
cargo install --path crates/kyma-cli
kyma setup claude-code      # or: cursor · windsurf
```

`setup` writes the agent's MCP config to launch `kyma mcp` over stdio; data lives
under `~/.kyma`. Restart the agent and it has the full toolset. `kyma setup list`
shows the supported agents; `--print` previews the config without writing it; any other
agent gets a paste-able stdio snippet.

## Automatic capture + recall — the Claude Code plugin

The plugin is the "it just remembers" path: **hooks** capture each session into the
firehose and inject recalled context into every prompt with no tool call, plus slash
commands. It shells out to the `kyma` CLI and points at a kyma endpoint (a server, or a
local `kyma serve`):

```bash
cargo install --path crates/kyma-cli   # the `kyma` CLI — the plugin's hooks use it
kyma connect <kyma-url>                # a kyma server, or a local `kyma serve`
kyma install-plugin                    # installs hooks + slash commands into ~/.claude
```

You get `/kyma-recall`, `/kyma-remember`, `/kyma-ask`, `/kyma-ingest`, and `/kyma-status`
on top of the automatic capture/recall. See **[Claude Code plugin](./claude-code-plugin)**
for the hook wiring and configuration.

## Any agent — the CLI + a skill

Any coding tool that can shell out becomes context-aware:

```bash
kyma connect <kyma-url> --token <bearer>
kyma install-skill --also-link-claude   # writes a SKILL.md that teaches the agent when to use kyma
kyma recall   "how do we run database migrations?"          # recall before answering
kyma remember "Prefer KQL over SQL in examples" --type preference   # save durable memory
kyma query    "what error logs from prod-api in the last 15 minutes?"   # ask live data
kyma entity   "payments service" --kind service --link "repo:owner/name|github|LIVES_IN"
```

The installed skill teaches the agent the full loop — **recall** before answering,
**remember** what's durable, **query** live data, **enrich** the graph.

See **[Connect a coding agent (CLI)](./connect-from-cli)** for the full CLI surface.

## MCP details

The MCP server exposes the **same toolset** over two transports:

- **stdio** — `kyma mcp` (what `setup` wires). Zero infra, local.
- **HTTP** — `POST /mcp/v1` on any running kyma server (or `kyma serve`), bearer-auth.

Either way an MCP client gets the full context-engine surface — memory
(`memory_search` · `recall_memory` · `save_memory` · `list_memories`), graph
(`ingest_entity` · `link_memory_to_entity` · `graph_traverse` · `find_references_to`),
live data (`run_kql` · `run_sql` · `explore_schema` · `describe_table` · `sample_rows` ·
`list_databases`), and curation (`update_memory_status/importance` · `memory_compare` ·
`memory_judge` · `memory_session_summary`). Call `memory_search` first; follow `linked`
resources with `graph_traverse`.

## Local ↔ server: two tiers, one memory

The same context engine runs two ways, and memory stays coherent across them:

- **`kyma` (local mode)** — one binary per developer: stdio MCP (`kyma mcp`), an optional
  local web UI (`kyma serve`), and on-demand ingest. Zero infra.
- **kyma server** — the team control plane: Postgres + object store, scheduled
  **connectors**, background consolidation, the full web app.

Keep a machine in sync with the control plane (push local changes + pull remote ones,
incrementally):

```bash
KYMA_CLOUD_URL=https://kyma.your-co.dev KYMA_CLOUD_TOKEN=… kyma sync
```

## Next

- **[Agentic Memory](./memory)** — how recall, the graph, and bi-temporal validity work.
- **[Claude Code plugin](./claude-code-plugin)** — hooks, slash commands, configuration.
- **[Engines](./engines)** — pick the LLM the kyma agent itself uses (Anthropic / OpenAI /
  Ollama / Claude Code OAuth).
