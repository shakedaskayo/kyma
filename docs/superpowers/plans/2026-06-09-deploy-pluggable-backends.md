# Pluggable Deploy Backends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `kyma deploy` (wizard + Terraform stack + new Helm chart + docs) pluggable across four orthogonal axes — compute (fargate/eks/helm/local), database (supabase/rds/external), storage (s3/supabase/external), auth (supabase/token/oidc) — selected interactively, with bring-your-own DB URL and S3-compatible storage.

**Architecture:** Orthogonal axes + a `validate(combo)` compatibility gate (Approach C). The engine is already backend-agnostic (`kyma-storage` S3Compatible/Local; generic `KYMA_CATALOG_URL`; `supabase`/`oidc`/`env` auth backends), so **no engine code changes** — work is confined to `crates/kyma-cli/src/deploy.rs`, `deploy/terraform/stack/`, a new `deploy/helm/kyma-engine/`, and `docs/site/deploy/`.

**Tech Stack:** Rust (clap, serde, reqwest, tokio, wiremock for tests), Terraform (AWS + Supabase + kubernetes/helm providers), Helm, VitePress docs.

**Spec:** `docs/superpowers/specs/2026-06-09-deploy-pluggable-backends-design.md`

**Working location:** worktree `feat/deploy-pluggable-backends` (branch `worktree-feat+deploy-pluggable-backends`), off `origin/main`. Merge to `main` with `git merge --no-ff` at the end (per user's local-merge preference).

**Validation boundary:** static/CI-grade only — `cargo test`/`clippy`/`fmt`, `terraform validate`, `helm lint`/`helm template`, and `--print-only` golden snapshots. A live AWS/EKS/RDS `apply` needs real credentials + spend and is out of scope (matches the existing "e2e smoke on a real account" open item).

**Build/test note:** `kyma-cli` is a **binary crate** — run unit tests with `cargo test -p kyma-cli --bins` (not `--lib`). `kyma-cli` and `kyma-bin` both emit `target/debug/kyma`; always pass `-p kyma-cli`. New embedded template files must be added to **both** `deploy/` on disk and the `DEPLOY_FILES` array in `deploy.rs`.

---

## File Structure

**Modify**
- `crates/kyma-cli/src/deploy.rs` — config model, matrix, renderers, CLI flags, lifecycle, `DEPLOY_FILES`, tests (split helpers into submodules if it grows past ~2500 lines).
- `crates/kyma-cli/src/main.rs` — pass new `deploy init` flags through (clap wiring around the existing `Op::Init`).
- `deploy/terraform/stack/{main,variables,outputs}.tf` — module gating + new vars/outputs.
- `deploy/terraform/stack/modules/supabase/*` — make conditionally instantiated.
- `deploy/terraform/stack/modules/secrets/*`, `.../storage/*` — generalize catalog/storage sources.
- `deploy/terraform/{variables,outputs,terraform.tfvars.example}.tf` — top-level passthrough of new vars.
- `deploy/README.md`, `docs/site/deploy/{index,cli,terraform,pulumi}.md`, `docs/site/.vitepress/config.*` (nav).

**Create**
- `deploy/terraform/stack/modules/rds/{main,variables,outputs,versions}.tf`
- `deploy/terraform/stack/modules/eks/{main,variables,outputs,versions}.tf`
- `deploy/helm/kyma-engine/{Chart.yaml,values.yaml,.helmignore}` + `templates/{deployment,service,ingress,serviceaccount,secret,_helpers.tpl,NOTES.txt}.yaml`
- `docs/site/deploy/helm.md`, `docs/site/deploy/kubernetes.md`

---

## Phase 1 — Wizard config model + compatibility matrix

Goal: orthogonal axis enums, a generalized `Answers`, a tested `validate()` gate, new CLI flags with `--target` back-compat, and `DeployState` migration. End state: `cargo test -p kyma-cli --bins` green; `kyma deploy init --help` shows new flags.

### Task 1.1: Axis enums + parse

**Files:** Modify `crates/kyma-cli/src/deploy.rs` (near the existing `IacTool`/`Target` enums ~L36); Test: same file `mod tests`.

- [ ] **Step 1: Write failing tests** for enum parsing + the `--target` alias.

```rust
#[test]
fn compute_parses_and_target_alias_maps() {
    assert_eq!(Compute::from_arg("fargate").unwrap(), Compute::Fargate);
    assert_eq!(Compute::from_arg("eks").unwrap(), Compute::Eks);
    assert_eq!(Compute::from_arg("helm").unwrap(), Compute::Helm);
    assert_eq!(Compute::from_arg("local").unwrap(), Compute::Local);
    // legacy --target aws|local
    assert_eq!(Compute::from_target("aws"), Some(Compute::Fargate));
    assert_eq!(Compute::from_target("local"), Some(Compute::Local));
    assert!(Compute::from_arg("ec2").is_err());
}

#[test]
fn database_and_storage_and_auth_parse() {
    assert_eq!(Database::from_arg("supabase").unwrap(), Database::Supabase);
    assert_eq!(Database::from_arg("rds").unwrap(), Database::Rds);
    assert_eq!(Database::from_arg("external").unwrap(), Database::External);
    assert_eq!(Storage::from_arg("s3").unwrap(), Storage::S3);
    assert_eq!(Storage::from_arg("external").unwrap(), Storage::External);
    assert_eq!(Auth::from_arg("oidc").unwrap(), Auth::Oidc);
}
```

- [ ] **Step 2: Run, expect fail** (`cargo test -p kyma-cli --bins compute_parses` → unresolved `Compute`).

- [ ] **Step 3: Implement** the enums with `clap::ValueEnum` + `as_str`/`from_arg`/`from_target`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Compute { Fargate, Eks, Helm, Local }
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Database { Supabase, Rds, External }
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Storage { S3, Supabase, External }
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Auth { Supabase, Token, Oidc }

impl Compute {
    fn as_str(self) -> &'static str { match self { Self::Fargate=>"fargate", Self::Eks=>"eks", Self::Helm=>"helm", Self::Local=>"local" } }
    fn from_arg(s: &str) -> anyhow::Result<Self> { match s { "fargate"=>Ok(Self::Fargate), "eks"=>Ok(Self::Eks), "helm"=>Ok(Self::Helm), "local"=>Ok(Self::Local), o=>anyhow::bail!("unknown compute {o:?}") } }
    fn from_target(s: &str) -> Option<Self> { match s { "aws"=>Some(Self::Fargate), "local"=>Some(Self::Local), _=>None } }
    fn is_aws(self) -> bool { matches!(self, Self::Fargate | Self::Eks) }
    fn is_k8s(self) -> bool { matches!(self, Self::Eks | Self::Helm) }
}
// Database/Storage/Auth: mirror as_str + from_arg with their variants.
```

- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Commit** `git add -A && git commit -m "feat(deploy): orthogonal compute/database/storage/auth axis enums"`

### Task 1.2: Generalize `Answers` + BYO descriptors

**Files:** Modify `deploy.rs` `struct Answers` (~L309) and its test fixture (~L1646).

- [ ] **Step 1:** Update the `answers()` test fixture to the new shape (compile-driver), then adjust the existing `tfvars_renders_every_answer` test in Task 2.1.
- [ ] **Step 2:** Replace `Answers` with:

```rust
#[derive(Debug, Clone)]
struct ExternalStorage { endpoint: String, bucket: String, region: String, access_key_id: String, secret_access_key: String, path_style: bool }

#[derive(Debug, Clone)]
struct Answers {
    name: String,                 // workspace name (e.g. "prod")
    project_name: String,         // "kyma-{name}"
    compute: Compute,
    database: Database,
    storage: Storage,
    auth: Auth,
    aws_region: String,
    // Supabase (used when database==Supabase or storage==Supabase)
    supabase_org_id: String,
    supabase_region: String,
    supabase_db_password: String,
    supabase_s3_access_key_id: String,
    supabase_s3_secret_access_key: String,
    // BYO Postgres
    database_url: String,         // External only
    // BYO S3-compatible
    external_storage: Option<ExternalStorage>,
    // auth
    admin_emails: Vec<String>,
    allowed_email_domains: Vec<String>,
    oauth_providers: Vec<String>,
    admin_token: String,          // minted for Auth::Token
    oidc_issuer: String,
    oidc_client_id: String,
    // common
    domain: String,
    route53_zone_id: String,
    image_tag: String,
    // k8s
    kube_context: String,         // Helm target
    ingress_host: String,
}
```

- [ ] **Step 3:** Make it compile (renderers updated in Phase 2; for now keep `render_tfvars` reading the new fields). **Step 4:** `cargo build -p kyma-cli`. **Step 5:** Commit `refactor(deploy): generalize Answers for all backend axes`.

### Task 1.3: `validate(combo)` compatibility matrix

**Files:** Modify `deploy.rs` (new `fn validate_combo`); Test: `mod tests`.

- [ ] **Step 1: Failing tests** — the truth table:

```rust
fn combo(c: Compute, d: Database, s: Storage, a: Auth) -> Result<()> {
    validate_combo(c, d, s, a)
}
#[test]
fn valid_combos_pass() {
    assert!(combo(Compute::Fargate, Database::Supabase, Storage::Supabase, Auth::Supabase).is_ok());
    assert!(combo(Compute::Fargate, Database::Rds, Storage::S3, Auth::Token).is_ok());
    assert!(combo(Compute::Eks, Database::Rds, Storage::S3, Auth::Oidc).is_ok());
    assert!(combo(Compute::Helm, Database::External, Storage::External, Auth::Token).is_ok());
    assert!(combo(Compute::Local, Database::External, Storage::External, Auth::Token).is_ok());
    assert!(combo(Compute::Local, Database::Supabase, Storage::Supabase, Auth::Supabase).is_ok());
}
#[test]
fn invalid_combos_rejected_with_reason() {
    // native S3 needs AWS compute
    let e = combo(Compute::Helm, Database::External, Storage::S3, Auth::Token).unwrap_err().to_string();
    assert!(e.contains("native S3") && e.contains("external"), "{e}");
    // RDS needs AWS compute
    assert!(combo(Compute::Local, Database::Rds, Storage::External, Auth::Token).is_err());
    // supabase auth needs supabase db
    assert!(combo(Compute::Fargate, Database::Rds, Storage::S3, Auth::Supabase).is_err());
    // supabase storage needs a supabase project (db=supabase here is absent)
    assert!(combo(Compute::Eks, Database::Rds, Storage::Supabase, Auth::Token).is_err());
}
```

- [ ] **Step 2:** Run → fail (unresolved `validate_combo`).
- [ ] **Step 3: Implement:**

```rust
fn validate_combo(c: Compute, d: Database, s: Storage, a: Auth) -> Result<()> {
    if s == Storage::S3 && !c.is_aws() {
        bail!("storage=s3 (native AWS S3, keyless via task/pod role) requires an AWS compute target (fargate or eks). \
               Use storage=external with endpoint+keys for a non-AWS S3-compatible store, or switch compute to fargate/eks.");
    }
    if d == Database::Rds && !c.is_aws() {
        bail!("database=rds is provisioned inside the stack VPC and requires compute=fargate or eks. \
               Use database=external with a postgresql:// URL on this compute target instead.");
    }
    if a == Auth::Supabase && d != Database::Supabase {
        bail!("auth=supabase requires database=supabase (Supabase Auth is tied to the Supabase project). \
               Use auth=token (a minted admin token) or auth=oidc instead.");
    }
    if s == Storage::Supabase && d != Database::Supabase {
        bail!("storage=supabase needs a Supabase project; either set database=supabase, or choose storage=s3/external. \
               (A storage-only Supabase project is not auto-provisioned in this version.)");
    }
    Ok(())
}
```

(Note: §3.4 of the spec mentions a storage-only Supabase project; we simplify to "supabase storage requires supabase db" — recorded as an intentional narrowing. Update the spec line if needed.)

- [ ] **Step 4:** Run → pass. **Step 5:** Commit `feat(deploy): compatibility matrix validate_combo`.

### Task 1.4: Default-derivation helpers

**Files:** `deploy.rs`; Test: `mod tests`.

- [ ] **Step 1: Failing test:**

```rust
#[test]
fn storage_default_follows_db_then_compute() {
    assert_eq!(default_storage(Compute::Fargate, Database::Supabase), Storage::Supabase);
    assert_eq!(default_storage(Compute::Fargate, Database::Rds), Storage::S3);
    assert_eq!(default_storage(Compute::Eks, Database::External), Storage::S3);
    assert_eq!(default_storage(Compute::Helm, Database::External), Storage::External);
    assert_eq!(default_auth(Database::Supabase), Auth::Supabase);
    assert_eq!(default_auth(Database::Rds), Auth::Token);
    assert_eq!(default_auth(Database::External), Auth::Token);
}
```

- [ ] **Step 2-3:** Implement:

```rust
fn default_storage(c: Compute, d: Database) -> Storage {
    if d == Database::Supabase { Storage::Supabase }
    else if c.is_aws() { Storage::S3 }
    else { Storage::External }
}
fn default_auth(d: Database) -> Auth { if d == Database::Supabase { Auth::Supabase } else { Auth::Token } }
```

- [ ] **Step 4-5:** Run → pass; commit `feat(deploy): smart default derivation for storage/auth`.

### Task 1.5: CLI flags + `--target` deprecation

**Files:** Modify `deploy.rs` `Op::Init` (~L52) and `crates/kyma-cli/src/main.rs` (clap wiring).

- [ ] **Step 1:** Add flags to `Op::Init`, keeping `target: Option<Target>` for back-compat:

```rust
#[arg(long, value_enum)] compute: Option<Compute>,
#[arg(long, value_enum)] database: Option<Database>,
#[arg(long)] database_url: Option<String>,
#[arg(long, value_enum)] storage: Option<Storage>,
#[arg(long)] storage_endpoint: Option<String>,
#[arg(long)] storage_bucket: Option<String>,
#[arg(long)] storage_region: Option<String>,
#[arg(long)] storage_access_key: Option<String>,
#[arg(long)] storage_secret: Option<String>,
#[arg(long)] storage_path_style: Option<bool>,
#[arg(long, value_enum)] auth: Option<Auth>,
#[arg(long)] oidc_issuer: Option<String>,
#[arg(long)] oidc_client_id: Option<String>,
#[arg(long)] kube_context: Option<String>,
#[arg(long)] ingress_host: Option<String>,
/// DEPRECATED alias: --target aws→--compute fargate, local→--compute local.
#[arg(long, value_enum)] target: Option<Target>,
```

- [ ] **Step 2:** In `run()`/`cmd_init`, resolve `compute = compute.or(target.and_then(|t| ...)).unwrap_or(Fargate)` and print a deprecation note when `--target` is used. Keep `tool`, `region`, `supabase_org`, `domain`, `admin_email`, `yes`, `print_only`, `force`.
- [ ] **Step 3:** `cargo build -p kyma-cli`; run `cargo run -p kyma-cli -- deploy init --help` and confirm the flags render. **Step 4:** Commit `feat(deploy): CLI flags for axis selection + --target back-compat`.

### Task 1.6: `DeployState` migration

**Files:** `deploy.rs` `struct DeployState` (~L253) + `load_state`; Test.

- [ ] **Step 1: Failing test:**

```rust
#[test]
fn deploy_state_migrates_legacy_target() {
    // legacy file with only `target`
    let legacy = r#"{"target":"aws","tool":"terraform","project_name":"kyma-prod","aws_region":"us-east-1","image_tag":"v1"}"#;
    let st: DeployState = serde_json::from_str(legacy).unwrap();
    assert_eq!(st.compute(), Compute::Fargate);
    let legacy_local = r#"{"target":"local","tool":"terraform","project_name":"p","aws_region":"r","image_tag":"v"}"#;
    let st2: DeployState = serde_json::from_str(legacy_local).unwrap();
    assert_eq!(st2.compute(), Compute::Local);
}
```

- [ ] **Step 2-3:** Add `#[serde(default)] compute: Option<String>` etc. and a `fn compute(&self) -> Compute { self.compute.as_deref().and_then(|s| Compute::from_arg(s).ok()).or_else(|| Compute::from_target(&self.target)).unwrap_or(Compute::Fargate) }`. Add `database`/`storage`/`auth` optional fields with similar accessors. Keep writing `target` for forward/back compat (set from compute).
- [ ] **Step 4-5:** Run → pass; commit `feat(deploy): DeployState carries axes + migrates legacy target`.

---

## Phase 2 — Config rendering

End state: three renderers produce correct artifacts for every axis combination, all golden-tested.

### Task 2.1: Extend `render_tfvars`

**Files:** `deploy.rs` `render_tfvars` (~L334) + stack `variables.tf` additions happen in Phase 3; Test: rewrite `tfvars_renders_every_answer`.

- [ ] **Step 1: Failing test** covering the new vars for a `fargate + rds + s3 + token` combo and a `fargate + supabase + supabase + supabase` combo:

```rust
#[test]
fn tfvars_renders_rds_s3_token() {
    let mut a = answers(); // fixture now defaults fargate+supabase
    a.compute = Compute::Fargate; a.database = Database::Rds; a.storage = Storage::S3; a.auth = Auth::Token;
    a.admin_token = "tok123".into();
    let r = render_tfvars(&a);
    for n in [r#"compute_backend       = "fargate""#, r#"database_backend      = "rds""#,
              r#"storage_backend       = "s3""#, r#"auth_backend          = "token""#,
              r#"admin_token           = "tok123""#] {
        assert!(r.contains(n), "missing {n} in:\n{r}");
    }
    assert!(!r.contains("supabase_org_id       = \"\""), "no empty supabase org when rds");
}
```

- [ ] **Step 2-3:** Extend the tfvars template with `compute_backend`, `database_backend`, `auth_backend`, `database_url`, `admin_token`, `oidc_issuer`, `oidc_client_id`, `oauth_providers`, and external-storage vars (`storage_endpoint`/`storage_bucket`/`storage_region`/`storage_access_key`/`storage_secret`/`storage_path_style`). Quote-escape via existing `hcl_string_list` for lists. **Keep secrets** (`database_url`, `storage_secret`, `admin_token`, `supabase_db_password`) in the tfvars (written 0600, never committed) — matches today's pattern.
- [ ] **Step 4-5:** Run → pass; commit `feat(deploy): render_tfvars covers all axes`.

### Task 2.2: `render_helm_values`

**Files:** `deploy.rs` new fn; Test.

- [ ] **Step 1: Failing test:**

```rust
#[test]
fn helm_values_render_external_db_and_storage() {
    let mut a = answers();
    a.compute = Compute::Helm; a.database = Database::External; a.storage = Storage::External; a.auth = Auth::Token;
    a.database_url = "postgresql://u:p@host:5432/db".into();
    a.admin_token = "tok".into();
    a.ingress_host = "kyma.example.com".into();
    a.external_storage = Some(ExternalStorage{ endpoint:"https://minio:9000".into(), bucket:"kyma".into(), region:"us-east-1".into(), access_key_id:"AK".into(), secret_access_key:"SK".into(), path_style:true });
    let y = render_helm_values(&a);
    assert!(y.contains("repository: ghcr.io/shakedaskayo/kyma-engine"));
    assert!(y.contains("KYMA_CATALOG_URL: postgresql://u:p@host:5432/db"));
    assert!(y.contains("KYMA_AUTH_BACKEND: token"));
    assert!(y.contains("KYMA_AUTH_TOKENS: tok:admin"));
    assert!(y.contains("KYMA_S3_ENDPOINT: https://minio:9000"));
    assert!(y.contains("KYMA_S3_PATH_STYLE: \"true\""));
    assert!(y.contains("host: kyma.example.com"));
}
```

- [ ] **Step 2-3:** Implement a YAML-emitting function (string template; values are simple — quote numbers/bools as strings to keep env values as strings). Map auth: supabase→`KYMA_SUPABASE_*`, token→`KYMA_AUTH_TOKENS: <token>:admin`, oidc→`KYMA_OIDC_ISSUER`/`KYMA_OIDC_CLIENT_ID`. Storage: external→full `KYMA_S3_*`; s3→bucket+region+`serviceAccount.annotations` for IRSA (no keys); supabase is **not** a Helm path (matrix forbids supabase storage without supabase db, and supabase-db Helm uses external storage or supabase storage keys via env — include keys when present).
- [ ] **Step 4-5:** Run → pass; commit `feat(deploy): render_helm_values`.

### Task 2.3: Generalize `render_local_env`

**Files:** `deploy.rs` `render_local_env` (~L381); update existing two local-env tests + add BYO test.

- [ ] **Step 1:** Add a failing test for `local + external db + external storage + token`:

```rust
#[test]
fn local_env_external_db_storage_token() {
    let a = /* Answers: Local/External/External/Token, database_url set, external_storage set, admin_token "tok" */;
    let env = render_local_env_from(&a);
    assert!(env.contains("KYMA_CATALOG_URL=postgresql://"));
    assert!(env.contains("KYMA_AUTH_BACKEND=token"));
    assert!(env.contains("KYMA_AUTH_TOKENS=tok:admin"));
    assert!(env.contains("KYMA_S3_ENDPOINT="));
}
```

- [ ] **Step 2-3:** Introduce `render_local_env_from(&Answers) -> String` that handles all db/storage/auth branches, and keep the old `render_local_env(...)` signature as a thin adapter used by the Supabase provisioning path (so existing tests `local_env_wires_supabase_*` keep passing unchanged). **Step 4-5:** Run → pass; commit `feat(deploy): local.env supports BYO db/storage + token/oidc auth`.

---

## Phase 3 — Terraform stack modularization

End state: `terraform -chdir=deploy/terraform/stack init` works no-args (Pulumi invariant) and `terraform validate` passes for each `compute_backend`×`database_backend`. (Use a tiny harness dir that calls the stack module — or `terraform validate` inside `stack/` directly since modules validate standalone.)

### Task 3.1: New variables + conditional Supabase validation

**Files:** `deploy/terraform/stack/variables.tf`.

- [ ] **Step 1:** Add `compute_backend` (default `"fargate"`, validation `contains(["fargate","eks","helm","local"], …)`), `database_backend` (default `"supabase"`, `contains(["supabase","rds","external"])`), `auth_backend` (default `"supabase"`), `database_url` (default `""`, sensitive), `admin_token` (default `""`, sensitive), `oidc_issuer`/`oidc_client_id` (default `""`), external-storage vars (`storage_endpoint`/`storage_region`/`storage_access_key`/`storage_secret` default `""`, `storage_path_style` bool default `true`), `oauth_providers` already exists.
- [ ] **Step 2:** Make `supabase_org_id`/`supabase_db_password` **conditionally** required (TF ≥1.9 cross-var validation):

```hcl
validation {
  condition     = var.database_backend != "supabase" || length(var.supabase_org_id) > 0
  error_message = "supabase_org_id is required when database_backend = \"supabase\"."
}
```

- [ ] **Step 3:** `terraform -chdir=deploy/terraform/stack init -backend=false && terraform -chdir=deploy/terraform/stack validate` → expect pass with defaults. **Step 4:** Commit `feat(deploy/tf): backend-selector variables + conditional supabase validation`.

### Task 3.2: Gate the Supabase module

**Files:** `deploy/terraform/stack/main.tf`.

- [ ] **Step 1:** `locals { use_supabase = var.database_backend == "supabase" || var.storage_backend == "supabase" }`. Add `count = local.use_supabase ? 1 : 0` to `module.supabase`; change all `module.supabase.X` → `module.supabase[0].X` guarded by `local.use_supabase`. **Step 2:** `terraform validate` with `-var database_backend=external -var compute_backend=helm` → pass. **Step 3:** Commit `feat(deploy/tf): make supabase module conditional`.

### Task 3.3: `modules/rds`

**Files:** Create `deploy/terraform/stack/modules/rds/{main,variables,outputs,versions}.tf`.

- [ ] **Step 1:** `aws_db_subnet_group` (private subnets from network), `aws_security_group` allowing 5432 from the engine SG only, `aws_db_instance` (postgres, `storage_encrypted=true`, `publicly_accessible=false`, `backup_retention_period=7`, `db_name="kyma"`, username/password from vars/random). Output `database_url = "postgresql://${username}:${password}@${endpoint}/kyma"` (sensitive).
- [ ] **Step 2:** In `main.tf`, instantiate with `count = var.database_backend == "rds" ? 1 : 0`, passing `vpc_id`/private subnet ids/engine SG. Requires the network module to expose private subnets — add them (Task 3.6). **Step 3:** `terraform validate -var database_backend=rds` → pass. **Step 4:** Commit `feat(deploy/tf): RDS Postgres catalog module`.

### Task 3.4: `modules/eks` (provisions cluster + installs the Helm chart)

**Files:** Create `deploy/terraform/stack/modules/eks/*`; `versions.tf` adds `kubernetes` + `helm` providers.

- [ ] **Step 1:** `aws_eks_cluster` + `aws_eks_node_group` (managed, small), `aws_iam_openid_connect_provider` from the cluster OIDC issuer, an IRSA `aws_iam_role` trusting the SA `system:serviceaccount:<ns>:kyma-engine` with the S3 access policy. Configure `kubernetes`/`helm` providers via the cluster endpoint + `data.aws_eks_cluster_auth` exec token. A `helm_release "kyma"` pointing at `${path.module}/../../../../helm/kyma-engine` (or a packaged chart path) with values set from inputs (image, env, ingress_host, serviceAccount IRSA annotation, S3 bucket/region).
- [ ] **Step 2:** In `main.tf`, `count = var.compute_backend == "eks" ? 1 : 0`; ecs-service module gets `count = var.compute_backend == "fargate" ? 1 : 0`. **Step 3:** `terraform validate -var compute_backend=eks` (providers require no creds for validate). **Step 4:** Commit `feat(deploy/tf): EKS module installing the kyma Helm chart (IRSA keyless S3)`.

### Task 3.5: Generalize secrets + storage env locals

**Files:** `deploy/terraform/stack/main.tf`, `modules/secrets`, `modules/storage`.

- [ ] **Step 1:** `KYMA_CATALOG_URL` source local:

```hcl
locals {
  catalog_url = (
    var.database_backend == "supabase" ? module.supabase[0].database_url :
    var.database_backend == "rds"      ? module.rds[0].database_url :
    var.database_url
  )
}
```

- [ ] **Step 2:** Extend `storage_environment`/`storage_secrets` with the `external` branch (use `var.storage_endpoint/bucket/region/path_style`, keys to secrets) alongside the existing s3/supabase branches. Auth env local switches on `var.auth_backend` (supabase vars | `KYMA_AUTH_TOKENS=${var.admin_token}:admin` | oidc vars). **Step 3:** `terraform validate` across backends. **Step 4:** Commit `feat(deploy/tf): catalog/storage/auth env selection by backend`.

### Task 3.6: Network private subnets + outputs + top-level passthrough

**Files:** `modules/network/*`, `stack/outputs.tf`, `deploy/terraform/{main,variables,outputs}.tf`, `terraform.tfvars.example`.

- [ ] **Step 1:** Add private subnets + (for RDS, no NAT needed since RDS isn't egressing) to network module; export `private_subnet_ids`. **Step 2:** Surface new stack outputs (`engine_url` already exists; keep it computed per compute backend — for eks/helm the URL comes from ingress host). Passthrough all new vars in the top-level `deploy/terraform/variables.tf` + example. **Step 3:** `terraform -chdir=deploy/terraform init -backend=false && validate`. **Step 4:** Commit `feat(deploy/tf): network private subnets, outputs, top-level var passthrough`.

---

## Phase 4 — Helm chart

End state: `helm lint deploy/helm/kyma-engine` clean; `helm template` renders for token/supabase/oidc + s3/external storage value sets.

### Task 4.1: Chart scaffold + values

**Files:** Create `deploy/helm/kyma-engine/Chart.yaml`, `values.yaml`, `.helmignore`, `templates/_helpers.tpl`.

- [ ] **Step 1:** `Chart.yaml` (apiVersion v2, name kyma-engine, type application, version 0.1.0, appVersion matches engine). `values.yaml` keys: `image.{repository,tag,pullPolicy}`, `replicaCount: 1` (with a comment: engine is single-writer per catalog), `env: {}` (map of KYMA_* → string), `secretEnv: {}` (sensitive KYMA_* → string, rendered into a Secret), `service.{type,port:8080}`, `ingress.{enabled,className,host,tls,annotations}`, `serviceAccount.{create,name,annotations}`, `resources`. **Step 2:** `helm lint deploy/helm/kyma-engine`. **Step 3:** Commit `feat(deploy/helm): chart scaffold + values`.

### Task 4.2: Templates

**Files:** `templates/{serviceaccount,secret,deployment,service,ingress,NOTES}.yaml`.

- [ ] **Step 1:** `serviceaccount.yaml` (create if `.Values.serviceAccount.create`, with annotations — IRSA role ARN on EKS). `secret.yaml` (Opaque, `stringData` from `.Values.secretEnv`). `deployment.yaml` (1 replica, container port 8080, `envFrom` the Secret + explicit `env` from `.Values.env`, liveness/readiness `GET /health`, resources, serviceAccountName). `service.yaml` (ClusterIP :8080). `ingress.yaml` (gated on `.Values.ingress.enabled`, host + TLS + annotations). `NOTES.txt` (how to reach it + mint a token).
- [ ] **Step 2:** Render checks:

```bash
helm template t deploy/helm/kyma-engine \
  --set env.KYMA_AUTH_BACKEND=token --set secretEnv.KYMA_AUTH_TOKENS=tok:admin \
  --set env.KYMA_S3_ENDPOINT=https://minio:9000 --set ingress.enabled=true --set ingress.host=k.example.com | head -60
```

Expect a valid Deployment/Service/Ingress/Secret. **Step 3:** Commit `feat(deploy/helm): engine templates (deployment/service/ingress/sa/secret)`.

---

## Phase 5 — CLI lifecycle + DEPLOY_FILES

End state: `kyma deploy init/up/status/destroy` work for fargate (unchanged), helm (helm install), eks (terraform), local (docker); `--print-only` prints the right artifact for every combo; all new template files embedded.

### Task 5.1: Rewrite `cmd_init` to use the axis model

**Files:** `deploy.rs` `cmd_init` (~L1108).

- [ ] **Step 1:** Resolve axes from flags → interactive prompts (compute→database[+url]→storage[+creds]→auth) → fill defaults via `default_storage`/`default_auth` → `validate_combo()` (bail with the message on error). Mint `admin_token = random_token(40)` when `auth==Token`. For Supabase paths keep the token/org/pooler chain; skip it entirely for external/rds+token. **Step 2:** materialize + write the right artifact (tfvars for fargate/eks, values.yaml for helm, local.env for local) via the Phase 2 renderers; write secrets 0600. **Step 3:** Build + `cargo test -p kyma-cli --bins`. **Step 4:** Commit `feat(deploy): cmd_init drives the axis model + matrix`.

### Task 5.2: `--print-only` per artifact (golden smoke)

**Files:** `deploy.rs` print-only branch (~L1262); Test.

- [ ] **Step 1: Failing test** that calls a pure helper `planned_artifact(&Answers) -> (String /*filename*/, String /*contents*/)` and asserts the filename is `terraform.tfvars` / `values.yaml` / `local.env` per compute. **Step 2-3:** Extract that helper and use it in both `--print-only` and `cmd_init`. **Step 4-5:** Run → pass; commit `feat(deploy): print-only renders the chosen artifact; planned_artifact helper`.

### Task 5.3: `cmd_up`/`status`/`destroy` branch on compute

**Files:** `deploy.rs` (~L1360–1638).

- [ ] **Step 1:** Switch on `state.compute()`:
  - `Fargate`/`Eks` → existing terraform|pulumi path (Eks reuses it; engine_url from ingress output for eks).
  - `Helm` → `helm upgrade --install kyma <chart> -f values.yaml -n kyma --create-namespace [--kube-context <ctx>]`; status via `helm status` + `/health`; destroy via `helm uninstall`.
  - `Local` → docker (unchanged).
- [ ] **Step 2:** The Supabase-storage two-phase key-paste stays only on the supabase-storage branch. **Step 3:** Build + tests. **Step 4:** Commit `feat(deploy): up/status/destroy branch on compute (helm/eks)`.

### Task 5.4: Embed new templates in `DEPLOY_FILES`

**Files:** `deploy.rs` `DEPLOY_FILES` (~L111); Test `materialize_writes_all_templates_and_never_tfvars` already iterates the list.

- [ ] **Step 1:** Add `include_str!` entries for `modules/rds/*`, `modules/eks/*`, and the helm chart files (`helm/kyma-engine/Chart.yaml`, `values.yaml`, each template). Use a relative `helm/kyma-engine/...` workspace path. **Step 2:** `cargo test -p kyma-cli --bins materialize` → pass (proves every embedded file exists on disk). **Step 3:** Commit `feat(deploy): embed RDS/EKS modules + Helm chart in DEPLOY_FILES`.

---

## Phase 6 — Docs

End state: deploy docs describe the matrix and every backend; nav updated.

### Task 6.1: `deploy/README.md` + `docs/site/deploy/index.md`

- [ ] **Step 1:** Rewrite the intro to a compute×db×storage matrix table + "which do I pick" (developer-first: commands/vars first, no marketing — per `feedback_docs_developer_friendly`). **Step 2:** Commit `docs(deploy): backend matrix overview`.

### Task 6.2: `cli.md`, `terraform.md`, `pulumi.md`

- [ ] **Step 1:** `cli.md` — new flags, interactive flow, `--target` deprecation, examples per combo. `terraform.md` — new variables (rds/eks/external/auth), conditional supabase, `terraform validate` recipe. `pulumi.md` — unchanged bridge note + new vars. **Step 2:** Commit `docs(deploy): cli/terraform/pulumi for pluggable backends`.

### Task 6.3: new `helm.md` + `kubernetes.md` + nav

- [ ] **Step 1:** `helm.md` — `helm repo`/local chart install, full values reference, BYO cluster, ingress/TLS. `kubernetes.md` — EKS via `kyma deploy`, IRSA keyless S3, namespace. Add both to `docs/site/.vitepress/config.*` deploy sidebar. **Step 2:** Commit `docs(deploy): Helm + Kubernetes (EKS) pages`.

---

## Phase 7 — Full validation

- [ ] `cargo test -p kyma-cli --bins` → all green (capture `test result: ok`).
- [ ] `cargo clippy -p kyma-cli --bins -- -D warnings` → clean.
- [ ] `cargo fmt -p kyma-cli -- --check` → clean.
- [ ] `terraform -chdir=deploy/terraform/stack init -backend=false` then `validate` under each: defaults (fargate+supabase), `-var compute_backend=eks -var database_backend=rds -var storage_backend=s3`, `-var database_backend=external -var storage_backend=external -var compute_backend=helm`. Also `terraform fmt -check -recursive deploy/terraform`.
- [ ] `helm lint deploy/helm/kyma-engine` + `helm template` for token/supabase/oidc value sets.
- [ ] `cargo run -p kyma-cli -- deploy init --print-only` for ≥4 representative combos (fargate+supabase+supabase, fargate+rds+s3+token, eks+rds+s3+oidc, helm+external+external+token, local+external+external+token) — eyeball each artifact.
- [ ] Update the spec's §3.4 note if the storage-only-Supabase narrowing was applied. Commit any doc fixups.

If any command isn't installed (terraform/helm), note it and rely on the unit + `--print-only` golden tests; do not claim a check passed that wasn't run.

---

## Phase 8 — Merge to main

- [ ] Re-run Phase 7 quick gate (`cargo test -p kyma-cli --bins`) to confirm green.
- [ ] `git -C <main worktree> checkout main && git pull` is NOT used here — instead, per the local-merge preference: from a clean main checkout, `git merge --no-ff worktree-feat+deploy-pluggable-backends` and `git push`. Confirm with the user before pushing if main has moved.
- [ ] Use `superpowers:requesting-code-review` before merge; address findings.
- [ ] After merge, offer to remove the worktree (`ExitWorktree`).

---

## Self-Review notes (author)

- Spec coverage: compute (Ph1 enums, Ph3 eks, Ph4/5 helm), database (Ph1, Ph2 renderers, Ph3 rds+external), storage (Ph2, Ph3 external branch, Ph4 IRSA), auth (Ph1 default_auth, Ph2 token/oidc env, Ph3 auth env local). Matrix = Task 1.3. Back-compat = 1.5/1.6. Docs = Ph6. Testing = Ph7. ✔
- Intentional narrowing vs spec §3.4: "supabase storage requires database=supabase" (no auto storage-only Supabase project). Reconcile spec text in Phase 7.
- Type consistency: `validate_combo`, `default_storage`, `default_auth`, `render_tfvars`, `render_helm_values`, `render_local_env_from`, `planned_artifact`, `DeployState::compute()` — names used consistently across tasks.
