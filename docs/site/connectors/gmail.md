---
title: Gmail
description: Ingest Gmail as a correspondent graph — threads, messages, and the people you email — from header metadata only, authenticated with OAuth2.
---

# Gmail

Ingests Gmail as a **correspondent graph**: threads, messages, and the people
you email. **Header metadata only** — it never reads message bodies (scope
`gmail.metadata`). Lists message ids page-by-page (carrying the page token in its
cursor across ticks), then fetches `From`/`To`/`Cc`/`Subject`/`Date` headers for
each.

::: tip OAuth connector
Authorize once with **Connect** in the UI. Provider slug `google` (shared with
Google Drive); see [OAuth connectors](/connectors/oauth). Requested scope:
`https://www.googleapis.com/auth/gmail.metadata` (the lowest-privilege Gmail
scope — headers and labels, no bodies).
:::

## Connect

1. **Connectors → Add connector → Gmail**.
2. Name it, **Connect Gmail**, approve access.
3. (Optional, **Advanced**) set a Gmail search `query` to scope which messages
   are ingested (default `newer_than:1y`).
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `gmail_nodes` / `gmail_edges` (graph name `gmail`); ids
are prefixed `gmail:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Thread` | `gmail:thread:{threadId}` | subject |
| `Message` | `gmail:msg:{id}` | subject, snippet, labelIds |
| `User` | `gmail:user:{email}` | name, email |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `IN_THREAD` | Message → Thread | message belongs to a thread |
| `SENT` | User → Message | the `From` correspondent |
| `RECEIVED` | Message → User | each `To` / `Cc` correspondent |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `query` | string | `newer_than:1y` | Gmail search query scoping which messages are ingested |
| `max_messages_per_tick` | int | `100` | hard cap on messages fetched per tick |

```json
{
  "credential_id": "0e8f…",
  "query": "newer_than:1y",
  "max_messages_per_tick": 100
}
```

Default sync interval is 15 minutes.
