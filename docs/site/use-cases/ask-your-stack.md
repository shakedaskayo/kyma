---
title: Ask your whole stack in plain English
description: Turn natural-language questions into KQL/SQL across logs, traces, data sources, and your operational databases — via the built-in agent endpoint, the CLI, or any connected coding agent.
---

# Ask your whole stack in plain English

## The problem

Most questions about production are one-offs. "Why is checkout slow today?" "Which
customers hit 5xx this morning?" "Is AWS spend up, and where?" You don't want to hand-write
KQL for each one, remember every table's schema, or open three dashboards. You want to ask.

kyma ships a built-in agent that turns a plain-English question into the same query you'd
have written — and runs it over every signal in one engine.

## Three ways to ask

Same engine underneath; pick the surface:

```bash
# 1. From your terminal — streams the answer to stdout
kyma query "p99 latency on /api/checkout in the last hour, by version"

# 2. From any connected coding agent (MCP) — it calls run_kql / run_sql for you
#    "ask kyma which services errored most in the last 15 minutes"

# 3. Over HTTP — Server-Sent Events from the agent endpoint
curl -N -X POST $KYMA/v1/agent/ask -H 'content-type: application/json' \
  -d '{"question":"which customers hit checkout 5xx today?"}'
```

## What you can ask

The agent reasons across everything kyma holds — not just logs:

- **Telemetry** — "errors per service in the last hour", "p99 latency on `/api/checkout`".
- **Data sources & DBs** — "top customers by support tickets this week" (federated to Postgres),
  "AWS spend by team this month" (billing exports ingested as a table).
- **Agent + memory** — "what did we decide about the payments gateway?", "which sessions
  called `send_email` with `draft:true` last Tuesday?"

## How it works

- **NL → the right query.** The agent picks KQL or SQL, writes it, runs it through the same
  [three-level pruning cascade](/concepts/the-pruning-cascade), and streams Arrow back —
  so it can ask twenty exploratory questions per prompt without melting a credit card.
- **Schema-accurate.** Schema RAG keeps the agent's mental model of your tables current as
  they evolve, so it doesn't hallucinate columns. Read-only by design.
- **Your choice of model.** Pick Anthropic, OpenAI, Ollama, or your local Claude Code OAuth
  in `/settings#engine` — see [Engines](/agent/engines).

## What you'll see

```
$ kyma query "which services errored most in the last 15 minutes?"
checkout-svc   247
payments-svc    89
web-svc         14
(KQL: otel_logs | where _timestamp > ago(15m) and severity_text=="ERROR"
      | summarize n=count() by service_name | order by n desc)
```

The agent shows you the query it ran — so you can trust it, tweak it, or save it.

## Next

- The deeper incident loop: [Debug a prod incident from your editor](/use-cases/debug-from-your-editor).
- The languages it writes: [KQL](/query/kql) · [SQL](/query/sql).
- The agent endpoint internals: [The agent loop](/concepts/the-agent-loop).
