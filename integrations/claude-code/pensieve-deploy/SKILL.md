---
name: pensieve-deploy
description: Deploy pensieve to production (AWS Fargate + S3 + Supabase) or run a Supabase-backed local test drive. Use when the user asks to deploy pensieve, self-host pensieve in production, set up pensieve on AWS/Supabase, or tear a pensieve deployment down. Drives the `pensieve deploy` CLI wizard (Terraform or Pulumi under the hood).
---

# Deploying pensieve to production

pensieve's production topology: **ECS Fargate** runs the engine container (web UI +
API, ARM64), **S3** holds the columnar extents (keyless IAM task-role auth),
**Supabase** provides the catalog Postgres and user sign-in (Supabase Auth —
email/password + OAuth providers). Terraform is the source of truth; Pulumi
wraps the same stack. Cost floor ≈ ALB ~$16/mo + 1 Fargate task ~$15/mo + S3 +
Supabase tier (no NAT gateway).

## The happy path

```sh
pensieve deploy init    # wizard: credentials, region, domain, admin emails
pensieve deploy up      # terraform init+apply (streams output), then prints the engine URL
pensieve deploy status  # outputs + live /health probe
```

`init` is interactive by default and acquires credentials on the user's
behalf where possible:

- **Supabase token**: `SUPABASE_ACCESS_TOKEN` env → reuse `supabase login`
  (`~/.supabase/access-token`) → browser OAuth (if `PENSIEVE_SUPABASE_OAUTH_CLIENT_ID`
  is configured) → guided paste from
  https://supabase.com/dashboard/account/tokens
- **AWS**: standard credential chain; on failure it tells the user to run
  `aws configure` or `aws sso login`.

Useful flags: `--name <ws>` (multiple deployments), `--tool pulumi`,
`--region`, `--domain pensieve.example.com`, `--admin-email a@b.com`,
`--yes` (non-interactive; requires `--supabase-org` + a token source),
`--print-only` (render config + show planned commands, run nothing — use
this to preview for the user before doing anything real).

## Local test drive (no AWS account)

```sh
pensieve deploy init --target local   # provisions ONLY a Supabase project,
                                  # writes ~/.pensieve/deploy/<name>/local.env
pensieve deploy up                    # docker run of the engine image with that env
```

The engine runs at http://localhost:8080 with the catalog + sign-in on the
real Supabase project — validates the whole Supabase wiring before paying
for AWS. Extents stay on a docker volume (S3 lines in `local.env` are the
commented upgrade path).

## After `up`

1. Open the engine URL → sign in with a Supabase account. Emails listed in
   `admin_emails` get the pensieve **admin** role on first sign-in (users are
   JIT-provisioned).
2. Mint an API token in the web UI (Settings → API tokens), then:
   `pensieve connect <engine-url> --token <api-token>` — the CLI/MCP/CI path.
3. A fallback `admin` user (password in the `up` output and in SSM) exists
   for break-glass; primary auth is Supabase.

## Files & layout

- Workspace: `~/.pensieve/deploy/<name>/` — materialized Terraform/Pulumi
  templates, rendered `terraform.tfvars` (0600, contains the DB password),
  `deploy.json` state, `local.env` for the local target.
- Repo copies of the IaC (for reading/auditing): `deploy/terraform/`
  (stack in `deploy/terraform/stack/`), `deploy/pulumi/`.

## Don'ts

- Don't commit `terraform.tfvars` or `local.env` — they hold secrets.
- Don't run `terraform apply` by hand in the repo's `deploy/terraform/`;
  use the workspace (`pensieve deploy up`) so state stays in one place.
- Don't leave `allowed_email_domains` empty on a Supabase project with
  public signup — anyone who registers would get read access. Set it (the
  wizard derives it from the admin email) or disable public signup in
  Supabase → Authentication.
- Don't expect gRPC (Arrow Flight) or OTLP-gRPC through the ALB — they're
  disabled in this stack; OTLP/HTTP and REST ingest work.

## Troubleshooting

- `up` fails creating the Supabase project → it can take minutes or flake;
  `pensieve deploy up` is idempotent — re-run it.
- ALB shows unhealthy targets → check the log group from
  `pensieve deploy status` (`aws logs tail <group> --follow`); first boot runs
  catalog migrations and can take ~a minute.
- ACM validation hangs (custom domain without Route53) → create the printed
  CNAME at the DNS provider; apply resumes automatically.
- 401s from the CLI → mint a fresh API token in Settings → API tokens.

## Teardown

```sh
pensieve deploy destroy           # terraform/pulumi destroy, or docker rm +
                              # Supabase project delete for the local target
```
