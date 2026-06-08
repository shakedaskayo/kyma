---
title: Quickstart
description: Give your coding agent memory in one command — local, zero infra — then grow into live data (KQL/SQL) and a graph of your whole stack.
---

<script setup>
import { withBase } from 'vitepress'
</script>

<p class="kyma-eyebrow">Get started</p>

# Quickstart

<p class="kyma-qs-lede">Give your coding agent memory in one command — local, zero infra. Then watch it grow into live data and a graph of your whole stack.</p>

<div class="kyma-terminal">
  <div class="kyma-terminal-bar"><span class="led"></span> kyma ~ install</div>
  <pre><code><span class="prompt">$</span>curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash<span class="caret"></span></code></pre>
</div>

<p class="kyma-eyebrow">Start here</p>

<a class="kyma-path kyma-path--primary" :href="withBase('/quickstart/give-your-agent-memory')">
  <span class="kyma-path-kicker">Local · zero infra · ~5 min</span>
  <span class="kyma-path-title">Give your agent memory</span>
  <span class="kyma-path-desc">Install, wire it to your coding agent (plugin / CLI / MCP), and it recalls durable, graph-aware memory into every prompt — across sessions and machines. This is the fastest, most useful way in.</span>
  <span class="kyma-path-cmd">kyma setup claude-code</span>
  <span class="kyma-path-cta">Start here →</span>
</a>

<p class="kyma-eyebrow">Go further</p>

<div class="kyma-next">
  <a :href="withBase('/quickstart/five-minute-start')">Five-minute start <span>· run the full engine — docker compose, ingest, KQL</span></a>
  <a :href="withBase('/concepts/how-kyma-works')">How kyma works <span>· memory → data → graph, plus modes, dreaming, sync</span></a>
  <a :href="withBase('/quickstart/first-real-run')">First real run <span>· a batched ingest, a real summarize, the agent endpoint</span></a>
  <a :href="withBase('/quickstart/concepts-cheatsheet')">Concepts cheatsheet <span>· the invariants, default ports, every KYMA_* var</span></a>
</div>

## The shape of it

kyma grows in three steps — each one is the same engine and the same graph, doing more:

1. **Memory** — your agent remembers across sessions ([Give your agent memory](/quickstart/give-your-agent-memory)).
2. **Live data** — point logs, traces, and code at it; query in KQL/SQL ([Five-minute start](/quickstart/five-minute-start)).
3. **The graph** — connectors build a property graph and memory links to the real resources it's about ([How kyma works](/concepts/how-kyma-works)).

## Where to go after

- The mental model, end to end: [How kyma works](/concepts/how-kyma-works) · [Concepts](/concepts/).
- More ways to get data in: [Ingest](/ingest/) · [Connectors](/connectors/).
- More ways to ask questions: [Query](/query/).
- Worked examples: [Recipes](/recipes/).
