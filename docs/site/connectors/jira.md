---
title: Jira
description: Ingest Jira Cloud — projects, issues, epics, assignees, and issue links — as a queryable property graph, authenticated with Atlassian OAuth2.
---

# Jira

Ingests Jira Cloud as a property graph: projects, issues, epics, the people
assigned to and reporting them, and the **links between issues**. Fetches the
most recently updated issues (bounded per tick) and reuses the same graph node
labels — `Issue`, `User`, `Project` — as the code connectors, so cross-vendor
views line up.

::: tip OAuth connector
Authorize once with **Connect** in the UI. Provider slug `atlassian` (one app
serves both Jira and Confluence); see [OAuth connectors](/connectors/oauth) for
the Atlassian developer-console setup. Requested scopes: `read:jira-work`,
`read:jira-user`, `read:me`, `offline_access`. The site **cloudId** is resolved
automatically from the token's accessible resources.
:::

## Connect

1. **Connectors → Add connector → Jira**.
2. Name it, **Connect Jira**, approve access to your site.
3. (Optional) restrict to specific project keys; empty = all accessible.
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `jira_nodes` / `jira_edges` (graph name `jira`); ids are
prefixed `jira:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Project` | `jira:proj:{key}` | name |
| `Issue` / `Epic` | `jira:issue:{key}` | summary, status, issuetype, updated |
| `User` | `jira:user:{accountId}` | name |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `HAS_ISSUE` | Project → Issue | issue belongs to a project |
| `ASSIGNED_TO` / `REPORTED_BY` | Issue → User | assignee / reporter |
| `CHILD_OF` | Issue → Issue/Epic | sub-task / epic parent |
| `LINKS_TO` | Issue → Issue | issue link (carries the link type, e.g. *blocks*) |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `cloud_id` | string | auto | Atlassian site id; resolved from the token if omitted |
| `projects` | string[] | `[]` | restrict to these project keys; empty = all |
| `max_pages_per_tick` | int | `5` | `/search` pages per tick (≤100 issues each) |

```json
{
  "credential_id": "0e8f…",
  "projects": [],
  "max_pages_per_tick": 5
}
```

Default sync interval is 10 minutes.
