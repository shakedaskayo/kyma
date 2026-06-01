---
title: Google Drive
description: Ingest Google Drive — files, folders, owners, and the folder tree — as a queryable property graph (metadata only), authenticated with OAuth2.
---

# Google Drive

Ingests Drive as a property graph: files, folders, their owners, and the folder
structure that connects them. **Metadata only** — it never downloads file
content (scope `drive.metadata.readonly`). Walks `files.list` page-by-page,
carrying the page token in its cursor so a large drive is covered across ticks
and then re-walked to pick up changes.

::: tip OAuth connector
Authorize once with **Connect** in the UI. Provider slug `google` (shared with
Gmail); see [OAuth connectors](/connectors/oauth) for the Google Cloud app
setup. Requested scope: `https://www.googleapis.com/auth/drive.metadata.readonly`.
:::

## Connect

1. **Connectors → Add connector → Google Drive**.
2. Name it, **Connect Google Drive**, approve access.
3. (Optional) restrict to specific folder ids, or enable shared drives.
4. Pick a sync interval and **Create**.

## What it ingests

A property graph under `googledrive_nodes` / `googledrive_edges` (graph name
`googledrive`); ids are prefixed `gdrive:`.

**Nodes**

| Label | Id | Key props |
|---|---|---|
| `Folder` | `gdrive:{fileId}` | name, modifiedTime, webViewLink, trashed |
| `File` | `gdrive:{fileId}` | name, mimeType, size, modifiedTime, webViewLink |
| `User` | `gdrive:user:{email}` | name, email |

**Edges**

| Type | From → To | Meaning |
|---|---|---|
| `CONTAINS` | Folder → File/Folder | folder structure (one per parent) |
| `OWNED_BY` | File/Folder → User | ownership |

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `credential_id` | uuid | — (required) | the OAuth2 credential (set by the Connect flow) |
| `folders` | string[] | `[]` | restrict to direct children of these folder ids; empty = whole drive |
| `include_shared_drives` | bool | `false` | also walk shared drives |
| `max_pages_per_tick` | int | `10` | `files.list` pages per tick (≤100 files each) |

```json
{
  "credential_id": "0e8f…",
  "folders": [],
  "include_shared_drives": false,
  "max_pages_per_tick": 10
}
```

Default sync interval is 10 minutes.
