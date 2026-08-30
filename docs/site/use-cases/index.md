---
title: Use cases
description: What pensieve is for — organized along the memory → data → graph spine. Give your coding agent durable memory, ground it in live data, connect everything in the graph, and make anything queryable.
---

<script setup>
import { withBase } from 'vitepress'
</script>

<p class="pensieve-eyebrow">What it's for</p>

# Use cases

Each one is the same engine — **memory**, **live data**, and the **graph** that links
them — doing real work for a coding agent. They build along the spine: memory first, then
the live data it's grounded in, then the graph that connects it all, then anything else
your org runs on.

<p class="pensieve-eyebrow">Memory</p>

<div class="feature-grid">

<div class="feature-card">

### [Your agent remembers across sessions](/use-cases/remember-across-sessions)

Stop re-explaining your codebase. Architectural decisions, conventions, and gotchas
persist and get recalled into every prompt — across sessions and machines.

```bash
pensieve remember "Auth is Supabase JWT; prefer KQL over SQL in examples."
```

</div>

<div class="feature-card">

### [Onboard an engineer's agent in a day](/agent/connect)

Point a new hire's agent at the team's shared memory and the repo graph, sync once, and
it already knows the architecture, the services, and the conventions.

```bash
PENSIEVE_CLOUD_URL=https://pensieve.your-co.dev pensieve sync
```

</div>

</div>

<p class="pensieve-eyebrow">Live data</p>

<div class="feature-grid">

<div class="feature-card">

### [Debug a prod incident from your editor](/use-cases/debug-from-your-editor)

The agent recalls similar past incidents, queries live logs and traces, and walks the
service graph to the failing call — without you leaving the IDE.

```kql
otel_logs | where _timestamp > ago(15m) and severity_text == "ERROR"
| summarize n = count() by service_name | order by n desc
```

</div>

<div class="feature-card">

### [Ask your whole stack in plain English](/use-cases/ask-your-stack)

"Why is checkout slow today?" → the agent turns it into KQL/SQL across logs, traces,
data sources, and your operational databases, and streams the answer.

```bash
pensieve query "p99 latency on /api/checkout in the last hour, by version"
```

</div>

</div>

<p class="pensieve-eyebrow">The graph</p>

<div class="feature-grid">

<div class="feature-card">

### [Trace one customer across every service](/query/graph)

From a customer to their auth events to the deploy that broke their checkout — one graph
traversal, not five dashboards open side by side.

```kql
context_edges | graph-traverse source "cus_42" from src to dst max-hops 3
```

</div>

<div class="feature-card">

### [Turn your repo into a queryable graph](/data-sources/github)

Connect GitHub and the agent traverses code, PRs, issues, and people — and your memories
link to the real files and services they're about.

```bash
pensieve datasource add github your-org/your-repo --start
```

</div>

</div>

<p class="pensieve-eyebrow">Anything</p>

<div class="feature-grid">

<div class="feature-card">

### [Make any source queryable](/ingest/)

Webhooks, billing exports, CI outcomes, your own metrics — one POST and it's queryable,
agent-readable data that prunes well from day one.

```bash
curl -X POST $PENSIEVE/v1/ingest -H "X-Table: github_actions" --data-binary @runs.ndjson
```

</div>

</div>

## Build your own

These are starting points, not a closed list — anything with a timestamp and a body becomes
memory-adjacent, queryable data. The mental model is [How pensieve works](/concepts/how-pensieve-works);
the surfaces are [Ingest](/ingest/), [Query](/query/), and [Connect your agent](/agent/connect).
