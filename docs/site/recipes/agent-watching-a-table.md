---
title: Agent watching a table
description: Schedule the agent endpoint to summarize a table every hour and write the answer back as a kyma row. Self-observable kyma in 30 lines of bash.
---

# Agent watching a table

## The problem

You have an `otel_logs` table that's seen by a dozen humans during the
day and zero humans overnight. Real incidents are visible in the data
hours before someone notices. You want a hourly summary that runs
unattended and writes its answer somewhere queryable — so you can
read it the next morning, chart it, and feed it to alerting.

The agent endpoint plus a tiny shell loop covers this. No new
infrastructure.

## The schema

Two tables. The first is whatever you're watching — `otel_logs` here.
The second is one you create for the recipe:

| Table              | Columns                                                                |
| ------------------ | ---------------------------------------------------------------------- |
| `otel_logs`        | Whatever your OTLP exporter populates.                                 |
| `agent_summaries`  | `_timestamp:timestamp, table_watched:string, question:string, answer:string, run_id:string, model:string` |

Provision the second table:

```bash
kyma-cli create-table \
  --db default \
  --name agent_summaries \
  --schema "_timestamp:timestamp,table_watched:string,question:string,answer:string,run_id:string,model:string"
```

## The script

A 30-line shell loop. Save as `kyma-watcher.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

KYMA="${KYMA_URL:-http://localhost:8080}"
TABLE="${TABLE:-otel_logs}"
QUESTION="${QUESTION:-Summarize the last hour of $TABLE. What's notable about the error patterns? Are any services anomalous?}"
DB="${DB:-default}"

# Stream the agent's response and capture the answer_final + run_id.
RESPONSE=$(
  curl -sN -X POST "$KYMA/v1/agent/ask" \
    -H "Content-Type: application/json" \
    -d "{\"question\":\"$QUESTION\",\"database\":\"$DB\"}"
)

# Pull run_id from run_started, answer text from answer_final, model from run_started.
RUN_ID=$(echo "$RESPONSE" | awk '/^event: run_started$/{getline; sub(/^data: /,""); print; exit}' | jq -r .run_id)
MODEL=$(echo "$RESPONSE"  | awk '/^event: run_started$/{getline; sub(/^data: /,""); print; exit}' | jq -r .model)
ANSWER=$(echo "$RESPONSE" | awk '/^event: answer_final$/{getline; sub(/^data: /,""); print; exit}' | jq -r .text)

# Ingest the row back into kyma. Idempotency-key uses the run_id, so
# replays are no-ops at the catalog boundary.
NOW=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
ROW=$(jq -cn \
  --arg ts "$NOW" --arg t "$TABLE" --arg q "$QUESTION" \
  --arg a "$ANSWER" --arg r "$RUN_ID" --arg m "$MODEL" \
  '{_timestamp:$ts, table_watched:$t, question:$q, answer:$a, run_id:$r, model:$m}')

curl -sS -X POST "$KYMA/v1/ingest" \
  -H "X-Database: $DB" \
  -H "X-Table: agent_summaries" \
  -H "X-Idempotency-Key: agent-watcher-$RUN_ID" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary "$ROW"
```

Schedule it via `cron`, `systemd-timer`, GitHub Actions, or whatever
already runs your other periodic jobs:

```cron
0 * * * * /usr/local/bin/kyma-watcher.sh > /var/log/kyma-watcher.log 2>&1
```

## Reading the summaries back

The `agent_summaries` table is now a normal kyma table. Query it the
same way you'd query anything else:

```kql
agent_summaries
| where _timestamp > ago(24h)
| where table_watched == "otel_logs"
| project _timestamp, answer
| order by _timestamp desc
```

The agent's previous answers, oldest at the top, become a daily
operational diary. If something interesting fires at 3 AM, the answer
is sitting there at 9 AM ready to read.

## What you should see

The `agent_summaries` rows look like this:

| _timestamp           | answer (truncated)                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| 2026-05-03T14:00:00Z | Last hour: 14,283 logs across 8 services. ERROR rate ~0.4% (typical). One short spike on payments-svc at 13:48Z (7× …) |
| 2026-05-03T13:00:00Z | Last hour: 13,810 logs across 8 services. ERROR rate ~0.3%. No anomalies; checkout-svc latency p99 trending higher (...) |
| 2026-05-03T12:00:00Z | Last hour: 14,001 logs. Quiet. One ECONNREFUSED burst from auth-svc at 11:14Z, recovered within 90s. (...)               |

## Variations

- **Different question:** override `QUESTION` env var. The agent answers
  whatever you ask; the recipe's structure doesn't change.
- **Different cadence:** change the cron line. Every 5 minutes is fine
  for high-signal tables; daily is fine for slow ones.
- **Per-service summaries:** loop over services in the bash script,
  one agent call per service. Set `X-Idempotency-Key` to include the
  service name.
- **Alert on answers:** add a second query that filters
  `agent_summaries` for keywords ("anomalous", "spike", "error") and
  pages on a hit. The agent's prose is informally structured — mine it
  for known patterns rather than expecting JSON.
