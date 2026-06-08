---
title: Quickstart
description: Connect a coding agent to the context engine in one command (local, zero infra), or boot the full engine with docker compose and run your first KQL query.
---

<script setup>
import { withBase } from 'vitepress'
</script>

<p class="kyma-eyebrow">Get started</p>

# Quickstart

<p class="kyma-qs-lede">Connect a coding agent to the context engine in one command — local, zero infra. Or boot the full engine and run your first KQL query.</p>

<div class="kyma-terminal">
  <div class="kyma-terminal-bar"><span class="led"></span> kyma ~ install</div>
  <pre><code><span class="prompt">$</span>curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash<span class="caret"></span></code></pre>
</div>

<div class="kyma-paths">
  <a class="kyma-path kyma-path--primary" :href="withBase('/agent/connect')">
    <span class="kyma-path-kicker">Local · zero infra</span>
    <span class="kyma-path-title">Connect your agent</span>
    <span class="kyma-path-desc">Durable memory + live data + graph over MCP — embedded SQLite + local files, no Postgres, no Docker. Plugin, CLI, or MCP.</span>
    <span class="kyma-path-cmd">kyma setup claude-code</span>
    <span class="kyma-path-cta">Start here →</span>
  </a>
  <a class="kyma-path kyma-path--primary" :href="withBase('/quickstart/five-minute-start')">
    <span class="kyma-path-kicker">Full engine</span>
    <span class="kyma-path-title">Five-minute start</span>
    <span class="kyma-path-desc">docker compose up, send one row, query it back. Five minutes from a fresh clone to your first KQL result.</span>
    <span class="kyma-path-cmd">docker compose up</span>
    <span class="kyma-path-cta">Boot the engine →</span>
  </a>
</div>

<p class="kyma-eyebrow">Then</p>

<div class="kyma-next">
  <a :href="withBase('/quickstart/first-real-run')">First real run <span>· a real summarize + the agent endpoint</span></a>
  <a :href="withBase('/quickstart/concepts-cheatsheet')">Concepts cheatsheet <span>· invariants, ports, every KYMA_* var</span></a>
</div>

## What you'll have at the end

A working kyma — engine, Postgres catalog, MinIO object store, Redpanda
broker — running locally on your machine. One table, a handful of rows, proof
that ingest and query both work end to end, and a short list of the ten things
you'll want to try next.

## Where to go after

- The mental model: [Concepts](/concepts/).
- More ways to get data in: [Ingest](/ingest/).
- More ways to ask questions: [Query](/query/).
- Worked examples: [Recipes](/recipes/).
