---
title: Agent endpoint
description: POST /v1/agent/ask is pensieve's natural-language surface — JSON in, Server-Sent Events out. This is the call reference; the conceptual model lives in /concepts/the-agent-loop.
---

# Agent endpoint

`POST /v1/agent/ask` is pensieve's natural-language surface. A question goes
in as JSON; an SSE stream of events comes back. The conceptual model —
how the agent picks tables, why SSE over WebSockets, what the schema RAG
layer does — is on [The agent loop](/concepts/the-agent-loop). This page
is the precise call reference.

## Request

```
POST /v1/agent/ask
Authorization: Bearer <token>          (when PENSIEVE_AUTH_TOKENS is set)
Content-Type:  application/json
```

JSON body:

| Field              | Type      | Required | Default         | Notes                                                |
| ------------------ | --------- | -------- | --------------- | ---------------------------------------------------- |
| `question`         | `string`  | yes      | —               | Trimmed; must be non-empty.                          |
| `database`         | `string`  | no       | _server-side_   | Which database the agent's tools see by default.     |
| `include_thinking` | `bool`    | no       | `false`         | Stream `thinking_delta` events alongside the answer. |

The model id and embedding backend are configured server-side at startup
(see [Dynamic and vectors](/concepts/dynamic-and-vectors) for the
embedding backends). They aren't selectable per request.

## Response

`Content-Type: text/event-stream`. The connection stays open for the
duration of the run (capped at 60 s wall-clock; the run is aborted with a
`run_error{code:"timeout"}` if it exceeds that).

Each frame is one SSE event with a named `event:` line and a `data:` line
carrying a JSON object.

### Event names and payloads

| Event             | Payload shape                                                   | When                                       |
| ----------------- | --------------------------------------------------------------- | ------------------------------------------ |
| `run_started`     | `{ run_id, model, question }`                                   | First frame, always.                       |
| `thinking_delta`  | `{ text }`                                                      | Only if `include_thinking: true`.          |
| `answer_delta`    | `{ text }`                                                      | One per generation chunk.                  |
| `tool_call`       | `{ tool, args, call_index }`                                    | One per tool invocation.                   |
| `tool_result`     | `{ tool, result }`                                              | After each `tool_call` resolves.           |
| `answer_final`    | `{ text, kql_used, sql_used }`                                  | Once, on success only.                     |
| `run_error`       | `{ code, message }`                                             | On timeout, tool-loop, or runner failure.  |
| `run_finished`    | `{ run_id, usage, elapsed_ms, tool_calls }`                     | Last frame, always.                        |

Every run ends with `run_finished` — even on error. Treat that as the
stream terminator.

`answer_final.sql_used` carries the *last* `run_sql` invocation's query
string (if any). `answer_final.kql_used` is reserved but currently
always `null`; don't depend on it being populated.

`tool_call.args` is the full JSON arguments object the model passed to
the tool. The eight built-in tools (`list_databases`, `describe_table`,
`run_sql`, `run_kql`, `sample_rows`, `explore_schema`,
`find_references_to`, `graph_traverse`) are documented in
[The agent loop](/concepts/the-agent-loop).

`run_error.code` is one of:

| Code            | Meaning                                                              |
| --------------- | -------------------------------------------------------------------- |
| `timeout`       | The 60-second wall-clock budget elapsed.                             |
| `tool_loop`     | More than 12 tool calls in one run — likely an infinite loop.        |
| `runner_error`  | The underlying ADK runner returned an error (LLM, tool exception).   |
| `init_error`    | Failed to construct the runner (config / startup issue).             |
| `internal`      | Unexpected internal error.                                           |

## Curl example

```bash
curl -N -X POST http://localhost:8080/v1/agent/ask \
  -H "Authorization: Bearer reader-tok" \
  -H "Content-Type: application/json" \
  -d '{
    "question": "which service errored most in the last hour?",
    "database": "default",
    "include_thinking": false
  }'
```

`-N` disables curl's output buffering so SSE frames render as they
arrive. Without it the whole stream lands at once at end-of-run.

## Python example

```python
import httpx
import json

with httpx.stream(
    "POST",
    "http://localhost:8080/v1/agent/ask",
    headers={
        "Authorization": "Bearer reader-tok",
        "Content-Type":  "application/json",
    },
    json={
        "question":         "top 5 errors in the last hour",
        "database":         "default",
        "include_thinking": True,
    },
    timeout=None,
) as resp:
    event = None
    for line in resp.iter_lines():
        if line.startswith("event:"):
            event = line.removeprefix("event:").strip()
        elif line.startswith("data:"):
            payload = json.loads(line.removeprefix("data:").strip())
            print(event, payload)
        elif line == "":
            event = None  # frame boundary
```

Production code wants a real SSE parser (`httpx-sse`, `aiohttp-sse-client`,
etc.); the snippet above is the smallest readable thing that works.

## Replay

Every run is persisted to the `agent_runs` catalog table with the
complete event trace. To pull a run back out:

```
GET /v1/agent/runs/:run_id
```

Returns a JSON document:

```json
{
  "run_id":      "01HZ...",
  "question":    "top 5 errors in the last hour",
  "model_id":    "gemma4:latest",
  "status":      "success",
  "started_at":  "2026-05-02T10:14:22Z",
  "finished_at": "2026-05-02T10:14:24Z",
  "usage":       { "tool_calls": 2, "elapsed_ms": 1840 },
  "trace":       [ { "event": "run_started", "data": { ... } }, ... ]
}
```

`status` is `success`, `error`, or `budget_exceeded`. The `trace` array
is the full event log in order — the same shape you saw on the SSE
stream. Useful for debugging a wrong answer without replaying the run
through the LLM.

## Failure modes

- **Empty question.** `400 bad_request` with body `{"error":"question
  must be non-empty"}`. No SSE stream.
- **Invalid run id on lookup.** `400 bad_request` from
  `GET /v1/agent/runs/...`.
- **Run not found.** `404` from `GET /v1/agent/runs/...`.
- **Runner init failure.** A two-frame SSE stream — `run_error{code:
  "init_error"}` then `run_finished`. The HTTP status is still `200`,
  because the run started before the failure surfaced.
- **Tool exception (e.g. `run_sql` against a missing table).** Surfaces
  as a `tool_result` event with the error embedded in the result
  payload. The agent typically reads it, narrates the failure to the
  user via `answer_delta`, and finishes normally.
- **LLM upstream failure.** `run_error{code:"runner_error"}` with the
  underlying message in `message`.
- **Run timeout / tool loop.** `run_error{code:"timeout"}` or `code:
  "tool_loop"`, then `run_finished` with `status: "budget_exceeded"`
  on the persisted row.

## Where to go next

- The conceptual model: [The agent loop](/concepts/the-agent-loop).
- Embedding backends and vector columns: [Dynamic and
  vectors](/concepts/dynamic-and-vectors).
- The query path the agent's tools call into: [SQL](/query/sql),
  [Arrow Flight](/query/arrow-flight).
