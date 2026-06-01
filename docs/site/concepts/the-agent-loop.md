---
title: The agent loop
description: How `/v1/agent/ask` turns a natural-language question into KQL, runs it, and streams the answer over Server-Sent Events. Schema RAG via pgvector keeps the agent's mental model accurate as the schema evolves.
---

# The agent loop

`/v1/agent/ask` is kyma's natural-language surface. A user (or another
agent) sends a question; kyma figures out which table to look at, runs
the query, and streams the answer back as Server-Sent Events.

The shape is intentionally narrow. The agent doesn't replace KQL or SQL
— it turns "show me the top errors" into the same KQL you'd have written
yourself, runs it, and tells you what it found.

## Request

```bash
curl -N -X POST http://localhost:8080/v1/agent/ask \
  -H 'Content-Type: application/json' \
  -d '{
    "question": "which service errored most in the last hour?",
    "database": "default"
  }'
```

`-N` keeps curl from buffering — important for SSE.

## Response

Server-Sent Events. Each event is a JSON envelope:

```
event: run_started
data: {"run_id": "01HZ...", "model": "gemma4:latest"}

event: thinking_delta
data: {"text": "Looking at the schema. otel_logs has severity_text and..."}

event: tool_call
data: {"tool": "run_sql", "args": {"query": "SELECT service_name, COUNT(*) ..."}, "call_index": 0}

event: tool_result
data: {"rows": [{"service_name": "payments-svc", "n": 412}, ...]}

event: answer_delta
data: {"text": "Over the last hour, payments-svc had the most errors (412), "}

event: answer_delta
data: {"text": "followed by auth-svc (198) and search-svc (89)."}

event: answer_final
data: {"text": "Over the last hour, payments-svc had the most errors..."}

event: run_finished
data: {"run_id": "01HZ...", "elapsed_ms": 1840, "usage": {"input_tokens": 940, "output_tokens": 320}, "tool_calls": 2}
```

The `answer_delta` events let you render answers token-by-token while
they generate. `tool_call` and `tool_result` give you full transparency
into which queries were run — useful for debugging a wrong answer.

## What the agent has access to

Eight built-in tools, plus a schema RAG layer:

- **`list_databases`** returns every database the catalog knows about
  (kyma-native plus any registered external sources).
- **`describe_table`** returns the schema of a specific table —
  columns, types, recent sample rows, present `dynamic` paths.
- **`run_sql`** executes a SQL query and returns the result rows.
- **`run_kql`** executes a KQL pipeline against the same engine — same
  Arrow result, just a different surface.
- **`sample_rows`** pulls a small random sample for exploratory work.
- **`explore_schema`** returns a one-shot "context graph" view of a
  database: all tables, their columns, and the foreign-key-shaped edges
  between them. Lets the agent reason about an unfamiliar schema in
  one tool call.
- **`find_references_to`** traverses the catalog for tables that
  reference a given entity — e.g., "every table that has a `user_id`
  column."
- **`graph_traverse`** walks the kyma graph layer. Wraps the KQL
  `graph-traverse` operator for multi-hop entity-relationship queries.

Plus the **memory tools** — `memory_search` (graph-aware hybrid recall),
`save_memory`, `recall_memory`, `list_memories`, `link_memory_to_entity` — that
give the agent a persistent memory across sessions. See
[Agentic Memory](/agent/memory) for the full picture.

Schema RAG: every table in the catalog is embedded into the
`schema_embeddings` pgvector table at create time. When the agent
needs to find a table, it does a vector search over the embeddings
rather than reading the entire catalog. This keeps prompt size bounded
even on a catalog with thousands of tables.

## Why SSE and not WebSockets

SSE is one-way (server → client) over HTTP/1.1. That fits the request
shape perfectly: the client asks one question, the server streams
many events. No reconnection logic, no framing protocol, works through
any HTTP proxy that handles streaming responses.

If you need bidirectional tool use — agent asks the client for
permission to do something, etc. — that lands as a separate endpoint
in a later milestone.

## Run history

Every run is persisted to the `agent_runs` catalog table:

```sql
SELECT run_id, question, model_id, status, started_at, finished_at,
       usage_json, trace_json
  FROM agent_runs
 ORDER BY started_at DESC
 LIMIT 10
```

Replay a specific run with the full event trace via:

```bash
curl http://localhost:8080/v1/agent/runs/01HZABCDE...
```

## Models

The agent's LLM is **Ollama**, configured at startup via two env vars:

- `KYMA_AGENT_OLLAMA_HOST` — Ollama daemon URL. Defaults to the host's
  local Ollama installation.
- `KYMA_AGENT_MODEL` — model name. Defaults to `gemma4:latest`.

The choice of Ollama is deliberate: the agent runs against your
infrastructure, with no telemetry leaving the cluster, and you control
which weights it sees. Other LLM backends (Anthropic Claude, OpenAI,
Gemini) are tracked as a follow-up; the underlying ADK abstraction
already supports them.

Separately, the **embedding backend** for schema RAG configures
independently — see [Dynamic and vectors](/concepts/dynamic-and-vectors).
Out of the box: `fastembed` (default, CPU), `ollama`, OpenAI-compatible,
or Gemini. Embeddings power the schema search; the LLM consumes the
result.

## Limits

The agent is read-only by design. The exposed tools never write — no
`ingest_rows`, no `create_table`. Write tools are tracked under the
roadmap section of the spec; the constraint is intentional, since
giving an LLM the ability to mutate production data without an explicit
approval step is a bad default.

## Where to go next

- Persistent memory for agents: [Agentic Memory](/agent/memory).
- Vectors and embedding backends: [Dynamic and vectors](/concepts/dynamic-and-vectors).
- Calling the endpoint from your code: [Query](/query/).
- The schema the agent reasons over: [Schema model](/concepts/schema-model).
