# C0 — Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Infrastructure-as-Code foundation for Kyma Cloud — a bootstrapped AWS ops account holding remote Terraform state, GitHub OIDC so CI authenticates with no long-lived keys, pinned providers for AWS + Railway + Supabase + Stripe, a skeleton of reusable modules, a `dev` environment that composes them, and a CI pipeline that plans on PR and applies on merge. Closes every open decision from §7 of the master design.

**Architecture:** Terraform (OpenTofu-compatible) with an S3 + DynamoDB-lock remote backend living in a dedicated bootstrap AWS account. The bootstrap layer is applied once, by hand, with local state, then migrated to the remote backend it just created (the standard chicken-and-egg bootstrap). Everything after authenticates via GitHub OIDC. Modules are thin and single-purpose; environments compose them. "Tests" for infra are `terraform fmt -check`, `terraform validate`, `tflint`, and a successful `terraform plan` — these are the RED/GREEN gates.

**Tech Stack:** Terraform ≥ 1.9 (OpenTofu ≥ 1.7 compatible). Providers: `hashicorp/aws`, `terraform-community-providers/railway` (confirm in Task 4), `supabase/supabase`, `stripe/stripe`. GitHub Actions for CI. tflint + tfsec for policy. No application code in this phase.

**File Structure (created across this plan):**

```
infra/
  bootstrap/                 state backend + GitHub OIDC (local state, applied once)
    main.tf  variables.tf  outputs.tf  versions.tf  README.md
  modules/
    s3-extent-bucket/        main.tf variables.tf outputs.tf README.md
    iam-scoped-role/         main.tf variables.tf outputs.tf README.md
    railway-service/         main.tf variables.tf outputs.tf README.md
    supabase-project/        main.tf variables.tf outputs.tf README.md
    stripe-product/          main.tf variables.tf outputs.tf README.md
  envs/
    dev/                     main.tf variables.tf outputs.tf backend.tf versions.tf terraform.tfvars.example
  README.md                  conventions: regions, naming, workspaces, how to apply
docs/cloud/
  decisions.md               resolves §7 of the master design (the ADR for Cloud)
.github/workflows/
  infra-plan.yml             plan on PR
  infra-apply.yml            apply on merge to main (per-env)
.tflint.hcl                  lint config
```

**Conventions locked here (used by all later phases):**
- Region: single AWS region for everything — `us-east-1` (override per §7; confirm Supabase availability in Task 7).
- Naming: `kyma-<env>-<purpose>` (e.g. `kyma-dev-extents`, `kyma-prod-extents`). Tenants add `<tenant_id>` as an S3 *prefix*, never a new bucket (pooled).
- State key per env/component: `env/<env>/<component>.tfstate`.
- Tags on every AWS resource: `Project=kyma-cloud`, `Env=<env>`, `ManagedBy=terraform`.

---

## Task 1: Create the IaC conventions README

**Objective:** Write down the rules every later task and phase follows, so no one guesses.

**Files:**
- Create: `infra/README.md`

- [ ] **Step 1: Write `infra/README.md`**

````markdown
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
````

- [ ] **Step 2: Verify it renders and commit**

Run: `test -f infra/README.md && echo OK`
Expected: `OK`

```bash
git add infra/README.md
git commit -m "docs(cloud): add infra conventions readme"
```

---

## Task 2: Write the Cloud decisions record (resolves §7)

**Objective:** Turn every master-design recommendation into a locked decision with rationale, so later phases don't reopen them.

**Files:**
- Create: `docs/cloud/decisions.md`

- [ ] **Step 1: Write `docs/cloud/decisions.md`**

````markdown
# Kyma Cloud — Decisions Record

Locks §7 of `2026-05-25-kyma-cloud-platform-design.md`. One row per decision;
change requires a new dated entry, not an edit.

## 2026-05-25 — initial locks

| # | Decision | Choice | Rationale |
|---|---|---|---|
| 1 | IaC tool | Terraform 1.9+ / OpenTofu 1.7+ | Broadest provider coverage; mature OIDC + remote state |
| 2 | Tenancy at launch | Pooled, with silo path | Density now, isolation later, cheap migration |
| 3 | Pooled isolation | Scoped STS session creds + `X-Database` + gateway quotas | Strongest practical isolation without per-tenant processes |
| 4 | Supabase split | Two projects: A control-plane, B catalog | Independent blast radius / backups / roles |
| 5 | Control-plane lang | TypeScript (Node) | Native to Supabase/Next.js/Stripe; CRUD not hot-path |
| 6 | Gateway lang | Rust, reuse `kyma-server` crates | Hot path; keep OSS engine unmodified |
| 7 | Region | `us-east-1` for AWS + Supabase + Railway | Kill cross-cloud hot-path latency |
| 8 | Provisioning split | Pooled = app logic; silo + envs = Terraform | Never terraform-apply per signup |
| 9 | Billing meters | Ingest bytes/rows · storage GB-mo · query bytes-scanned/Flight-time | Maps to real S3 + compute cost; engine already emits these |
| 10 | Secrets | GH OIDC→AWS; tokens in Secrets Manager/Doppler; none in state | Supply-chain hygiene |

## Open items to confirm during C0 (close before C0 done)
- [ ] Railway Terraform provider maturity (Task 4) — else fall back to `railway.json` config-as-code + a thin `null_resource`/CLI wrapper.
- [ ] Supabase region parity with `us-east-1` (Task 7).
- [ ] Secrets manager choice: AWS Secrets Manager vs Doppler (Task 9).
````

- [ ] **Step 2: Commit**

```bash
git add docs/cloud/decisions.md
git commit -m "docs(cloud): record locked decisions (master-design section 7)"
```

---

## Task 3: Bootstrap — versions + provider pins

**Objective:** Pin Terraform and the AWS provider for the bootstrap layer (the only layer that touches AWS at bootstrap time).

**Files:**
- Create: `infra/bootstrap/versions.tf`

- [ ] **Step 1: Write `infra/bootstrap/versions.tf`**

```hcl
terraform {
  required_version = ">= 1.9.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.60"
    }
  }
  # NOTE: backend is intentionally absent. Bootstrap starts on LOCAL state,
  # then migrates to the S3 backend it creates (see README). Do not add a
  # backend block until Step in Task 6.
}

provider "aws" {
  region = var.aws_region
  default_tags {
    tags = {
      Project   = "kyma-cloud"
      Env       = "bootstrap"
      ManagedBy = "terraform"
    }
  }
}
```

- [ ] **Step 2: Verify Terraform parses it**

Run: `cd infra/bootstrap && terraform fmt -check && echo FMT_OK`
Expected: `FMT_OK` (no diff). If `terraform init` is needed first, that's fine — provider download happens in Task 6.

- [ ] **Step 3: Commit**

```bash
git add infra/bootstrap/versions.tf
git commit -m "infra(bootstrap): pin terraform + aws provider"
```

---

## Task 4: Confirm Railway / Supabase / Stripe providers, pin them

**Objective:** Resolve the open §7 item — which providers are real, and pin exact versions. This gates the module tasks.

**Files:**
- Modify: `docs/cloud/decisions.md` (check the Railway open item)
- Create: `infra/modules/_providers.md` (shared note of pinned provider sources)

- [ ] **Step 1: Research current provider sources/versions**

Run (record findings, do not guess):
```bash
echo "Check registry pages:"
echo "  https://registry.terraform.io/providers/supabase/supabase/latest"
echo "  https://registry.terraform.io/search?q=railway"
echo "  https://registry.terraform.io/providers/lukasaron/stripe (or stripe/stripe)"
```
Decision rule: if the Railway provider is unmaintained/thin, fall back to Railway's config-as-code (`railway.json` / `railway.toml`) invoked via a `null_resource` + the Railway CLI, and note it. Supabase + Stripe providers are expected to be usable.

- [ ] **Step 2: Write `infra/modules/_providers.md` with the pinned sources you confirmed**

````markdown
# Pinned provider sources (confirmed C0 Task 4)

| Provider | Source | Version | Notes |
|---|---|---|---|
| AWS | hashicorp/aws | ~> 5.60 | first-class |
| Supabase | supabase/supabase | <PIN> | projects, settings |
| Stripe | <PIN source> | <PIN> | products, prices, webhooks |
| Railway | <PIN or "CLI fallback"> | <PIN> | if provider thin → railway.json + CLI null_resource |

If a provider is the CLI fallback, the module wraps the Railway CLI in a
`null_resource` with a documented `triggers` map; state tracks only the
service id returned.
````

- [ ] **Step 3: Update the decisions open item**

In `docs/cloud/decisions.md`, change `- [ ] Railway Terraform provider maturity` to `- [x]` with a one-line conclusion.

- [ ] **Step 4: Commit**

```bash
git add infra/modules/_providers.md docs/cloud/decisions.md
git commit -m "infra: confirm and pin railway/supabase/stripe providers"
```

---

## Task 5: Bootstrap — state backend + OIDC resources

**Objective:** Define (not yet apply) the S3 state bucket, DynamoDB lock table, GitHub OIDC provider, and the CI role with a least-privilege-ish policy.

**Files:**
- Create: `infra/bootstrap/variables.tf`
- Create: `infra/bootstrap/main.tf`
- Create: `infra/bootstrap/outputs.tf`

- [ ] **Step 1: Write `infra/bootstrap/variables.tf`**

```hcl
variable "aws_region" {
  type    = string
  default = "us-east-1"
}

variable "state_bucket_name" {
  type        = string
  description = "Globally-unique S3 bucket for Terraform remote state."
  default     = "kyma-cloud-tfstate"
}

variable "lock_table_name" {
  type    = string
  default = "kyma-cloud-tflock"
}

variable "github_org" {
  type        = string
  description = "GitHub org/owner that hosts the kyma repo."
}

variable "github_repo" {
  type    = string
  default = "kyma"
}
```

- [ ] **Step 2: Write `infra/bootstrap/main.tf`**

```hcl
# --- Remote state backend resources ---
resource "aws_s3_bucket" "state" {
  bucket = var.state_bucket_name
}

resource "aws_s3_bucket_versioning" "state" {
  bucket = aws_s3_bucket.state.id
  versioning_configuration { status = "Enabled" }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "state" {
  bucket = aws_s3_bucket.state.id
  rule {
    apply_server_side_encryption_by_default { sse_algorithm = "aws:kms" }
  }
}

resource "aws_s3_bucket_public_access_block" "state" {
  bucket                  = aws_s3_bucket.state.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_dynamodb_table" "lock" {
  name         = var.lock_table_name
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "LockID"
  attribute {
    name = "LockID"
    type = "S"
  }
}

# --- GitHub OIDC ---
data "tls_certificate" "github" {
  url = "https://token.actions.githubusercontent.com/.well-known/openid-configuration"
}

resource "aws_iam_openid_connect_provider" "github" {
  url             = "https://token.actions.githubusercontent.com"
  client_id_list  = ["sts.amazonaws.com"]
  thumbprint_list = [data.tls_certificate.github.certificates[0].sha1_fingerprint]
}

data "aws_iam_policy_document" "ci_assume" {
  statement {
    actions = ["sts:AssumeRoleWithWebIdentity"]
    principals {
      type        = "Federated"
      identifiers = [aws_iam_openid_connect_provider.github.arn]
    }
    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:aud"
      values   = ["sts.amazonaws.com"]
    }
    condition {
      test     = "StringLike"
      variable = "token.actions.githubusercontent.com:sub"
      values   = ["repo:${var.github_org}/${var.github_repo}:*"]
    }
  }
}

resource "aws_iam_role" "ci" {
  name               = "kyma-cloud-ci"
  assume_role_policy = data.aws_iam_policy_document.ci_assume.json
}

# Start broad-but-bounded; tighten in C2 once the resource set is known.
resource "aws_iam_role_policy_attachment" "ci_poweruser" {
  role       = aws_iam_role.ci.name
  policy_arn = "arn:aws:iam::aws:policy/PowerUserAccess"
}
```

Add `tls` to `required_providers` in `versions.tf`:
```hcl
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
```

- [ ] **Step 3: Write `infra/bootstrap/outputs.tf`**

```hcl
output "state_bucket" { value = aws_s3_bucket.state.id }
output "lock_table"   { value = aws_dynamodb_table.lock.name }
output "ci_role_arn"  { value = aws_iam_role.ci.arn }
```

- [ ] **Step 4: Validate (RED→GREEN gate)**

Run:
```bash
cd infra/bootstrap
terraform init -backend=false
terraform fmt -check
terraform validate
```
Expected: `Success! The configuration is valid.`

- [ ] **Step 5: Commit**

```bash
git add infra/bootstrap/
git commit -m "infra(bootstrap): state backend + github oidc ci role"
```

---

## Task 6: Apply bootstrap by hand, migrate to remote state

**Objective:** The one manual apply. Creates the backend, then moves bootstrap's own state into it.

**Files:**
- Create: `infra/bootstrap/backend.tf` (added AFTER first apply)
- Create: `infra/bootstrap/README.md`

- [ ] **Step 1: Write `infra/bootstrap/README.md` (the runbook)**

````markdown
# Bootstrap (run once, by hand)

Requires AWS admin creds for the ops account exported in your shell.

```bash
cd infra/bootstrap
terraform init -backend=false
terraform apply -var github_org=<ORG>     # creates state bucket, lock, OIDC, CI role
```

Then migrate this layer's state into the bucket it just created:

1. Create `backend.tf` (see repo).
2. `terraform init -migrate-state` and answer "yes".
3. Commit `backend.tf`.

Outputs (`state_bucket`, `lock_table`, `ci_role_arn`) feed `envs/*` backends
and the CI workflows.
````

- [ ] **Step 2: Operator runs the apply** (human-in-the-loop; not CI)

Run:
```bash
cd infra/bootstrap
terraform init -backend=false
terraform apply -var github_org=<ORG>
```
Expected: bucket, table, OIDC provider, role created. Record outputs.

- [ ] **Step 3: Write `infra/bootstrap/backend.tf` and migrate**

```hcl
terraform {
  backend "s3" {
    bucket         = "kyma-cloud-tfstate"
    key            = "bootstrap/bootstrap.tfstate"
    region         = "us-east-1"
    dynamodb_table = "kyma-cloud-tflock"
    encrypt        = true
  }
}
```
Run: `terraform init -migrate-state` → answer `yes`.
Expected: "Successfully configured the backend s3!"

- [ ] **Step 4: Commit**

```bash
git add infra/bootstrap/backend.tf infra/bootstrap/README.md
git commit -m "infra(bootstrap): migrate to remote s3 state backend"
```

---

## Task 7: Module — `s3-extent-bucket`

**Objective:** Reusable module for the extent bucket (used by `dev` now, by silo tenants in C6). Versioning + SSE + lifecycle + public-access-block.

**Files:**
- Create: `infra/modules/s3-extent-bucket/{main,variables,outputs}.tf`
- Create: `infra/modules/s3-extent-bucket/README.md`

- [ ] **Step 1: Write `variables.tf`**

```hcl
variable "bucket_name" { type = string }
variable "env"         { type = string }
variable "noncurrent_expiration_days" {
  type    = number
  default = 30
}
```

- [ ] **Step 2: Write `main.tf`**

```hcl
resource "aws_s3_bucket" "this" {
  bucket = var.bucket_name
  tags   = { Project = "kyma-cloud", Env = var.env, ManagedBy = "terraform" }
}

resource "aws_s3_bucket_versioning" "this" {
  bucket = aws_s3_bucket.this.id
  versioning_configuration { status = "Enabled" }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "this" {
  bucket = aws_s3_bucket.this.id
  rule {
    apply_server_side_encryption_by_default { sse_algorithm = "aws:kms" }
  }
}

resource "aws_s3_bucket_public_access_block" "this" {
  bucket                  = aws_s3_bucket.this.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "this" {
  bucket = aws_s3_bucket.this.id
  rule {
    id     = "expire-noncurrent"
    status = "Enabled"
    noncurrent_version_expiration { noncurrent_days = var.noncurrent_expiration_days }
  }
}
```

- [ ] **Step 3: Write `outputs.tf`**

```hcl
output "bucket_name" { value = aws_s3_bucket.this.id }
output "bucket_arn"  { value = aws_s3_bucket.this.arn }
```

- [ ] **Step 4: Write a one-paragraph `README.md`** describing inputs/outputs and that pooled tenants are prefixes within this bucket.

- [ ] **Step 5: Validate**

Run: `cd infra/modules/s3-extent-bucket && terraform init -backend=false && terraform validate`
Expected: `Success! The configuration is valid.`

- [ ] **Step 6: Commit**

```bash
git add infra/modules/s3-extent-bucket/
git commit -m "infra(modules): s3-extent-bucket"
```

---

## Task 8: Module — `iam-scoped-role`

**Objective:** The role the gateway assumes, plus the **session-policy template** that scopes a tenant's STS session to its own prefix (the core of pooled isolation, consumed in C2).

**Files:**
- Create: `infra/modules/iam-scoped-role/{main,variables,outputs}.tf`
- Create: `infra/modules/iam-scoped-role/README.md`

- [ ] **Step 1: Write `variables.tf`**

```hcl
variable "env"         { type = string }
variable "bucket_arn"  { type = string }
variable "gateway_principal_arn" {
  type        = string
  description = "ARN of the gateway's task/role allowed to assume this role."
}
```

- [ ] **Step 2: Write `main.tf`** (broad role; gateway narrows per-request via session policy)

```hcl
data "aws_iam_policy_document" "assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "AWS"
      identifiers = [var.gateway_principal_arn]
    }
  }
}

resource "aws_iam_role" "tenant_data" {
  name               = "kyma-${var.env}-tenant-data"
  assume_role_policy = data.aws_iam_policy_document.assume.json
}

# Broad bucket access on the ROLE; the gateway passes a SESSION POLICY at
# AssumeRole time that restricts s3:prefix to s3://<bucket>/<tenant_id>/*.
data "aws_iam_policy_document" "bucket_rw" {
  statement {
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${var.bucket_arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [var.bucket_arn]
  }
}

resource "aws_iam_role_policy" "bucket_rw" {
  role   = aws_iam_role.tenant_data.id
  policy = data.aws_iam_policy_document.bucket_rw.json
}
```

- [ ] **Step 3: Write `outputs.tf`**

```hcl
output "role_arn" { value = aws_iam_role.tenant_data.arn }
```

- [ ] **Step 4: Write `README.md`** — document the session-policy pattern with an example the gateway will use:

````markdown
The gateway calls `AssumeRole(RoleArn=role_arn, Policy=<session policy>)` where
the session policy pins the prefix:

```json
{ "Version": "2012-10-17", "Statement": [
  { "Effect": "Allow", "Action": ["s3:GetObject","s3:PutObject","s3:DeleteObject"],
    "Resource": "arn:aws:s3:::kyma-prod-extents/${tenant_id}/*" },
  { "Effect": "Allow", "Action": "s3:ListBucket",
    "Resource": "arn:aws:s3:::kyma-prod-extents",
    "Condition": { "StringLike": { "s3:prefix": "${tenant_id}/*" } } } ]}
```
The effective permission is the INTERSECTION of role policy and session policy,
so tenant A's session can never touch tenant B's prefix.
````

- [ ] **Step 5: Validate + commit**

Run: `cd infra/modules/iam-scoped-role && terraform init -backend=false && terraform validate`
Expected: valid.
```bash
git add infra/modules/iam-scoped-role/
git commit -m "infra(modules): iam-scoped-role with per-tenant session policy"
```

---

## Task 9: Module stubs — `supabase-project`, `stripe-product`, `railway-service`

**Objective:** Land thin, valid module skeletons so `envs/dev` can reference them. Full bodies filled where the provider is confirmed (Task 4); otherwise documented stub + CLI fallback.

**Files:**
- Create: `infra/modules/supabase-project/{main,variables,outputs}.tf` + README
- Create: `infra/modules/stripe-product/{main,variables,outputs}.tf` + README
- Create: `infra/modules/railway-service/{main,variables,outputs}.tf` + README

- [ ] **Step 1: `supabase-project`** — variables: `name`, `org_id`, `region`, `db_pass` (from env/secret, marked `sensitive`); resource: `supabase_project`; output: `project_ref`, `database_url` (sensitive). Pin provider per Task 4.

- [ ] **Step 2: `stripe-product`** — variables: `name`, `unit_label`, `metered_prices` (list of {nickname, unit_amount_decimal, meter}); resources: `stripe_product` + `stripe_price` per tier; outputs: price IDs map.

- [ ] **Step 3: `railway-service`** — variables: `project_id`, `name`, `image` or `repo`, `env_vars` (map, sensitive), `region`, `replicas`; if provider confirmed, `railway_service` + vars; else `null_resource` wrapping `railway up`/`railway variables` with a documented `triggers` map; output: `service_id`, `public_domain`.

- [ ] **Step 4: Validate each module**

Run:
```bash
for m in supabase-project stripe-product railway-service; do
  (cd infra/modules/$m && terraform init -backend=false && terraform validate) || exit 1
done
echo ALL_VALID
```
Expected: `ALL_VALID`

- [ ] **Step 5: Resolve the secrets-manager open item** in `docs/cloud/decisions.md` (AWS Secrets Manager vs Doppler) — pick one, check the box, one-line rationale.

- [ ] **Step 6: Commit**

```bash
git add infra/modules/ docs/cloud/decisions.md
git commit -m "infra(modules): supabase/stripe/railway service module skeletons"
```

---

## Task 10: Environment — `envs/dev`

**Objective:** Compose the modules into a real `dev` environment definition that `terraform plan` accepts. (Apply happens via CI in Task 12 / consumed by C1.)

**Files:**
- Create: `infra/envs/dev/{versions,backend,variables,main,outputs}.tf`
- Create: `infra/envs/dev/terraform.tfvars.example`

- [ ] **Step 1: `versions.tf`** — required_version + all four providers pinned (aws, supabase, stripe, railway-or-fallback).

- [ ] **Step 2: `backend.tf`**

```hcl
terraform {
  backend "s3" {
    bucket         = "kyma-cloud-tfstate"
    key            = "env/dev/main.tfstate"
    region         = "us-east-1"
    dynamodb_table = "kyma-cloud-tflock"
    encrypt        = true
  }
}
```

- [ ] **Step 3: `main.tf`** — instantiate:
  - `module "extents"` → `s3-extent-bucket` (`bucket_name = "kyma-dev-extents"`)
  - `module "catalog"` → `supabase-project` (Supabase B — engine catalog)
  - `module "control_plane_db"` → `supabase-project` (Supabase A — control plane)
  - `module "tenant_role"` → `iam-scoped-role` (bucket_arn from extents)
  - `module "billing"` → `stripe-product` (Free/Pro/Enterprise metered prices)
  - Railway services (`engine`, `gateway`, `api`, `dashboard`) deferred to C1/C2 — leave commented placeholders referencing `railway-service` with a `# TODO(C1)` note so the seam is visible.

- [ ] **Step 4: `variables.tf` + `terraform.tfvars.example`** — non-secret values only; secrets via `TF_VAR_*` env in CI.

- [ ] **Step 5: Validate + plan-shape check**

Run:
```bash
cd infra/envs/dev
terraform init -backend=false
terraform validate
```
Expected: `Success! The configuration is valid.` (Full `plan` needs creds; that's Task 12 in CI.)

- [ ] **Step 6: Commit**

```bash
git add infra/envs/dev/
git commit -m "infra(envs): dev environment composing core modules"
```

---

## Task 11: Lint + policy config

**Objective:** Add `tflint` + `tfsec` so CI has real RED/GREEN gates beyond `validate`.

**Files:**
- Create: `.tflint.hcl`

- [ ] **Step 1: Write `.tflint.hcl`**

```hcl
plugin "terraform" {
  enabled = true
  preset  = "recommended"
}
plugin "aws" {
  enabled = true
  version = "0.31.0"
  source  = "github.com/terraform-linters/tflint-ruleset-aws"
}
```

- [ ] **Step 2: Run locally**

Run:
```bash
tflint --init && tflint --recursive
tfsec infra/ || true   # review findings; suppress with justification only
```
Expected: tflint passes (or only documented suppressions).

- [ ] **Step 3: Commit**

```bash
git add .tflint.hcl
git commit -m "infra: tflint config"
```

---

## Task 12: CI — plan on PR, apply on merge

**Objective:** Wire GitHub OIDC so CI plans every infra PR and applies on merge to `main`, with no long-lived AWS keys.

**Files:**
- Create: `.github/workflows/infra-plan.yml`
- Create: `.github/workflows/infra-apply.yml`

- [ ] **Step 1: Write `infra-plan.yml`**

```yaml
name: infra-plan
on:
  pull_request:
    paths: ["infra/**", ".tflint.hcl", ".github/workflows/infra-*.yml"]
permissions:
  id-token: write
  contents: read
  pull-requests: write
jobs:
  plan:
    runs-on: ubuntu-22.04
    strategy:
      matrix: { env: [dev] }
    steps:
      - uses: actions/checkout@v4
      - uses: hashicorp/setup-terraform@v3
        with: { terraform_version: "1.9.8" }
      - uses: aws-actions/configure-aws-credentials@v4
        with:
          role-to-assume: ${{ secrets.AWS_CI_ROLE_ARN }}
          aws-region: us-east-1
      - name: fmt
        run: terraform -chdir=infra/envs/${{ matrix.env }} fmt -check -recursive
      - name: init
        run: terraform -chdir=infra/envs/${{ matrix.env }} init
      - name: validate
        run: terraform -chdir=infra/envs/${{ matrix.env }} validate
      - name: plan
        env:
          TF_VAR_supabase_db_pass: ${{ secrets.SUPABASE_DB_PASS }}
          SUPABASE_ACCESS_TOKEN: ${{ secrets.SUPABASE_ACCESS_TOKEN }}
          STRIPE_API_KEY: ${{ secrets.STRIPE_API_KEY }}
          RAILWAY_TOKEN: ${{ secrets.RAILWAY_TOKEN }}
        run: terraform -chdir=infra/envs/${{ matrix.env }} plan -no-color
```

- [ ] **Step 2: Write `infra-apply.yml`** — same auth, trigger `on: push: branches: [main]` with the same `paths`, replace the plan step with `terraform … apply -auto-approve`. Add an `environment: dev` GitHub environment so a required reviewer can gate prod later.

- [ ] **Step 3: Add the required GitHub secrets** (operator step, documented in `infra/README.md`): `AWS_CI_ROLE_ARN` (from bootstrap output), `SUPABASE_ACCESS_TOKEN`, `SUPABASE_DB_PASS`, `STRIPE_API_KEY`, `RAILWAY_TOKEN`.

- [ ] **Step 4: Verify the workflow triggers**

Open a draft PR touching `infra/envs/dev/` and confirm `infra-plan` runs, assumes the role, and posts a plan. Expected: green plan job, no credential errors.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/infra-plan.yml .github/workflows/infra-apply.yml
git commit -m "ci(infra): plan on PR, apply on merge via github oidc"
```

---

## Task 13: Phase exit checklist

**Objective:** Prove C0 is done and C1 is unblocked.

- [ ] All §7 decisions locked in `docs/cloud/decisions.md`; all three open items checked.
- [ ] `terraform validate` passes in `bootstrap`, all `modules/*`, and `envs/dev`.
- [ ] Bootstrap applied; remote state + lock table + OIDC role exist; `ci_role_arn` recorded as a GH secret.
- [ ] `infra-plan` runs green on a PR via OIDC (no static AWS keys anywhere).
- [ ] `tflint --recursive` clean (or documented suppressions).
- [ ] `infra/README.md` lets a new engineer apply `dev` without tribal knowledge.

- [ ] **Commit the checklist result** into the master design as a status line:

In `docs/superpowers/specs/2026-05-25-kyma-cloud-platform-design.md`, under the C0 appendix stub, add:
`**Status:** ✅ C0 complete — see docs/superpowers/plans/2026-05-25-c0-foundations.md`

```bash
git add docs/superpowers/specs/2026-05-25-kyma-cloud-platform-design.md
git commit -m "docs(cloud): mark C0 foundations complete"
```

---

## Notes for the implementer

- **Bootstrap is the only manual apply.** Everything else goes through CI. If you find yourself running `apply` locally for anything but `bootstrap/`, stop.
- **Never put a secret in `.tf`, `.tfvars`, or state.** Secrets arrive as `TF_VAR_*` / provider env vars in CI only.
- **Pooled tenants are not in this plan and never will be.** This phase builds the *shared* substrate. Tenant creation is C2 app logic. The only per-tenant TF is the silo module in C6.
- **Match the engine's env conventions** (`KYMA_S3_BUCKET`, `KYMA_S3_REGION`, `KYMA_CATALOG_URL`, `KYMA_S3_PATH_STYLE`) when C1 wires the engine container — they're already used in `.github/workflows/gauntlet-pr.yml`.
- **Keep modules dumb.** No environment-specific values inside `modules/`. If a module needs to know it's "dev", that's a variable.
