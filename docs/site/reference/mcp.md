---
title: MCP
description: Kyma's Model Context Protocol server — transports, tool catalog (23 tools), auth, and quick-start wiring for coding agents.
---

# MCP

Kyma ships a built-in MCP server that exposes its full query, memory, and
graph surface as JSON-RPC 2.0 tools. Any MCP-capable client — Claude Code,
Cursor, Windsurf, or any custom agent — can drive kyma through this interface
without writing HTTP calls by hand.

Related pages: [Coding agents](/agent/coding-agents) · [Connect a coding agent](/agent/connect) · [Agent memory](/agent/memory) · [Graph queries](/query/graph)

---

## Transports

### stdio — `kyma mcp`

The embedded engine starts and serves JSON-RPC 2.0 over stdin/stdout. Zero
infrastructure: a single binary, no network port, no credentials to manage.

```bash
kyma mcp   # run directly to test; agents invoke this as the MCP server process
```

`kyma setup <agent>` writes (or merges into) the agent's MCP config file so
the agent spawns `kyma mcp` automatically. See [`kyma setup`](/reference/cli#kyma-setup-agent-list-print).

Example config entry written by `kyma setup claude-code` (`.mcp.json`):

```json
{
  "mcpServers": {
    "kyma": {
      "type": "stdio",
      "command": "/path/to/kyma",
      "args": ["mcp"]
    }
  }
}
```

**Supported agents:** `claude-code` (`.mcp.json`, project scope), `cursor`
(`.cursor/mcp.json`, project scope), `windsurf`
(`~/.codeium/windsurf/mcp_config.json`, global). Any unknown agent key prints
a generic snippet to paste manually.

### HTTP — `POST /mcp/v1`

When agents talk to a shared kyma server instead of a local binary, they can
use the Streamable HTTP transport:

| Method | Path      | Content-Type       | Effect                              |
| ------ | --------- | ------------------ | ----------------------------------- |
| POST   | `/mcp/v1` | `application/json` | JSON-RPC 2.0 channel (request/response). |
| GET    | `/mcp/v1` | —                  | SSE upgrade for streaming sessions. |

Use stdio for local development and single-user setups. Use HTTP when a team
shares a single kyma deployment, or when an agent runs in a remote environment
that cannot exec a binary.

::: tip Note on connector tools
`list_connectors` and `connector_read` are **server mode only** — they need
the credential store. They are not available over stdio / `kyma mcp`.
:::

---

## Auth and scoping

The HTTP MCP surface requires a bearer token with at least `Role::Read` — the
same token class used by `/v1/*` routes. See [HTTP API](/reference/api#mcp-over-http)
for the route details.

Scoped tokens (per-database tokens) are **blocked** at the middleware level
for the `/mcp/v1` routes because MCP tools address databases as arguments, not
as URL segments. Use a full-scope `Read` token.

The stdio transport (`kyma mcp`) runs under the local user's identity and
requires no token.

---

## Tools

**23 tools total: 21 available over stdio, plus 2 connector tools in server mode only.**

### Query

| Tool | Purpose |
| ---- | ------- |
| `list_databases` | List every database in the kyma cluster. Call first to discover what databases exist. |
| `describe_table` | Describe the columns of a table: names, Arrow data types, nullability. Call this before writing a SQL query against an unfamiliar table. |
| `run_sql` | Execute a read-only SQL query via DataFusion. Use `cosine_distance` / `l2_distance` UDFs for vector similarity. Returns up to `max_rows` (default 200) rows as JSON. |
| `run_kql` | Execute a KQL query against kyma — the primary query tool. KQL is pipe-syntax (`requests \| where status >= 500 \| summarize n=count() by url \| top 10 by n`). |
| `sample_rows` | Fetch N representative rows from a table. Use when `describe_table`'s column types aren't enough to understand the data shape (JSON/dynamic columns, text formats). |
| `explore_schema` | Return the full schema of a database in one call: every table, every column, types, and sample values. Use first for questions that span multiple tables. |
| `find_references_to` | Find every (database, table, column) where a given value appears in the catalog's distinct-value index — the relationship-traversal primitive. |
| `graph_traverse` | Traverse a graph stored as edges in a kyma table using the KQL `graph-traverse` operator. Returns reachable nodes as (node, depth) pairs. |

### Memory

| Tool | Purpose |
| ---- | ------- |
| `memory_search` | Find anything fast across the agent's memory: hybrid semantic + keyword search, graph-expanded over connected memories, catalog resources, and distributed traces. Use this first when a question may depend on prior context. |
| `recall_memory` | Recall the most relevant stored memories for a query using graph-aware hybrid search (semantic + keyword), expanded over connected memories and resources. |
| `save_memory` | Persist a durable memory (fact, decision, preference, learning, or summary) so it can be recalled in later sessions. Pass a `topic_key` to upsert in place. |
| `save_memories` | Persist many durable memories in one call — batches one embedding round-trip and one storage commit. Faster than repeated `save_memory` when saving two or more items. |
| `flush_memory` | Wait until queued memory writes are fully committed. Call only when you need an explicit durability checkpoint (e.g. right before the session ends). |
| `list_memories` | List stored memories with optional filters (type, status, realm, tags), newest first. Use for browsing/auditing rather than semantic search. |
| `update_memory_status` | Change a memory's lifecycle status (active/background/archived). Archived memories are hidden from recall. |
| `update_memory_importance` | Set a memory's importance (0.0–1.0). Higher importance surfaces a memory earlier in recall. |
| `memory_compare` | Fetch two memories side by side to judge their relationship, then record it with `memory_judge`. Returns both full memory rows. |
| `memory_judge` | Record a conflict/relationship verdict between two memories as a graph edge (`supersedes`, `merged`, `conflicts`, `related`, `compatible`). |
| `memory_session_summary` | Save a structured end-of-session summary (goal, instructions, discoveries, accomplished, next steps, files) as a durable `summary` memory. |

### Graph / entities

| Tool | Purpose |
| ---- | ------- |
| `ingest_entity` | Create (or update) a virtual resource/entity on the knowledge graph and wire it to existing graph nodes and memories. Idempotent on (realm, kind, name). |
| `link_memory_to_entity` | Link a memory to an existing graph entity (repo, service, table, user, …) by node id, creating a REFERENCES edge. |

### Connectors (server mode only)

These two tools are added when kyma runs as a server and has access to the
credential store. They are **not available** over stdio (`kyma mcp`).

| Tool | Purpose |
| ---- | ------- |
| `list_connectors` | List the configured data connectors (id, name, kind, enabled, target database). Use this to discover which external sources you can read from with `connector_read`. |
| `connector_read` | Read-only access to a configured connector's source using its stored credential. Supports github operations (`get_repo`, `get_readme`, `get_file`, `list_issues`, `get_issue`, `list_pulls`) and postgres (`query`). |

---

## Cross-links

- [Coding agents — setup and configuration](/agent/coding-agents)
- [Connect a coding agent](/agent/connect)
- [Agent memory — memory model and knobs](/agent/memory)
- [Graph queries — KQL graph operators](/query/graph)
- [HTTP API — `/mcp/v1` route details](/reference/api#mcp-over-http)
- [CLI reference — `kyma mcp` and `kyma setup`](/reference/cli)
