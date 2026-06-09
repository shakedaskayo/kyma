# Pluggable deploy backends: compute × database × storage × auth

**Date:** 2026-06-09
**Status:** Design — approved, pending spec review
**Branch (current):** `feat/platform-enrichment` (work to land on its own branch)
**Related:** `deploy/README.md`, `crates/kyma-cli/src/deploy.rs`, `deploy/terraform/stack/`, prior prod-deploy track (AWS Fargate + Supabase)

## 1. Summary

Make `kyma deploy` (wizard + IaC + docs) **backend-pluggable** along four
orthogonal axes the user selects interactively:

| Axis | Options |
|------|---------|
| **Compute** | `fargate` (default), `eks` (Terraform-provisioned), `helm` (any existing k8s cluster), `local` (docker) |
| **Database** (catalog Postgres) | `supabase` (provisioned), `rds` (provisioned), `external` (bring-your-own `postgresql://` URL) |
| **Storage** (columnar extents) | `s3` (native AWS, keyless), `supabase` (Supabase Storage), `external` (BYO S3-compatible: MinIO / R2 / GCS-interop) |
| **Auth** | `supabase` (when db=supabase), `token` (default for BYO/RDS), `oidc` (selectable) |

The key enabling fact: **the engine is already backend-agnostic.** No engine
code changes are required.

- Object store (`crates/kyma-storage/src/lib.rs`): `StorageConfig::S3Compatible`
  already serves native AWS S3, Supabase Storage, MinIO, R2 and GCS-interop via
  `endpoint` + `path_style` + optional keys, with keyless fallback to the AWS
  provider chain (task role / IRSA / IMDS). `Local`/`Memory` exist for dev/test.
- Catalog: generic Postgres via `KYMA_CATALOG_URL` — any provider, including a
  user-supplied URL. The engine self-migrates on first connect.
- Auth (`crates/kyma-server/src/auth/`): `supabase_backend`, `oidc_backend`,
  and static-token `env_backend` (`KYMA_AUTH_TOKENS=tok:role`) all exist;
  `KYMA_AUTH_BACKEND` selects.

All work is therefore confined to: the wizard (`deploy.rs`), the Terraform
`stack/`, a new Helm chart, and the deploy docs.

## 2. Goals / non-goals

**Goals**

1. Wizard interactively selects each axis, with bring-your-own DB URL and BYO
   S3-compatible storage as first-class paths.
2. Add `eks` and `helm` compute targets alongside `fargate` and `local`.
3. Add `rds` and `external` database backends alongside `supabase`.
4. Add `s3`, `supabase`, and `external` storage backends, asked interactively.
5. Non-Supabase deploys authenticate via a minted admin token (default) or OIDC.
6. Docs rewritten to a backend matrix: cli / terraform / pulumi / helm / kubernetes.
7. Preserve the provider-free Terraform `stack/` invariant required by the
   Pulumi `terraform-module` bridge.
8. Back-compat: existing `--target aws|local` flags and existing workspaces keep
   working.

**Non-goals**

- No engine code changes (backends already supported).
- No GKE/AKS-specific Terraform (the Helm chart + BYO-cluster path covers them;
  only EKS gets bespoke provisioning).
- No raw `kubectl`-apply manifest set (the Helm chart is the single k8s artifact;
  "vanilla" k8s = the Helm chart on a BYO cluster).
- No managed external-secrets integration (chart creates a k8s `Secret`;
  swapping to external-secrets is documented as a follow-up).
- gRPC / OTLP-gRPC over the ALB remains out (unchanged; needs an NLB).

## 3. Architecture: Approach C — orthogonal axes + compatibility matrix

The four axes are independent selectors. A single `validate(combo) -> Result<()>`
gate runs before any workspace is materialized, rejecting invalid corners with a
clear message and the nearest valid alternative, and filling smart defaults.

This gives full interactive flexibility ("support both… ask the user") without a
silent cartesian blow-up, and the matrix is the natural home for golden tests.

### 3.1 Config model (`deploy.rs`)

Replace `enum Target { Aws, Local }` with orthogonal enums:

```rust
enum Compute  { Fargate, Eks, Helm, Local }
enum Database { Supabase, Rds, External }      // External carries a URL (in Answers)
enum Storage  { S3, Supabase, External }       // External carries endpoint/bucket/region/keys/path_style
enum Auth     { Supabase, Token, Oidc }        // derived from Database unless overridden
```

`Answers` generalizes to hold: compute/database/storage/auth choices, the BYO
`database_url`, the BYO storage descriptor, Supabase fields (when used), AWS
region, domain/route53, image tag, admin emails, allowed domains, oauth providers,
and (for token auth) a freshly minted admin token.

### 3.2 CLI surface

New flags (all optional; interactive prompts fill the rest):

```
--compute   fargate|eks|helm|local
--database  supabase|rds|external
--database-url <postgresql://...>        # external db
--storage   s3|supabase|external
--storage-endpoint/--storage-bucket/--storage-region
--storage-access-key/--storage-secret/--storage-path-style   # external storage
--auth      supabase|token|oidc
--oidc-issuer/--oidc-client-id/...        # oidc
```

Back-compat: `--target aws` ⇒ `--compute fargate`; `--target local` ⇒
`--compute local`. `--target` is marked deprecated in `--help` and prints a
one-line note steering to `--compute`. `--yes` (non-interactive) requires every
unsupplied axis to have a valid default or an explicit flag.

### 3.3 Wizard flow (interactive)

Order: **compute → database → storage → auth → policy (domain, admins)**. After
collection, `validate(combo)` runs; on success the workspace is materialized.

- **Database = external** → paste `postgresql://…` (read via the private prompt;
  never echoed, never committed).
- **Storage = external** → paste endpoint, bucket, region, access key, secret;
  `path_style` defaults true (MinIO/R2 friendly), toggleable.
- **Auth** defaults: `supabase` when db=supabase, else `token`. The prompt offers
  `token` (wizard mints an admin token, shown once) or `oidc` (collect issuer +
  client id). Supabase Auth is only offered when a Supabase project exists.

### 3.4 Compatibility matrix (the guardrail)

`validate(Compute, Database, Storage, Auth)` enforces (non-exhaustive):

- `storage=s3` or `database=rds` ⇒ requires AWS credentials **and** an
  AWS-rooted compute (`fargate`/`eks`) — native S3 is keyless via task role/IRSA,
  RDS lives in the stack VPC. (BYO/Helm/local with native S3 ⇒ rejected; suggest
  `external` storage with keys, or an AWS compute.)
- `storage=supabase` ⇒ requires a Supabase project: satisfied by `database=supabase`,
  else the wizard provisions a **storage-only** Supabase project.
- `compute=helm` ⇒ a reachable `kubectl` current-context (probed) and `helm` on PATH.
- `compute=local` ⇒ docker present; `database=rds`/`storage=s3` rejected (no VPC) —
  suggest `external`/`supabase`.
- `auth=supabase` ⇒ `database=supabase` (or a provisioned Supabase project).

Each rejection returns *why* + the closest valid alternative. Smart defaults:
storage defaults to `supabase` when db=supabase, else `s3` when compute is AWS,
else `external`.

## 4. Config rendering — one renderer per artifact

`Answers` → exactly one artifact, chosen by compute:

| Compute | Artifact | Lifecycle tool |
|---------|----------|----------------|
| fargate, eks | `terraform.tfvars` | terraform / pulumi |
| helm | `values.yaml` | helm |
| local | `local.env` | docker |

**Catalog URL resolution** into `KYMA_CATALOG_URL`:
- supabase → derived pooler URL (existing token/pooler chain),
- rds → Terraform output,
- external → the pasted URL verbatim.

**Storage env** extends today's `storage_environment` / `storage_secrets` locals
with an `external` branch (`KYMA_S3_ENDPOINT/BUCKET/REGION/PATH_STYLE` +
`KYMA_S3_ACCESS_KEY_ID/SECRET` in secrets). Native S3 stays keyless (no keys
emitted). Supabase Storage unchanged (paste flow).

**Auth env**: `KYMA_AUTH_BACKEND` + the matching vars (`KYMA_SUPABASE_*` |
`KYMA_OIDC_*` | `KYMA_AUTH_TOKENS=<minted>:admin`).

Secrets (`database_url`, storage keys, minted token) are written with
`write_private` and pushed to SSM (Fargate/EKS) or a k8s `Secret` (helm); never
into committed tfvars.

## 5. Terraform `stack/` modularization

Add `compute_backend` and `database_backend` variables. Gate modules with
`count`/`for_each` (the stack already does this for `module.storage`):

- `modules/supabase` → `count = use_supabase ? 1 : 0` where
  `use_supabase = database_backend == "supabase" || storage_backend == "supabase"`.
  All `module.supabase.X` refs become `module.supabase[0].X`. Its two
  defaulted+validated vars (`supabase_org_id`, `supabase_db_password`) become
  **conditionally** required via TF ≥1.9 cross-variable validation
  (`condition = var.database_backend != "supabase" || length(...) > 0`), keeping
  the no-args `terraform init` working for the Pulumi bridge.
- **new `modules/rds`** → `count = database_backend == "rds" ? 1 : 0`: a Postgres
  instance in the stack VPC (private, encrypted, automated backups, SG limited to
  the engine SG), URL written to SSM as `KYMA_CATALOG_URL`.
- **new `modules/eks`** → `count = compute_backend == "eks" ? 1 : 0`: cluster +
  managed node group + OIDC provider + an IRSA IAM role for keyless S3, then a
  `helm_release` installing **the kyma Helm chart** (§6). kubernetes/helm
  providers are configured from the created cluster via exec-auth (standard
  pattern; introduces no required no-default vars).
- `modules/ecs-service` → `count = compute_backend == "fargate" ? 1 : 0`.
- `modules/storage` (S3) → unchanged `count` on `storage_backend == "s3"`.
- `modules/network` → shared by fargate + eks.
- `modules/secrets` (SSM) → catalog URL source generalized (supabase | rds | external).

**Pulumi invariant:** no new variable is required-without-default; all gating
keys default to the current behavior (`compute_backend="fargate"`,
`database_backend="supabase"`).

## 6. New Helm chart — `deploy/helm/kyma-engine/`

The single "engine on Kubernetes" artifact, reused by both the `helm` compute
target and the `eks` module's `helm_release`.

Templates: `Deployment` (replicaCount default **1** — engine is single-writer per
catalog; documented), `Service`, `Ingress` (host + TLS, annotations passthrough
for ALB controller / cert-manager), `ServiceAccount` (optional IRSA annotation
for keyless S3 on EKS), `Secret` (catalog URL, storage keys, minted token from
values, flagged sensitive), and env wiring (`KYMA_AUTH_BACKEND` + auth vars,
`KYMA_S3_*`, `KYMA_HTTP_ADDR`). `values.yaml` exposes image repo/tag, env, ingress,
resources, serviceAccount annotations, replicaCount.

Wizard `helm` path renders `values.yaml` and runs `helm upgrade --install`.

## 7. CLI lifecycle generalization

`cmd_up` / `cmd_status` / `cmd_destroy` branch on the stored compute backend:

| Compute | up | status | destroy |
|---------|----|--------|---------|
| fargate/eks | `terraform\|pulumi apply` | TF outputs + `/health` | `… destroy` |
| helm | `helm upgrade --install` | `helm status` + `/health` | `helm uninstall` |
| local | docker run | docker + `/health` | docker rm (+ project delete) |

`DEPLOY_FILES` (the `include_str!` embed list) gains the Helm chart files and the
new `rds`/`eks` modules so the standalone binary stays self-contained — keep in
lockstep with `deploy/` per the existing rule.

### State / back-compat

`DeployState` gains `compute`/`database`/`storage`/`auth` string fields with
serde defaults; the legacy `target` field is still read and migrated
(`aws→fargate`, `local→local`) so existing `~/.kyma/deploy/<name>` workspaces
keep working with `up`/`status`/`destroy`.

## 8. Security best practices (baked in)

- Keyless storage by default whenever `storage=s3` on AWS compute (Fargate task
  role / EKS IRSA); static keys only for BYO S3-compatible.
- RDS: private subnets, encryption at rest, no public access, SG scoped to engine.
- All secrets via `write_private` + SSM / k8s `Secret`; tfvars/values never carry
  the DB URL or keys in committed form (`.gitignore` already covers the workspace).
- TLS recommended everywhere; Ingress supports cert-manager/ALB annotations.
- Token auth mints a high-entropy admin token shown once; OIDC validated via JWKS.

## 9. Testing

- **Unit (`deploy.rs`)**: `validate(combo)` truth table — every valid combo
  passes, every rejected corner returns the documented message; `render_tfvars`,
  `render_values`, `render_local_env` golden output for representative combos;
  CLI flag parsing incl. `--target` back-compat; `DeployState` migration.
- **IaC**: `terraform validate` on `stack/` under each
  `compute_backend`×`database_backend` default set; `helm lint` + `helm template`
  on the chart. Driven through the existing `--print-only` path so each combo is
  cheaply snapshot-tested without cloud calls.
- **No engine tests change** (no engine code change).

## 10. Docs

`docs/site/deploy/` + `deploy/README.md`, developer-first tone (command/var/endpoint
first, no marketing):

- `index.md` — compute × db × storage decision matrix + "which do I pick".
- `cli.md` — new flags + interactive flow + `--target` deprecation.
- `terraform.md` — new vars (rds/eks/external storage), conditional Supabase.
- `helm.md` (new) — chart install, values reference, BYO cluster.
- `kubernetes.md` (new) — EKS path, IRSA keyless S3.
- `pulumi.md` — unchanged bridge note + new vars.

## 11. Build/deploy mechanics (for the implementer)

UI is embedded at compile time; this work touches no web assets, so the rebuild
is just `cargo build -p kyma-cli`. The CLI binary collides with `kyma-bin`
(`target/debug/kyma`) — build/test the specific package. New embedded template
files must be added to both `deploy/` on disk and `DEPLOY_FILES`.

## 12. Resolved decisions

- EKS reuses the Helm chart (not a separate manifest set) — DRY.
- Auth derived-but-overridable; OIDC is a first-class wizard option, token is the
  non-Supabase default.
- Everything in one push (no phasing), per the user's scope choice.
