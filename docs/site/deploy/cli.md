---
title: kyma deploy (CLI)
description: The kyma deploy wizard — init, up, status, destroy; targets, flags, credential acquisition, workspaces.
---

# `kyma deploy`

A fly-launch-style wizard that provisions the [production
topology](./index) for you. It renders the IaC into a private workspace,
runs Terraform (or Pulumi) with live output, and feeds the resulting
engine URL back into `kyma connect`.

## Commands

| Command | What it does |
| ------- | ------------ |
| `kyma deploy init` | Wizard: prereq checks, credential acquisition, settings → materializes `~/.kyma/deploy/<name>/`. |
| `kyma deploy up` | `terraform init && apply` (or `pulumi up`, or `docker run` for the local target). Prints the engine URL + sign-in summary. |
| `kyma deploy status` | Workspace summary, IaC outputs, live `/health` probe. |
| `kyma deploy destroy` | Tears the deployment down (with confirmation). |

## `init` flags

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--name` | `prod` | Workspace name — run several deployments side by side. |
| `--target` | `aws` | `aws` (full production stack) or `local` (Supabase-backed docker test drive). |
| `--tool` | `terraform` | `terraform` or `pulumi` (aws target). |
| `--region` | prompt / `us-east-1` | AWS + Supabase region. |
| `--supabase-org` | interactive picker | Supabase organization id. |
| `--domain` | prompt / none | Custom domain (ACM + HTTPS; Route53 fully automates DNS). |
| `--admin-email` | prompt | Comma-separated emails granted the kyma admin role. |
| `--yes` | off | Non-interactive (needs `--supabase-org` + a token source). |
| `--print-only` | off | Render config + print planned commands; runs nothing. |
| `--force` | off | Regenerate a workspace's rendered config. |

## Credential acquisition

`init` obtains tokens on your behalf where possible, in this order:

1. `SUPABASE_ACCESS_TOKEN` environment variable
2. Your Supabase CLI login (`~/.supabase/access-token` from `supabase login`)
3. Browser OAuth (authorization-code + PKCE) — when a Supabase OAuth app
   client id is configured via `KYMA_SUPABASE_OAUTH_CLIENT_ID`
4. Guided paste (the wizard prints the dashboard URL that mints a token)

AWS credentials use the standard chain (`aws configure`, SSO, env vars);
the wizard checks and tells you exactly what to run when missing.

## Workspaces

Everything lives in `~/.kyma/deploy/<name>/`:

```
terraform/             # materialized stack (embedded in the CLI binary)
terraform/terraform.tfvars   # rendered answers — 0600, contains the DB password
pulumi/typescript/     # the Pulumi consumer (when --tool pulumi)
local.env              # local-target env file (0600)
deploy.json            # target/tool/state for up/status/destroy
```

Re-running `init` refreshes the templates but never overwrites your
`terraform.tfvars` (use `--force` to regenerate). Terraform state stays in
the workspace by default; see [Terraform → State](./terraform#state) for
the team setup.

## Local target

`--target local` provisions **only** a Supabase project (via the
Management API), writes `local.env` with the catalog URL (session pooler —
the direct DB host is IPv6-only), Supabase Auth wiring, and Supabase
Storage as the extents store (the wizard opens the dashboard's S3-keys
page — Supabase has no API for those keys; skip to keep extents on the
docker volume). `up` runs `ghcr.io/shakedaskayo/kyma-engine` on port 8080;
`destroy` removes the container, its volume, and the Supabase project.
