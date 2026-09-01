---
title: Quickstart
description: Give your coding agent memory in one command — local, zero infra — then grow into live data (KQL/SQL) and a graph of your whole stack.
---

<script setup>
import { withBase } from 'vitepress'
</script>

<p class="pensieve-eyebrow">Get started</p>

# Quickstart

<p class="pensieve-qs-lede">Give your coding agent memory in one command — local, zero infra. Then watch it grow into live data and a graph of your whole stack.</p>

<div class="pensieve-terminal">
  <div class="pensieve-terminal-bar"><span class="led"></span> pensieve ~ install</div>
  <pre><code><span class="prompt">$</span>curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash<span class="caret"></span></code></pre>
</div>

<p class="pensieve-eyebrow">Start here</p>

<a class="pensieve-path pensieve-path--primary" :href="withBase('/quickstart/give-your-agent-memory')">
  <span class="pensieve-path-kicker">Local · zero infra · ~5 min</span>
  <span class="pensieve-path-title">Give your agent memory</span>
  <span class="pensieve-path-desc">Install, wire it to your coding agent (plugin / CLI / MCP), and it recalls durable, graph-aware memory into every prompt — across sessions and machines. This is the fastest, most useful way in.</span>
  <span class="pensieve-path-cmd">pensieve setup claude-code</span>
  <span class="pensieve-path-cta">Start here →</span>
</a>

<p class="pensieve-eyebrow">Go further</p>

<div class="pensieve-next">
  <a :href="withBase('/quickstart/five-minute-start')">Five-minute start <span>· run the full engine — docker compose, ingest, KQL</span></a>
  <a :href="withBase('/concepts/how-pensieve-works')">How pensieve works <span>· memory → data → graph, plus modes, dreaming, sync</span></a>
  <a :href="withBase('/quickstart/first-real-run')">First real run <span>· a batched ingest, a real summarize, the agent endpoint</span></a>
  <a :href="withBase('/quickstart/concepts-cheatsheet')">Concepts cheatsheet <span>· the invariants, default ports, every PENSIEVE_* var</span></a>
</div>

## The shape of it

pensieve grows in three steps — each one is the same engine and the same graph, doing more:

1. **Memory** — your agent remembers across sessions ([Give your agent memory](/quickstart/give-your-agent-memory)).
2. **Live data** — point logs, traces, and code at it; query in KQL/SQL ([Five-minute start](/quickstart/five-minute-start)).
3. **The graph** — data sources build a property graph and memory links to the real resources it's about ([How pensieve works](/concepts/how-pensieve-works)).

## Where to go after

- The mental model, end to end: [How pensieve works](/concepts/how-pensieve-works) · [Concepts](/concepts/).
- More ways to get data in: [Ingest](/ingest/) · [Data sources](/data-sources/).
- More ways to ask questions: [Query](/query/).
- What it's for: [Use cases](/use-cases/).
