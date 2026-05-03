---
title: First real run
description: A walkthrough that goes one step beyond the five-minute start — batched REST ingest, a real KQL summarize, and the agent SSE endpoint.
---

# First real run

You have the dev stack up from [Five-minute start](/quickstart/five-minute-start).
This page takes you one step further: a batched ingest, a KQL query that
actually aggregates, and a quick look at the agent endpoint.

## Ingest a small batch

NDJSON means one JSON object per line. The body below seeds eight rows into
`default.hello` with mixed services and severities so the next step has
something to summarize.

```bash
curl -sS -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: default" \
  -H "X-Table: hello" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"_timestamp":"2026-05-02T10:00:00Z","service_name":"payments","severity_text":"INFO","message":"charge ok"}
{"_timestamp":"2026-05-02T10:00:05Z","service_name":"payments","severity_text":"ERROR","message":"card declined"}
{"_timestamp":"2026-05-02T10:00:09Z","service_name":"payments","severity_text":"ERROR","message":"card declined"}
{"_timestamp":"2026-05-02T10:00:11Z","service_name":"checkout","severity_text":"INFO","message":"order placed"}
{"_timestamp":"2026-05-02T10:00:14Z","service_name":"checkout","severity_text":"ERROR","message":"upstream timeout"}
{"_timestamp":"2026-05-02T10:00:17Z","service_name":"auth","severity_text":"WARN","message":"token near expiry"}
{"_timestamp":"2026-05-02T10:00:21Z","service_name":"auth","severity_text":"ERROR","message":"invalid signature"}
{"_timestamp":"2026-05-02T10:00:24Z","service_name":"shipping","severity_text":"INFO","message":"label printed"}
EOF
```

Response:

```json
{
  "snapshot_id": "...",
  "extent_count": 1,
  "rows_ingested": 8,
  "bytes_written": 6411,
  "replayed": false
}
```

Schema evolution is on by default. New fields in subsequent NDJSON lines
trigger an `ALTER TABLE ADD COLUMN` mid-ingest; history is never rewritten,
older extents simply read as null for the new column.

## A real KQL query

`summarize` is the workhorse. The query below counts errors per service and
returns the top five:

```bash
curl -sS -X POST http://localhost:8080/v1/query \
  -H "X-Database: default" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'hello | where severity_text == "ERROR" | summarize n=count() by service_name | order by n desc | take 5'
```

Response (NDJSON, one row per line):

```json
{"service_name":"payments","n":2}
{"service_name":"checkout","n":1}
{"service_name":"auth","n":1}
```

KQL is parsed by `kyma-kql` (a chumsky-based parser), lowered to SQL, and
executed by DataFusion. Pipe-stage operators like `where`, `summarize`,
`extend`, `project`, `order by`, and `take` map onto the same logical plan
the SQL frontend uses.

## Ask the agent

`POST /v1/agent/ask` runs one agent turn against your data and streams the
result as Server-Sent Events. It is the same surface your own agents will
hit — the engine itself ships a small built-in one for prototyping.

```bash
curl -N -sS -X POST http://localhost:8080/v1/agent/ask \
  -H "Content-Type: application/json" \
  --data '{"question":"which service had the most errors?","database":"default"}'
```

The response is a stream of named SSE frames. The event names you'll see:

| Event           | When                                                       |
|-----------------|------------------------------------------------------------|
| `run_started`   | Once, with `run_id`, `model`, and the question.            |
| `thinking_delta`| Reasoning tokens (only if you pass `"include_thinking":true`). |
| `answer_delta`  | Streaming chunks of the model's answer text.               |
| `tool_call`     | The agent invoked a tool — typically `run_sql` against the catalog. |
| `tool_result`   | The tool returned. For `run_sql`, the rows are inline.     |
| `answer_final`  | The final aggregated answer, plus `sql_used` if any.       |
| `run_error`     | Init failure, runner error, tool-loop budget hit, or wall-clock timeout. |
| `run_finished`  | Always last. Carries `elapsed_ms` and `tool_calls`.        |

The full run is also persisted to `agent_runs` and can be replayed via
`GET /v1/agent/runs/{run_id}`. For an annotated SSE transcript and the full
event-payload schemas, see [Query](/query/).

## What's next

- Concepts that explain the pruning cascade and the agent loop: [Concepts](/concepts/)
- Other ingest frontends — OTLP, Kafka, file-drop: [Ingest](/ingest/)
- KQL reference and the SQL/Flight surfaces: [Query](/query/)
