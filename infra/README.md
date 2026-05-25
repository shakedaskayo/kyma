# Kyma Cloud — Infrastructure

Terraform (OpenTofu-compatible) for Kyma Cloud. See
`docs/superpowers/specs/2026-05-25-kyma-cloud-platform-design.md` for the
architecture and `docs/cloud/decisions.md` for locked decisions.

## Layout
- `bootstrap/` — applied ONCE by hand. Creates the S3+DynamoDB remote-state
  backend and the GitHub OIDC provider/roles. Starts on local state, then
  migrates to the backend it created.
- `modules/` — thin, single-purpose, reusable. No environment values inside.
- `envs/<env>/` — composes modules into a full environment (`dev`/`staging`/`prod`).

## Conventions
- **Region:** `us-east-1` for AWS + Supabase + (logical) Railway region. One metro.
- **Naming:** `kyma-<env>-<purpose>`. Pooled tenants are an S3 *prefix*
  (`s3://kyma-<env>-extents/<tenant_id>/`), never a new bucket.
- **State keys:** `env/<env>/<component>.tfstate` in the bootstrap state bucket.
- **Tags:** every AWS resource carries `Project=kyma-cloud`, `Env=<env>`,
  `ManagedBy=terraform`.
- **Secrets:** never in state or tfvars. CI uses GitHub OIDC for AWS; Railway/
  Supabase/Stripe tokens come from GH Actions secrets.
- **Pooled vs silo:** pooled tenants are provisioned by app logic at runtime —
  NEVER `terraform apply` on a signup. Only environments and silo tenants are TF.

## Apply order
1. `bootstrap/` once, by hand (see its README).
2. `envs/dev` via CI (`infra-apply.yml`) or locally after assuming the CI role.

## Local prerequisites
- terraform >= 1.9 (or opentofu >= 1.7), tflint, tfsec, awscli.

## CI secrets (operator-provisioned)
The infra workflows authenticate to AWS via GitHub OIDC and need these GitHub
Actions secrets set on the repo (no long-lived AWS keys anywhere):
- `AWS_CI_ROLE_ARN` — the `ci_role_arn` output from `bootstrap/`.
- `SUPABASE_ACCESS_TOKEN`, `SUPABASE_DB_PASS`, `SUPABASE_ORG_ID` — Supabase
  management API token, DB password, and owning org id.
- `STRIPE_API_KEY` — Stripe secret key (test mode for `dev`).
- `RAILWAY_TOKEN` — Railway account/project token (or CLI fallback token).
