---
title: GitLab
description: Ingest GitLab projects, branches, merge requests, issues, and members into a property graph.
---

# GitLab data source

Ingests GitLab project metadata into `gitlab_nodes` and
`gitlab_edges`. No source parsing yet — metadata only.

## Setup — CLI

```bash
# gitlab.com
export GITLAB_TOKEN=glpat-...
pensieve create-database gitlab
pensieve datasource add gitlab gitlab-org/gitlab --start

# Self-hosted GitLab
pensieve datasource add gitlab my-group/my-project \
  --api-url https://gitlab.mycorp.io/api/v4 \
  --start
```

## Token sources

In order:
1. `--token <pat>`
2. `--credential-id <uuid>`
3. `$GITLAB_TOKEN`
4. `$GL_TOKEN`

PAT needs `read_api` scope. Leave blank for public projects.

## Modules

- `projects` — project nodes (description, default branch, visibility, stars).
- `branches` — branch nodes + `BELONGS_TO_PROJECT` edges.
- `merge_requests` — MR nodes + `OPENED_BY` / `MERGED_INTO`.
- `issues` — issue nodes + `OPENED_BY` / `CLOSED_BY`.
- `members` — user nodes + `MEMBER_OF` with access level.

All on by default; toggle with `--modules m1,m2,…`.

## Defaults

- Schedule: every 5 minutes (`schedule_ms = 300000`).
- API URL: `https://gitlab.com/api/v4`. Override with `--api-url`.
