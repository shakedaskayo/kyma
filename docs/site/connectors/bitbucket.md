---
title: Bitbucket
description: Ingest Bitbucket Cloud repositories, branches, pull requests, and issues into a property graph.
---

# Bitbucket connector

Ingests Bitbucket Cloud metadata into `bitbucket_nodes` and
`bitbucket_edges`. No source parsing yet — metadata only.

## Setup — CLI

```bash
# App-password auth (most common):
export BITBUCKET_TOKEN=<app-password>
kyma create-database bitbucket
kyma connector add bitbucket atlassian/python-bitbucket \
  --username me@example.com \
  --app-password "$BITBUCKET_TOKEN" \
  --start

# Or with a PAT (Bitbucket Server / Data Center):
kyma connector add bitbucket my-ws/my-repo --token bbtok-... --start
```

## Auth

Bitbucket Cloud accepts two auth shapes:

- **App password + username** — stored as a `basic` credential.
  Get one at *Account → Personal settings → App passwords*, scope
  `repository:read` (and `issue:read` for issues).
- **PAT** (Server / Data Center, or token-issuing organizations) —
  stored as a `pat` credential.

The CLI picks the right shape: if both `--username` and
`--app-password` are present, basic auth; otherwise PAT.

## Token sources (PAT path)

1. `--token <pat>`
2. `--credential-id <uuid>`
3. `$BITBUCKET_TOKEN`
4. `$BB_TOKEN`

## Modules

- `repos` — repo nodes (description, language, project, is_private).
- `branches` — branch nodes + `BELONGS_TO_REPO` edges.
- `pulls` — pull-request nodes + `OPENED_BY` / `MERGED_INTO`.
- `issues` — issue nodes + `OPENED_BY` / `CLOSED_BY`.

## Defaults

- Schedule: every 5 minutes (`schedule_ms = 300000`).
- Auth mode: PAT unless `--username` AND `--app-password` are present.
