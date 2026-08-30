---
title: pensieve deploy (CLI)
description: The pensieve deploy wizard — init/up/status/destroy across compute (fargate/eks/helm/local), database, storage, and auth backends.
---

# `pensieve deploy`

A wizard that provisions the [deployment topology](./index) for you. It resolves
each axis (flag → interactive prompt → default), validates the combination,
renders the right artifact into a private workspace, and runs the tool for you.

## Commands

| Command | What it does |
| ------- | ------------ |
| `pensieve deploy init` | Resolve axes + credentials, materialize `~/.pensieve/deploy/<name>/`, render the artifact. |
| `pensieve deploy up` | Provision: `terraform`/`pulumi apply` (fargate/eks; EKS then runs `helm upgrade --install`), `helm upgrade --install` (helm), or `docker run` (local). Prints the engine URL. |
| `pensieve deploy status` | Workspace summary (compute/db/storage/auth), outputs, live `/health` probe. |
| `pensieve deploy destroy` | Tear down (with confirmation). |

## Axis flags

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--compute` | prompt / `fargate` | `fargate` · `eks` · `helm` · `local`. |
| `--database` | prompt / `supabase` | `supabase` · `rds` · `external`. |
| `--database-url` | prompt (secret) | `postgresql://…` for `--database external`. |
| `--storage` | derived | `s3` · `supabase` · `external`. Defaults to `supabase` with a Supabase DB, else `s3` on AWS, else `external`. |
| `--storage-endpoint` / `--storage-bucket` / `--storage-region` / `--storage-access-key` / `--storage-secret` / `--storage-path-style` | prompt | External S3-compatible store (MinIO/R2/…). |
| `--auth` | derived | `supabase` · `token` · `oidc`. Defaults to `supabase` with a Supabase DB, else `token` (a minted admin token). |
| `--oidc-issuer` / `--oidc-client-id` | prompt | For `--auth oidc`. |

## Other flags

| Flag | Default | Purpose |
| ---- | ------- | ------- |
| `--name` | `prod` | Workspace name — run several deployments side by side. |
| `--tool` | `terraform` | `terraform` or `pulumi` (fargate/eks). |
| `--region` | prompt / `us-east-1` | AWS + Supabase region. |
| `--supabase-org` | interactive picker | Supabase organization id (when a Supabase backend is used). |
| `--domain` | prompt / none | Custom domain + ACM/HTTPS (fargate). |
| `--ingress-host` | prompt / none | Kubernetes ingress host (helm/eks). |
| `--kube-context` | current context | kubectl context to deploy into (helm). |
| `--admin-email` | prompt | Comma-separated emails granted the pensieve admin role (supabase/oidc). |
| `--target` | — | **Deprecated** alias for `--compute` (`aws`→`fargate`, `local`→`local`). |
| `--yes` | off | Non-interactive (supply the relevant flags). |
| `--print-only` | off | Render the artifact + print planned commands; runs nothing. |
| `--force` | off | Regenerate a workspace's rendered config. |

## Credential acquisition

When a Supabase backend is in use, `init` obtains a Management-API token in this
order: `SUPABASE_ACCESS_TOKEN` → your `supabase login` session
(`~/.supabase/access-token`) → browser OAuth (PKCE, when
`PENSIEVE_SUPABASE_OAUTH_CLIENT_ID` is set) → guided paste. AWS compute uses the
standard AWS credential chain (`aws configure`, SSO, env vars). Non-Supabase
deployments need neither — only what the chosen backends require.

## Workspaces

Everything lives in `~/.pensieve/deploy/<name>/` (rendered files are `0600` and hold
secrets — never commit them):

```
terraform/                   # materialized stack (fargate/eks)
terraform/terraform.tfvars   # rendered answers
helm/pensieve-engine/            # the engine Helm chart (helm/eks)
helm-values.yaml             # rendered Helm values (helm/eks)
local.env                    # docker env file (local)
answers.json                 # persisted axis answers (helm/eks/local)
deploy.json                  # compute/db/storage/auth + state
```

Re-running `init` refreshes the embedded templates but never overwrites your
`terraform.tfvars` (use `--force`). State stays in the workspace by default;
see [Terraform → State](./terraform#state) for the team setup.

## Per-target notes

- **fargate** — `terraform`/`pulumi apply`; ALB + (optional) ACM/Route53.
- **eks** — `terraform apply` provisions the cluster + IRSA, then `up` runs
  `aws eks update-kubeconfig` and `helm upgrade --install`, wiring the resolved
  catalog URL, created bucket, and IRSA role into the chart. See
  [Kubernetes / EKS](./kubernetes).
- **helm** — renders `helm-values.yaml` at `init`; `up` runs
  `helm upgrade --install pensieve … -n pensieve --create-namespace`. See [Helm](./helm).
- **local** — provisions a Supabase project (if used) or wires your external
  Postgres/storage, then `docker run`s the engine on `:8080`.

With `--storage supabase`, the wizard opens the dashboard's S3-keys page
(Supabase has no API to mint them) and waits for the paste — one manual step.
