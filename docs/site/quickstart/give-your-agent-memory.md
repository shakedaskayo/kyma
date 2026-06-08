---
title: Give your agent memory
description: The fastest way into kyma — install the local binary, wire it to your coding agent, and it remembers across sessions, recalling the right context into every prompt. Zero infra, about five minutes.
---

<script setup>
import { withBase } from 'vitepress'
</script>

<p class="kyma-eyebrow">Start here</p>

# Give your agent memory

<p class="kyma-qs-lede">Your coding agent forgets everything when the session ends. kyma fixes that: install the local binary, wire it to your agent, and it recalls durable, graph-aware memory into every prompt. One binary, zero infra — about five minutes.</p>

## 1. Install — zero infra

One command. Installs the `kyma` CLI and a local engine backed by **embedded SQLite + local files** under `~/.kyma` — no Postgres, no Docker, no sudo.

<div class="kyma-terminal">
  <div class="kyma-terminal-bar"><span class="led"></span> kyma ~ install</div>
  <pre><code><span class="prompt">$</span>curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash<span class="caret"></span></code></pre>
</div>

## 2. Wire your agent

Three paths, same engine underneath — they compose, so you can start with one and add the others later.

<div class="kyma-paths">
  <a class="kyma-path kyma-path--primary" :href="withBase('/agent/claude-code-plugin')">
    <span class="kyma-path-kicker">Claude Code · automatic</span>
    <span class="kyma-path-title">The plugin</span>
    <span class="kyma-path-desc">Hooks capture every turn and inject the most relevant memories into <em>every</em> prompt — no tool call. Plus <code>/kyma-recall</code>, <code>/kyma-remember</code>, <code>/kyma-ask</code>. The "it just remembers" path.</span>
    <span class="kyma-path-cmd">kyma install-plugin</span>
    <span class="kyma-path-cta">Set up the plugin →</span>
  </a>
  <a class="kyma-path kyma-path--primary" :href="withBase('/agent/connect')">
    <span class="kyma-path-kicker">Any agent · MCP or CLI</span>
    <span class="kyma-path-title">MCP / CLI + skill</span>
    <span class="kyma-path-desc">Cursor, Windsurf, Aider, Continue — wire the full toolset over stdio MCP, or install a skill that teaches a shell-tool agent <em>when</em> to recall and remember.</span>
    <span class="kyma-path-cmd">kyma setup claude-code</span>
    <span class="kyma-path-cta">Connect any agent →</span>
  </a>
</div>

## 3. Remember, recall

Memory round-trips through the same engine the plugin, the CLI, and the web UI all use:

```bash
kyma remember "payments-svc deploys behind the Aurora gateway; error budget is 0.1%."
kyma recall   "how do we deploy payments and what's the error budget?"
# → returns the memory, scored by vector + keyword + graph
```

Recall is **graph-aware hybrid retrieval** — semantic + keyword, fused with Reciprocal
Rank Fusion, then expanded over the memory graph. It's durable across sessions *and*
machines. With the plugin, you rarely call `recall` yourself — the right memories are
injected into each prompt automatically.

::: tip That's the whole hook
Install → wire → remember/recall. Your agent now carries context across every session.
Everything below is kyma *growing* from there.
:::

## Then it grows

Memory is the front door. The same engine and the same graph keep going:

<div class="kyma-next">
  <a :href="withBase('/quickstart/five-minute-start')">Live data <span>· point logs, traces, and code at it; query in KQL/SQL</span></a>
  <a :href="withBase('/concepts/how-kyma-works')">How it all works <span>· the memory → data → graph model, end to end</span></a>
  <a :href="withBase('/agent/dreaming')">Dreaming <span>· background agent that keeps memory sharp</span></a>
  <a :href="withBase('/concepts/sync')">Sync <span>· keep memory coherent across machines and your team</span></a>
</div>

## Where to go next

- The mental model, end to end: [How kyma works](/concepts/how-kyma-works).
- The full connect surface (plugin · CLI · MCP): [Connect your agent](/agent/connect).
- How recall, the graph, and bi-temporal validity work: [Agentic Memory](/agent/memory).
- Run the whole engine and query live data: [Five-minute start](/quickstart/five-minute-start).
