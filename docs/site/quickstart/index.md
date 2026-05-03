---
title: Quickstart
description: From docker compose up to your first KQL query in five minutes. Then a slightly deeper walkthrough, and a one-screen reference card for the rest.
---

# Quickstart

Three pages, in order. The first gets you a running engine and a single
query. The second goes one step deeper — a real KQL query, the agent
endpoint, an idea of what's possible. The third is a one-screen
reference card for when you've forgotten which port is which.

<div class="feature-grid">

<div class="feature-card">

### [Five-minute start](/quickstart/five-minute-start)

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
