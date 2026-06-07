---
title: Production deployment
description: Deploy kyma to AWS Fargate + S3 + Supabase in one command — kyma deploy, Terraform, or Pulumi.
---

# Production deployment

kyma self-hosts on a deliberately small production topology:

```
            ┌─────────────────────────── AWS ───────────────────────────┐
            │  ALB (HTTPS, /health checks)                              │
  users ───▶│   └─▶ ECS Fargate: kyma-engine (web UI + API, ARM64)      │
            │         ├─▶ S3 extent bucket   (task-role auth, no keys)  │
            │         └─▶ SSM parameters     (secrets at start-up)      │
            └───────────────┬───────────────────────────────────────────┘
                            │
                  Supabase  ▼
                  ├─ Postgres  → kyma catalog (KYMA_CATALOG_URL)
                  └─ Auth      → sign-in (JWTs verified via project JWKS)
```

- **One engine container** on ECS Fargate serves the web UI and every API.
- **S3** holds the columnar extents — keyless, via the Fargate task role.
- **Supabase** provides the catalog Postgres and user sign-in
  (email/password + OAuth providers, JIT-provisioned into kyma with roles
  from `KYMA_ADMIN_EMAILS`).
- Cost floor ≈ ALB ~$16/mo + one 0.5 vCPU/1 GB ARM task ~$15/mo + S3 +
  your Supabase tier. No NAT gateway.

## One command

```sh
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash -s -- --prod-deploy
```

or with the CLI already installed:

```sh
kyma deploy init    # wizard — credentials, region, domain, admin emails
kyma deploy up      # applies the IaC, prints your engine URL
kyma deploy status  # outputs + live health probe
```

The wizard acquires credentials on your behalf where possible: it reuses
`SUPABASE_ACCESS_TOKEN` or your `supabase login` session (or walks you
through creating a token), probes the AWS credential chain, lists your
Supabase organizations to pick from, and pins the engine image to the
release matching your CLI. Preview everything first with
`kyma deploy init --print-only`.

## Local test drive (no AWS)

Validate the Supabase wiring before creating any AWS resources:

```sh
kyma deploy init --target local   # provisions ONLY a Supabase project
kyma deploy up                    # runs the engine container locally (docker)
```

You get http://localhost:8080 with real Supabase sign-in and the catalog on
Supabase Postgres; extents live on a docker volume.

## Choose your path

| Path | When |
| ---- | ---- |
| [`kyma deploy` CLI](./cli) | You want the wizard to drive everything. |
| [Terraform](./terraform) | You manage IaC yourself or need to customize the stack. |
| [Pulumi](./pulumi) | Your infra is Pulumi — same stack via the terraform-module bridge. |

## After deploying

1. Open the engine URL and sign in with Supabase (admin emails get the
   admin role on first sign-in).
2. Mint an API token under **Settings → API tokens**, then connect the CLI:
   `kyma connect <engine-url> --token <api-token>`.
3. Wire your coding agents: `kyma setup claude-code` (and friends) — see
   the [quickstart](/quickstart/).

A fallback `admin` user (password in the deploy output and SSM) exists for
break-glass; primary auth is Supabase.
