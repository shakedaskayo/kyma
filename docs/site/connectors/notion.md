---
title: Notion
description: Ingest a Notion workspace — pages, databases, users, and their relations — as a queryable property graph, authenticated with OAuth2.
---

# Notion

Ingests a Notion workspace as a property graph: pages, databases, the people who
authored them, and — most usefully — the **relations** between pages. Walks the
workspace via Notion's `search` API sorted by `last_edited_time`, so ticks are
incremental (a stored floor stops the walk once it reaches already-seen items).

::: tip OAuth connector
Authorize once with **Connect** in the UI — there's no token to paste. Provider
slug `notion`; see [OAuth connectors](/connectors/oauth) for the one-time
provider-app setup. Notion is capability-based: grant the integration **read
content** + **read user information** when you authorize.
:::

## Connect

1. **Connectors → Add connector → Notion**.
2. Name it, click **Connect Notion**, and approve access in the Notion window.
3. (Optional) restrict to specific databases; leave empty for the whole
   workspace the integration can see.
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `notion_nodes` / `notion_edges` (graph name `notion`); ids
are prefixed `notion:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Database` | `notion:db:{id}` | title, url, last_edited_time |
| `Page` | `notion:page:{id}` | title, url, archived, last_edited_time |
| `User` | `notion:user:{id}` | — |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `CONTAINS` | Database → Page | page lives in a database |
| `CHILD_OF` | Page → Page | sub-page of a parent page |
| `CREATED_BY` / `LAST_EDITED_BY` | Page/Database → User | authorship |
| `RELATES_TO` | Page → Page | a Notion **relation** property value (carries the property name) |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `databases` | string[] | `[]` | restrict to these database ids; empty = all |
| `max_pages_per_tick` | int | `10` | `search` pages fetched per tick (≤100 items each) |

```json
{
  "credential_id": "0e8f…",
  "databases": [],
  "max_pages_per_tick": 10
}
```

Default sync interval is 10 minutes. The connector resolves and refreshes the
OAuth token automatically each tick — see [OAuth connectors](/connectors/oauth).
