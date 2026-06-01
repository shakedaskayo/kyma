---
title: Slack
description: Ingest a Slack workspace — channels, messages, threads, mentions, and people — as a queryable property graph, authenticated with OAuth2.
---

# Slack

Ingests a Slack workspace as a property graph: channels, messages, reply
threads, `@`-mentions, and the people behind them. Incremental per channel via
the `oldest` message-timestamp cursor. Slack's Web API returns HTTP 200 with
`{"ok": false}` on logical errors, which the connector handles (a `ratelimited`
response is retried, anything else is a permanent error).

::: tip OAuth connector
Authorize once with **Connect** in the UI. Provider slug `slack`; see
[OAuth connectors](/connectors/oauth) for the Slack app setup. Requested scopes:
`channels:read`, `channels:history`, `groups:read`, `groups:history`,
`users:read`.
:::

## Connect

1. **Connectors → Add connector → Slack**.
2. Name it, **Connect Slack**, approve the workspace install.
3. (Optional) restrict to specific channel ids; empty = public channels the
   token can list.
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `slack_nodes` / `slack_edges` (graph name `slack`); ids
are prefixed `slack:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Channel` | `slack:chan:{id}` | name, is_private, topic |
| `Message` | `slack:msg:{channel}:{ts}` | text (truncated), ts, reply_count |
| `User` | `slack:user:{id}` | name |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `POSTED_IN` | Message → Channel | message location |
| `SENT_BY` | Message → User | author |
| `REPLY_TO` | Message → Message | thread reply → parent |
| `MENTIONS` | Message → User | each `<@U…>` mention |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `channels` | string[] | `[]` | restrict to these channel ids; empty = public channels |
| `max_channels_per_tick` | int | `20` | channels processed per tick (one history page each) |

```json
{
  "credential_id": "0e8f…",
  "channels": [],
  "max_channels_per_tick": 20
}
```

Default sync interval is 10 minutes. Private channels require the app/user to be
a member.
