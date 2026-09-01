---
title: Debug a prod incident from your editor
description: The full loop — your coding agent recalls similar past incidents, queries live logs and traces, and walks the service graph to the failing call, without leaving the IDE.
---

# Debug a prod incident from your editor

This is pensieve's whole thesis in one task: **memory + live data + the graph**, used by an
agent in your editor. A user filed "checkout broke around 14:32." You ask your agent.
It doesn't guess — it remembers, queries, and traverses.

## Setup

Your agent is [connected](/agent/connect), your services ship OTLP logs to pensieve, and your
repo is [connected as a graph](/data-sources/github). Now, in the editor:

> **You:** checkout is throwing 502s around 14:32 — what broke?

The agent runs the loop below over MCP (`recall_memory` → `run_kql` → `graph_traverse`).
Each step is a tool you can also run by hand.

## 1. Recall — has this happened before?

Before touching logs, the agent recalls prior context:

```bash
pensieve recall "checkout 502 / payments timeout incidents"
# → "2026-04-20: checkout 502s were stripe-api timeouts in payments-svc; error budget 0.1%"
```

That memory narrows the search instantly — last time, the culprit was `payments-svc`.

## 2. Query the live data — what's erroring now?

Bound by time, filter to errors, count by service — the textbook pruning shape:

```kql
otel_logs
| where _timestamp between (datetime("2026-05-03T14:30:00Z") .. datetime("2026-05-03T14:35:00Z"))
| where severity_text == "ERROR"
| summarize n = count() by service_name
| order by n desc
```

| service_name | n |
|---|---|
| checkout-svc | 247 |
| payments-svc | 89 |
| web-svc | 14 |

Then follow one request across services by `trace_id`:

```kql
otel_logs
| where trace_id == "abc1234…"
| project _timestamp, service_name, severity_text, body
| order by _timestamp asc
```

| _timestamp | service_name | severity_text | body |
|---|---|---|---|
| 14:32:08.131 | checkout-svc | INFO | charging via payments-svc |
| 14:32:11.487 | payments-svc | ERROR | upstream timeout: stripe-api 30000ms |
| 14:32:11.489 | web-svc | ERROR | 502 Bad Gateway |

## 3. Walk the graph — what changed?

The trace points at `payments-svc`. The agent walks the graph from that service to find
the recent deploy and the PR behind it:

```kql
context_edges | graph-traverse source "payments-svc" from src to dst max-hops 2
| where kind == "deploy" and at > ago(1h)
```

One hop to a deploy 12 minutes before the spike, another to the PR that bumped the Stripe
client timeout. From "got the bug" to "here's the PR" — in the editor, in about a minute.

## Why this needs all three

A memory tool would recall the past incident but couldn't see the live logs. An
observability tool would show the errors but not the decision history or the repo graph.
pensieve does the whole loop on one surface — which is the only reason the agent can answer
end-to-end without you stitching three tools together.

## Variations

- **No specific window:** replace the `between` filter with `where _timestamp > ago(15m)`.
- **Error rate over time:** add `extend bucket = bin(_timestamp, 30s)` and group by it.
- **Let the agent narrate it:** `pensieve query "what broke in checkout around 14:32 and what deploy caused it?"` runs the same shape through the [agent endpoint](/use-cases/ask-your-stack).

## Next

- The memory half: [Your agent remembers across sessions](/use-cases/remember-across-sessions).
- The query language: [KQL](/query/kql).
- The graph: [Querying graphs](/query/graph).
