# Deploying kyma to production

Self-hosted production deployment of the kyma engine: **AWS ECS Fargate**
(engine container) + **AWS S3** (columnar extents) + **Supabase** (catalog
Postgres + Auth). One `terraform apply` — or one `kyma deploy` — and you get
an HTTPS endpoint running the full engine with the web UI, Supabase login,
and keyless IAM-role S3 access.

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
                  └─ Auth      → login (JWTs verified via project JWKS)
```

## The easy way: `kyma deploy`

```sh
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash -s -- --prod-deploy
# or, with the CLI already installed:
kyma deploy init   # wizard: credentials, region, domain, tool
kyma deploy up     # terraform/pulumi apply + `kyma connect` to the result
kyma deploy status # outputs + live /health probe
kyma deploy destroy
```

The wizard collects AWS + Supabase credentials (reusing `SUPABASE_ACCESS_TOKEN`,
an existing `supabase login`, or a browser OAuth flow where available),
renders `terraform.tfvars`, and runs the IaC for you.

## The manual way: Terraform

Prereqs: Terraform ≥ 1.9, AWS credentials in the standard chain, and a
Supabase access token:

```sh
export SUPABASE_ACCESS_TOKEN=sbp_…   # https://supabase.com/dashboard/account/tokens
cd deploy/terraform
cp terraform.tfvars.example terraform.tfvars   # then edit
export TF_VAR_supabase_db_password="$(openssl rand -base64 24)"
terraform init
terraform apply
terraform output engine_url
```

Then `kyma connect "$(terraform output -raw engine_url)" --token <api-token>`
(mint an API token under Settings → API tokens after signing in).

### Domain & TLS

Set `domain` (+ `route53_zone_id` when the zone lives in Route53 — fully
automated). Without a zone id, `apply` prints the ACM validation CNAME and
waits while you create it at your DNS provider. With no domain at all, the
stack exposes plain HTTP on the ALB DNS name — fine for a test drive,
not for production (OAuth redirects want HTTPS).

### Extents storage

`storage_backend = "supabase"` (default) keeps the columnar extents in
**Supabase Storage** via its S3-compatible endpoint — everything stateful
lives in one Supabase project. Supabase has no API to mint S3 access keys,
so that path has one manual step (dashboard → Storage → S3 access keys);
`kyma deploy up` opens the page and waits for the paste. Set
`storage_backend = "s3"` for a native AWS bucket — fully automated and
keyless (Fargate task role), lowest latency from the engine.

### Sign-in policy

- `admin_emails` — Supabase-authenticated emails that get the kyma admin role.
- `allowed_email_domains` — **set this** unless you've disabled public signup
  in your Supabase project; anyone who can register otherwise gets read access.
- `oauth_providers` — login-page buttons; enable the same providers in
  Supabase → Authentication → Providers.

### State

Local state by default. For teams, uncomment the S3 backend in
`backend.tf` and `terraform init -migrate-state`.

### Cost floor

ALB (~$16/mo) + 1× Fargate task 0.5 vCPU/1GB ARM64 (~$15/mo) + S3 + Supabase
free/Pro tier. No NAT gateway (public subnets + task IP).

## Pulumi

The Terraform module is the single source of truth; Pulumi users consume it
via the official terraform-module bridge — see [`pulumi/`](./pulumi/).

## Notes

- gRPC (Arrow Flight) and OTLP-gRPC are disabled in this stack (ALB is
  HTTP-only); OTLP/HTTP and the REST ingest API work through the ALB.
- The engine self-migrates the catalog schema on first connect — no
  bootstrap job needed.
- Engine image: `ghcr.io/shakedaskayo/kyma-engine` (multi-arch on release
  tags). Pin `image_tag` to a release.
