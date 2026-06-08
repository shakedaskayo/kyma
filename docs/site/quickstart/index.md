---
title: Quickstart
description: Connect a coding agent to the context engine in one command (local, zero infra), or boot the full engine with docker compose and run your first KQL query.
---

# Quickstart

Two ways to start, depending on what you want first:

- **Connect a coding agent** to the context engine — the fastest path, a local
  single binary with no infra.
- **Run the full engine** (Postgres + object store + connectors + web app) with
  docker compose, and query it in KQL/SQL.

<div class="feature-grid">

<div class="feature-card">

### [Connect your agent](/agent/connect) · local, zero infra

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash
kyma setup claude-code     # or: cursor · windsurf
```

Your agent gets durable memory + live data + graph over MCP — embedded SQLite +
local files, no Postgres, no Docker. Or use the plugin / CLI. **Start here** if
you're wiring kyma into a coding agent.

</div>

<div class="feature-card">

### [Five-minute start](/quickstart/five-minute-start) · the full engine

`docker compose up`, send one row, query it back. Five minutes from a
fresh clone to your first KQL result. No language toolchain, no cloud
account, no auth setup.

</div>

<div class="feature-card">

### [First real run](/quickstart/first-real-run)

A multi-row batch, a real KQL `summarize`, and one call to the agent
endpoint. Same engine, slightly more interesting questions. About ten
minutes.

</div>

<div class="feature-card">

### [Concepts cheatsheet](/quickstart/concepts-cheatsheet)

A one-page reference. The five invariants in one line each, the data
shape, default endpoints and ports, every `KYMA_*` env var the binary
reads. Bookmark this.

</div>

</div>

## What you'll have at the end

A working kyma — engine, Postgres catalog, MinIO object store, Redpanda
broker — running locally on your machine. One table, a handful of rows,
proof that ingest and query both work end to end. A short list of
follow-ups for the ten things you'll want to try next.

## Where to go after

- The mental model: [Concepts](/concepts/).
- More ways to get data in: [Ingest](/ingest/).
- More ways to ask questions: [Query](/query/).
- Worked examples: [Recipes](/recipes/).
