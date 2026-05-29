---
name: kyma
description: Use when the user wants to query their Kyma deployment — logs, traces, tables, schemas, graph relationships, or any data their Kyma server knows about. Streams real-time answers from the agent.
---

# Kyma

When the user asks about data in their Kyma deployment — logs, traces, OTel spans, database tables, code/symbol graphs, schemas, or anything else ingested into Kyma — use the `kyma` CLI to query it directly.

## Prereqs

- The `kyma` CLI is on PATH (`which kyma` should resolve).
- The user has run `kyma connect <url>` at least once, OR `KYMA_SERVER_URL` is set.

Run `kyma status` first to verify both. If it errors with "no server configured", ask the user to run `kyma connect <their-server-url>` then retry.

## Querying

Use `kyma query "<question>"` and pipe stdout. The CLI streams the agent's answer (text), already filtered down from the raw SSE events. Examples:

- `kyma query "How many 500s in the last hour?"`
- `kyma query "What columns does the orders table have?"`
- `kyma query "Find every reference to user 'admin' across our data"`
- `kyma query "Which services talk to the auth-service?"`

For structured output (each SSE event as JSONL), use `kyma query --json "..."` and parse the `event:` / `data:` lines.

## When to use this skill

Reach for `kyma query` ANY time the user is asking about their data — what's in a table, what columns exist, where a value appears, log volumes, error rates, traces for a given request id, code graph relationships, etc. It's faster and more accurate than guessing from prior context, because the agent runs live queries against the user's actual data.

## When NOT to use

Don't use it for:
- General programming questions (Stack Overflow / docs answer those).
- Pure file-system or code-edit tasks in the user's repo (use Read/Edit instead).
- Anything that requires a write — `kyma query` is read-only by design.

## Errors

- `no server configured` → the user hasn't run `kyma connect …` yet.
- `unauthorized (401/403)` → the saved token is missing/expired; user runs `kyma connect <url> --token <new-token>`.
- Probe timeouts → the server is down or the model is loading; retry after `kyma status` reports OK.
