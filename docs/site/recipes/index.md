---
title: Recipes
description: Worked examples — copy, paste, adapt. Each one runs against a real kyma instance using only shipped surfaces.
---

# Recipes

Each recipe follows the same template: **the problem**, **the schema** it
operates on, **the queries**, **what you should see**, and **variations**.
Every snippet here uses only what's shipped today — the recipe runs as
written.

When DB-M1 ships, a federation recipe lands. When OTLP traces and
metrics ship, more observability recipes land. The list grows with the
engine.

<div class="feature-grid">

<div class="feature-card">

### [Debug a prod incident across services](/recipes/debug-prod-incident)

Three KQL queries take you from "checkout broke" to a single trace ID
spanning three services. The textbook pruning shape — time bound,
service equality, top-N rank.

</div>

<div class="feature-card">

### [Triage an alert spike](/recipes/triage-alert-spike)

A WARN/ERROR rate that doubled in the last fifteen minutes. Bucket by
time, group by error code, project the signal that explains it.

</div>

<div class="feature-card">

### [Agent watching a table](/recipes/agent-watching-a-table)

Schedule the agent endpoint to summarize an `otel_logs` table every hour
and write the answer back as a kyma row. Self-observable kyma in 30
lines.

</div>

<div class="feature-card">

### [Ingest something custom](/recipes/ingest-something-custom)

Take an arbitrary JSON event source — a webhook, a cron job's output,
your own metric — and get it into kyma in a way that prunes well.
Idempotency, schema evolution, the right column types from the start.

</div>

</div>

## Recipe template

If you write your own, follow the same shape. It's there because it
works:

1. **The problem.** One paragraph. What broke, what question, what
   pressure. No "imagine" or "let's say"; speak to the actual situation.
2. **The schema.** Which tables. Which columns matter. If anything was
   pre-provisioned, name it. Use real type signatures.
3. **The queries.** One per H3, in order. Each prints something useful
   on its own; chained, they walk to the answer.
4. **What you should see.** A small table — five-ish rows of expected
   output. Lets readers spot when their data is wrong vs. when their
   query is wrong.
5. **Variations.** One-line each. "If you want X instead, replace Y
   with Z." Keep the recipe focused; let the variations branch.
