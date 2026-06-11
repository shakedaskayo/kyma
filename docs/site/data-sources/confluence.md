---
title: Confluence
description: Ingest Atlassian Confluence — spaces, pages, authors, and the page hierarchy — as a queryable property graph, authenticated with Atlassian OAuth2.
---

# Confluence

Ingests Confluence as a property graph: spaces, pages, their authors, and the
page hierarchy. Fetches the most recently modified pages (bounded per tick) via
CQL search. Shares the Atlassian OAuth app and `cloudId` resolution with the
[Jira](/data-sources/jira) data source.

::: tip OAuth data source
Authorize once with **Connect** in the UI. Provider slug `atlassian` (one app
serves both Confluence and Jira); see [OAuth data sources](/data-sources/oauth) for
setup. Requested scopes: `read:confluence-content.all`,
`read:confluence-space.summary`, `read:confluence-user`, `read:me`,
`offline_access`. The site **cloudId** is resolved automatically.
:::

## Connect

1. **Data sources → Add data source → Confluence**.
2. Name it, **Connect Confluence**, approve access to your site.
3. (Optional) restrict to specific space keys; empty = all accessible.
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `confluence_nodes` / `confluence_edges` (graph name
`confluence`); ids are prefixed `conf:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Space` | `conf:space:{key}` | name |
| `Page` | `conf:page:{id}` | title, status, version |
| `User` | `conf:user:{accountId}` | name |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `CONTAINS` | Space → Page | page lives in a space |
| `CHILD_OF` | Page → Page | immediate parent (last ancestor) |
| `CREATED_BY` / `LAST_EDITED_BY` | Page → User | authorship |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `cloud_id` | string | auto | Atlassian site id; resolved from the token if omitted |
| `spaces` | string[] | `[]` | restrict to these space keys; empty = all |
| `max_pages_per_tick` | int | `5` | CQL search pages per tick (≤50 pages each) |

```json
{
  "credential_id": "0e8f…",
  "spaces": [],
  "max_pages_per_tick": 5
}
```

Default sync interval is 10 minutes.
