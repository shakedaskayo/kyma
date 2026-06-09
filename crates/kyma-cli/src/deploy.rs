//! `kyma deploy` — one-command production (and local test-drive) deployment.
//!
//! Targets:
//! - **aws** (default): ECS Fargate engine + S3 extents + Supabase
//!   (catalog Postgres + Auth), provisioned by the embedded Terraform stack
//!   (or Pulumi wrapping the same stack via the terraform-module bridge).
//! - **local**: test-drive the Supabase wiring without AWS — provisions just
//!   a Supabase project via the Management API, writes `local.env`, and runs
//!   the engine container locally with docker.
//!
//! Credential acquisition is interactive-by-default and obtains tokens on the
//! user's behalf where possible. Supabase access-token resolution order:
//!   1. `SUPABASE_ACCESS_TOKEN` env var
//!   2. The Supabase CLI's stored login (`~/.supabase/access-token`)
//!   3. Browser OAuth (authorization-code + PKCE against api.supabase.com)
//!      when an OAuth app client id is configured (`KYMA_SUPABASE_OAUTH_CLIENT_ID`)
//!   4. Guided manual paste (prints the dashboard URL that mints a token)
//!
//! Workspaces live in `~/.kyma/deploy/<name>/` — the embedded IaC templates
//! are materialized there, alongside the rendered `terraform.tfvars` (0600)
//! and a small `deploy.json` describing the deployment.

use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, Stdio};

const DEFAULT_SUPABASE_API: &str = "https://api.supabase.com";
const DEFAULT_IMAGE_REPO: &str = "ghcr.io/shakedaskayo/kyma-engine";

// ── argument parsing ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum IacTool {
    Terraform,
    Pulumi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Target {
    /// Full production stack on AWS Fargate + S3 + Supabase.
    Aws,
    /// Local engine container wired to a real Supabase project (test drive).
    Local,
}

// ── orthogonal deployment axes ───────────────────────────────────────────────
// The engine is backend-agnostic; these four independent selectors are what the
// wizard, the Terraform stack, and the Helm chart switch on. A `validate_combo`
// gate (below) rejects invalid corners with an explanatory message.

/// Where the engine container runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Compute {
    /// AWS ECS Fargate (Terraform/Pulumi). Default.
    Fargate,
    /// AWS EKS — Terraform provisions the cluster and installs the Helm chart.
    Eks,
    /// Any existing Kubernetes cluster via the Helm chart (BYO kubectl context).
    Helm,
    /// Engine container run locally with docker (test drive).
    Local,
}

/// Catalog Postgres provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Database {
    /// Provision a Supabase project (catalog + optional Auth + optional Storage).
    Supabase,
    /// Provision an AWS RDS Postgres instance in the stack VPC.
    Rds,
    /// Bring your own Postgres — a `postgresql://` URL you supply.
    External,
}

/// Columnar-extents object store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Storage {
    /// Native AWS S3 bucket — keyless via the task/pod IAM role.
    S3,
    /// Supabase Storage via its S3-compatible endpoint.
    Supabase,
    /// Bring your own S3-compatible store (MinIO / R2 / GCS-interop): endpoint + keys.
    External,
}

/// Sign-in backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Auth {
    /// Supabase Auth (only with `database = supabase`).
    Supabase,
    /// Static admin API token minted by the wizard (`KYMA_AUTH_TOKENS`).
    Token,
    /// OIDC issuer + client id (validated via JWKS).
    Oidc,
}

impl Compute {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fargate => "fargate",
            Self::Eks => "eks",
            Self::Helm => "helm",
            Self::Local => "local",
        }
    }
    fn from_arg(s: &str) -> Result<Self> {
        match s {
            "fargate" => Ok(Self::Fargate),
            "eks" => Ok(Self::Eks),
            "helm" => Ok(Self::Helm),
            "local" => Ok(Self::Local),
            o => bail!("unknown compute backend {o:?} (expected fargate|eks|helm|local)"),
        }
    }
    /// Map the deprecated `--target aws|local` flag onto a compute backend.
    fn from_target(s: &str) -> Option<Self> {
        match s {
            "aws" => Some(Self::Fargate),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
    fn is_aws(self) -> bool {
        matches!(self, Self::Fargate | Self::Eks)
    }
    fn is_k8s(self) -> bool {
        matches!(self, Self::Eks | Self::Helm)
    }
}

impl Database {
    fn as_str(self) -> &'static str {
        match self {
            Self::Supabase => "supabase",
            Self::Rds => "rds",
            Self::External => "external",
        }
    }
    fn from_arg(s: &str) -> Result<Self> {
        match s {
            "supabase" => Ok(Self::Supabase),
            "rds" => Ok(Self::Rds),
            "external" => Ok(Self::External),
            o => bail!("unknown database backend {o:?} (expected supabase|rds|external)"),
        }
    }
}

impl Storage {
    fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Supabase => "supabase",
            Self::External => "external",
        }
    }
    fn from_arg(s: &str) -> Result<Self> {
        match s {
            "s3" => Ok(Self::S3),
            "supabase" => Ok(Self::Supabase),
            "external" => Ok(Self::External),
            o => bail!("unknown storage backend {o:?} (expected s3|supabase|external)"),
        }
    }
}

impl Auth {
    fn as_str(self) -> &'static str {
        match self {
            Self::Supabase => "supabase",
            Self::Token => "token",
            Self::Oidc => "oidc",
        }
    }
    fn from_arg(s: &str) -> Result<Self> {
        match s {
            "supabase" => Ok(Self::Supabase),
            "token" => Ok(Self::Token),
            "oidc" => Ok(Self::Oidc),
            o => bail!("unknown auth backend {o:?} (expected supabase|token|oidc)"),
        }
    }
}

/// Reject invalid axis combinations with an explanatory message and the nearest
/// valid alternative. Smart defaults live in [`default_storage`]/[`default_auth`].
fn validate_combo(c: Compute, d: Database, s: Storage, a: Auth) -> Result<()> {
    if s == Storage::S3 && !c.is_aws() {
        bail!(
            "storage=s3 (native AWS S3, keyless via the task/pod IAM role) requires an AWS compute \
             target (fargate or eks). Use storage=external with an endpoint + keys for a non-AWS \
             S3-compatible store, or switch compute to fargate/eks."
        );
    }
    if d == Database::Rds && !c.is_aws() {
        bail!(
            "database=rds is provisioned inside the stack VPC and requires compute=fargate or eks. \
             Use database=external with a postgresql:// URL on this compute target instead."
        );
    }
    if a == Auth::Supabase && d != Database::Supabase {
        bail!(
            "auth=supabase requires database=supabase (Supabase Auth is tied to the Supabase \
             project). Use auth=token (a minted admin token) or auth=oidc instead."
        );
    }
    if s == Storage::Supabase && d != Database::Supabase {
        bail!(
            "storage=supabase needs a Supabase project; either set database=supabase, or choose \
             storage=s3 (AWS) / storage=external. A storage-only Supabase project is not \
             auto-provisioned in this version."
        );
    }
    Ok(())
}

/// Default storage backend for a (compute, database) pair: keep everything in
/// Supabase when the DB is Supabase, else native S3 on AWS, else BYO.
fn default_storage(c: Compute, d: Database) -> Storage {
    if d == Database::Supabase {
        Storage::Supabase
    } else if c.is_aws() {
        Storage::S3
    } else {
        Storage::External
    }
}

/// Default auth backend follows the database: Supabase Auth with Supabase, else
/// a minted admin token.
fn default_auth(d: Database) -> Auth {
    if d == Database::Supabase {
        Auth::Supabase
    } else {
        Auth::Token
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Op {
    /// Wizard: collect credentials + settings, materialize the IaC workspace.
    Init {
        /// Deployment name (workspace at ~/.kyma/deploy/<name>).
        #[arg(long, default_value = "prod")]
        name: String,
        /// aws (production) or local (Supabase-backed test drive).
        #[arg(long, value_enum, default_value = "aws")]
        target: Target,
        /// IaC tool for the aws target.
        #[arg(long, value_enum, default_value = "terraform")]
        tool: IacTool,
        /// AWS region.
        #[arg(long)]
        region: Option<String>,
        /// Supabase organization id (skips the interactive picker).
        #[arg(long)]
        supabase_org: Option<String>,
        /// Custom domain for the engine (aws target).
        #[arg(long)]
        domain: Option<String>,
        /// Email(s) granted the kyma admin role (comma-separated).
        #[arg(long)]
        admin_email: Option<String>,
        /// Answer prompts with defaults (requires --supabase-org + a token source).
        #[arg(long)]
        yes: bool,
        /// Render the workspace + print the planned commands, run nothing.
        #[arg(long = "print-only")]
        print_only: bool,
        /// Overwrite an existing workspace's rendered config.
        #[arg(long)]
        force: bool,
    },
    /// Provision: terraform/pulumi apply (aws) or docker run (local).
    Up {
        #[arg(long, default_value = "prod")]
        name: String,
        /// Skip the IaC tool's interactive approval.
        #[arg(long)]
        auto_approve: bool,
    },
    /// Show deployment outputs and probe the engine's /health.
    Status {
        #[arg(long, default_value = "prod")]
        name: String,
    },
    /// Tear everything down (terraform/pulumi destroy, or docker rm + project delete).
    Destroy {
        #[arg(long, default_value = "prod")]
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

// ── embedded IaC templates ───────────────────────────────────────────────────

/// (relative path in the workspace, contents). Mirrors `deploy/` in the repo;
/// embedding keeps templates in lockstep with the CLI version and lets the
/// standalone binary work without a checkout.
const DEPLOY_FILES: &[(&str, &str)] = &[
    (
        "terraform/main.tf",
        include_str!("../../../deploy/terraform/main.tf"),
    ),
    (
        "terraform/variables.tf",
        include_str!("../../../deploy/terraform/variables.tf"),
    ),
    (
        "terraform/outputs.tf",
        include_str!("../../../deploy/terraform/outputs.tf"),
    ),
    (
        "terraform/versions.tf",
        include_str!("../../../deploy/terraform/versions.tf"),
    ),
    (
        "terraform/backend.tf",
        include_str!("../../../deploy/terraform/backend.tf"),
    ),
    (
        "terraform/terraform.tfvars.example",
        include_str!("../../../deploy/terraform/terraform.tfvars.example"),
    ),
    (
        "terraform/stack/main.tf",
        include_str!("../../../deploy/terraform/stack/main.tf"),
    ),
    (
        "terraform/stack/variables.tf",
        include_str!("../../../deploy/terraform/stack/variables.tf"),
    ),
    (
        "terraform/stack/outputs.tf",
        include_str!("../../../deploy/terraform/stack/outputs.tf"),
    ),
    (
        "terraform/stack/versions.tf",
        include_str!("../../../deploy/terraform/stack/versions.tf"),
    ),
    (
        "terraform/stack/modules/network/main.tf",
        include_str!("../../../deploy/terraform/stack/modules/network/main.tf"),
    ),
    (
        "terraform/stack/modules/network/variables.tf",
        include_str!("../../../deploy/terraform/stack/modules/network/variables.tf"),
    ),
    (
        "terraform/stack/modules/network/outputs.tf",
        include_str!("../../../deploy/terraform/stack/modules/network/outputs.tf"),
    ),
    (
        "terraform/stack/modules/network/versions.tf",
        include_str!("../../../deploy/terraform/stack/modules/network/versions.tf"),
    ),
    (
        "terraform/stack/modules/storage/main.tf",
        include_str!("../../../deploy/terraform/stack/modules/storage/main.tf"),
    ),
    (
        "terraform/stack/modules/storage/variables.tf",
        include_str!("../../../deploy/terraform/stack/modules/storage/variables.tf"),
    ),
    (
        "terraform/stack/modules/storage/outputs.tf",
        include_str!("../../../deploy/terraform/stack/modules/storage/outputs.tf"),
    ),
    (
        "terraform/stack/modules/storage/versions.tf",
        include_str!("../../../deploy/terraform/stack/modules/storage/versions.tf"),
    ),
    (
        "terraform/stack/modules/supabase/main.tf",
        include_str!("../../../deploy/terraform/stack/modules/supabase/main.tf"),
    ),
    (
        "terraform/stack/modules/supabase/variables.tf",
        include_str!("../../../deploy/terraform/stack/modules/supabase/variables.tf"),
    ),
    (
        "terraform/stack/modules/supabase/outputs.tf",
        include_str!("../../../deploy/terraform/stack/modules/supabase/outputs.tf"),
    ),
    (
        "terraform/stack/modules/supabase/versions.tf",
        include_str!("../../../deploy/terraform/stack/modules/supabase/versions.tf"),
    ),
    (
        "terraform/stack/modules/secrets/main.tf",
        include_str!("../../../deploy/terraform/stack/modules/secrets/main.tf"),
    ),
    (
        "terraform/stack/modules/secrets/variables.tf",
        include_str!("../../../deploy/terraform/stack/modules/secrets/variables.tf"),
    ),
    (
        "terraform/stack/modules/secrets/outputs.tf",
        include_str!("../../../deploy/terraform/stack/modules/secrets/outputs.tf"),
    ),
    (
        "terraform/stack/modules/secrets/versions.tf",
        include_str!("../../../deploy/terraform/stack/modules/secrets/versions.tf"),
    ),
    (
        "terraform/stack/modules/ecs-service/main.tf",
        include_str!("../../../deploy/terraform/stack/modules/ecs-service/main.tf"),
    ),
    (
        "terraform/stack/modules/ecs-service/variables.tf",
        include_str!("../../../deploy/terraform/stack/modules/ecs-service/variables.tf"),
    ),
    (
        "terraform/stack/modules/ecs-service/outputs.tf",
        include_str!("../../../deploy/terraform/stack/modules/ecs-service/outputs.tf"),
    ),
    (
        "terraform/stack/modules/ecs-service/versions.tf",
        include_str!("../../../deploy/terraform/stack/modules/ecs-service/versions.tf"),
    ),
    (
        "pulumi/typescript/Pulumi.yaml",
        include_str!("../../../deploy/pulumi/typescript/Pulumi.yaml"),
    ),
    (
        "pulumi/typescript/index.ts",
        include_str!("../../../deploy/pulumi/typescript/index.ts"),
    ),
    (
        "pulumi/typescript/package.json",
        include_str!("../../../deploy/pulumi/typescript/package.json"),
    ),
    (
        "pulumi/typescript/tsconfig.json",
        include_str!("../../../deploy/pulumi/typescript/tsconfig.json"),
    ),
];

// ── workspace state ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
struct DeployState {
    target: String,
    tool: String,
    project_name: String,
    aws_region: String,
    image_tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engine_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supabase_project_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    container_name: Option<String>,
}

fn workspace_dir(name: &str) -> Result<PathBuf> {
    Ok(crate::client::config_dir()?.join("deploy").join(name))
}

fn load_state(dir: &Path) -> Result<DeployState> {
    let p = dir.join("deploy.json");
    let raw = std::fs::read_to_string(&p)
        .with_context(|| format!("read {} — run `kyma deploy init` first", p.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

fn save_state(dir: &Path, state: &DeployState) -> Result<()> {
    std::fs::write(dir.join("deploy.json"), serde_json::to_string_pretty(state)?)?;
    Ok(())
}

/// Materialize the embedded IaC templates into `dir`. Never touches
/// `terraform.tfvars`, `local.env`, state files, or anything not in the
/// template list — re-running `init` refreshes templates only.
fn materialize(dir: &Path) -> Result<()> {
    for (rel, contents) in DEPLOY_FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ── wizard answers + rendering ───────────────────────────────────────────────

/// Bring-your-own S3-compatible object store (MinIO / R2 / GCS-interop).
#[derive(Debug, Clone)]
struct ExternalStorage {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    path_style: bool,
}

#[derive(Debug, Clone)]
struct Answers {
    name: String,
    project_name: String,
    compute: Compute,
    database: Database,
    storage: Storage,
    auth: Auth,
    aws_region: String,
    image_tag: String,
    domain: String,
    route53_zone_id: String,
    // Supabase (used when database == Supabase || storage == Supabase)
    supabase_org_id: String,
    supabase_region: String,
    supabase_db_password: String,
    supabase_s3_access_key_id: String,
    supabase_s3_secret_access_key: String,
    // Populated once a Supabase project is provisioned (helm/local supabase paths).
    supabase_url: String,
    supabase_anon_key: String,
    // Bring-your-own Postgres (External)
    database_url: String,
    // Bring-your-own S3-compatible (External)
    external_storage: Option<ExternalStorage>,
    // Auth
    admin_emails: Vec<String>,
    allowed_email_domains: Vec<String>,
    oauth_providers: Vec<String>,
    admin_token: String,
    oidc_issuer: String,
    oidc_client_id: String,
    // Kubernetes (Helm target)
    kube_context: String,
    ingress_host: String,
}

impl Answers {
    fn use_supabase(&self) -> bool {
        self.database == Database::Supabase || self.storage == Storage::Supabase
    }
}

fn hcl_string_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Render terraform.tfvars from the wizard answers. Contains secrets (DB
/// password, BYO URL, storage keys, admin token) — written 0600, never
/// committed (workspace is outside any repo).
fn render_tfvars(a: &Answers) -> String {
    let ext = a.external_storage.clone().unwrap_or(ExternalStorage {
        endpoint: String::new(),
        bucket: "kyma".into(),
        region: a.aws_region.clone(),
        access_key_id: String::new(),
        secret_access_key: String::new(),
        path_style: true,
    });
    // `{:<21}` aligns `=` for the common block (longest key = 21 chars).
    let kv = |k: &str, v: &str| format!("{k:<21} = \"{v}\"\n");
    let raw = |k: &str, v: &str| format!("{k:<21} = {v}\n");
    let mut s =
        String::from("# Generated by `kyma deploy init` — edit freely; `init --force` regenerates.\n");
    s.push_str(&kv("project_name", &a.project_name));
    s.push_str(&kv("aws_region", &a.aws_region));
    s.push_str(&kv("compute_backend", a.compute.as_str()));
    s.push_str(&kv("database_backend", a.database.as_str()));
    s.push_str(&kv("storage_backend", a.storage.as_str()));
    s.push_str(&kv("auth_backend", a.auth.as_str()));
    s.push_str(&kv("image_tag", &a.image_tag));
    s.push_str(&kv("domain", &a.domain));
    s.push_str(&kv("route53_zone_id", &a.route53_zone_id));
    s.push_str(&kv("ingress_host", &a.ingress_host));
    s.push_str(&raw("admin_emails", &hcl_string_list(&a.admin_emails)));
    s.push_str(&raw("allowed_email_domains", &hcl_string_list(&a.allowed_email_domains)));
    s.push_str(&raw("oauth_providers", &hcl_string_list(&a.oauth_providers)));
    s.push_str(&kv("admin_token", &a.admin_token));
    s.push_str(&kv("oidc_issuer", &a.oidc_issuer));
    s.push_str(&kv("oidc_client_id", &a.oidc_client_id));
    s.push_str(&kv("database_url", &a.database_url));
    s.push_str(&kv("supabase_org_id", &a.supabase_org_id));
    s.push_str(&kv("supabase_region", &a.supabase_region));
    s.push_str(&kv("supabase_db_password", &a.supabase_db_password));
    s.push_str(&kv("storage_endpoint", &ext.endpoint));
    s.push_str(&kv("storage_bucket", &ext.bucket));
    s.push_str(&kv("storage_region", &ext.region));
    s.push_str(&kv("storage_access_key", &ext.access_key_id));
    s.push_str(&kv("storage_secret", &ext.secret_access_key));
    s.push_str(&raw("storage_path_style", &ext.path_style.to_string()));
    // The Supabase Storage S3 keys keep their longer field names (own column).
    s.push_str(&format!(
        "{:<29} = \"{}\"\n",
        "supabase_s3_access_key_id", a.supabase_s3_access_key_id
    ));
    s.push_str(&format!(
        "{:<29} = \"{}\"\n",
        "supabase_s3_secret_access_key", a.supabase_s3_secret_access_key
    ));
    s
}

/// The engine's runtime environment derived from the chosen axes, split into
/// non-secret (`env`) and secret (`secretEnv`) maps. Shared by the Helm values
/// renderer and the local-env renderer so backend wiring stays in one place.
/// (The Terraform stack computes the equivalent env in HCL; see `stack/main.tf`.)
fn engine_env(a: &Answers) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut env: Vec<(String, String)> = vec![
        ("KYMA_HTTP_ADDR".into(), "0.0.0.0:8080".into()),
        ("KYMA_GRPC_ADDR".into(), "off".into()),
        ("KYMA_OTLP_ADDR".into(), "off".into()),
        ("KYMA_AUTH_BACKEND".into(), a.auth.as_str().into()),
    ];
    let mut secret: Vec<(String, String)> = Vec::new();

    // Catalog Postgres URL is always a secret.
    if !a.database_url.is_empty() {
        secret.push(("KYMA_CATALOG_URL".into(), a.database_url.clone()));
    }

    match a.auth {
        Auth::Supabase => {
            if !a.supabase_url.is_empty() {
                env.push(("KYMA_SUPABASE_URL".into(), a.supabase_url.clone()));
            }
            if !a.supabase_anon_key.is_empty() {
                env.push(("KYMA_SUPABASE_ANON_KEY".into(), a.supabase_anon_key.clone()));
            }
            if !a.oauth_providers.is_empty() {
                env.push(("KYMA_SUPABASE_PROVIDERS".into(), a.oauth_providers.join(",")));
            }
            if !a.admin_emails.is_empty() {
                env.push(("KYMA_ADMIN_EMAILS".into(), a.admin_emails.join(",")));
            }
            if !a.allowed_email_domains.is_empty() {
                env.push((
                    "KYMA_ALLOWED_EMAIL_DOMAINS".into(),
                    a.allowed_email_domains.join(","),
                ));
            }
        }
        Auth::Token => {
            secret.push(("KYMA_AUTH_TOKENS".into(), format!("{}:admin", a.admin_token)));
        }
        Auth::Oidc => {
            env.push(("KYMA_OIDC_ISSUER".into(), a.oidc_issuer.clone()));
            env.push(("KYMA_OIDC_CLIENT_ID".into(), a.oidc_client_id.clone()));
        }
    }

    match a.storage {
        Storage::S3 => {
            // Native AWS S3: keyless via the task/pod IAM role; bucket+region are
            // injected by the IaC (TF stack / EKS module), not here.
            env.push(("KYMA_S3_REGION".into(), a.aws_region.clone()));
            env.push(("KYMA_S3_PATH_STYLE".into(), "false".into()));
            env.push(("KYMA_S3_ALLOW_HTTP".into(), "false".into()));
        }
        Storage::Supabase => {
            env.push(("KYMA_S3_BUCKET".into(), "kyma".into()));
            env.push(("KYMA_S3_PATH_STYLE".into(), "true".into()));
            env.push(("KYMA_S3_ALLOW_HTTP".into(), "false".into()));
            if !a.supabase_s3_access_key_id.is_empty() {
                secret.push((
                    "KYMA_S3_ACCESS_KEY_ID".into(),
                    a.supabase_s3_access_key_id.clone(),
                ));
                secret.push((
                    "KYMA_S3_SECRET_ACCESS_KEY".into(),
                    a.supabase_s3_secret_access_key.clone(),
                ));
            }
        }
        Storage::External => {
            if let Some(es) = &a.external_storage {
                env.push(("KYMA_S3_ENDPOINT".into(), es.endpoint.clone()));
                env.push(("KYMA_S3_BUCKET".into(), es.bucket.clone()));
                env.push(("KYMA_S3_REGION".into(), es.region.clone()));
                env.push(("KYMA_S3_PATH_STYLE".into(), es.path_style.to_string()));
                env.push(("KYMA_S3_ALLOW_HTTP".into(), "false".into()));
                secret.push(("KYMA_S3_ACCESS_KEY_ID".into(), es.access_key_id.clone()));
                secret.push((
                    "KYMA_S3_SECRET_ACCESS_KEY".into(),
                    es.secret_access_key.clone(),
                ));
            }
        }
    }
    (env, secret)
}

/// Minimal YAML double-quoted scalar (escapes backslash + quote). Env values are
/// always quoted so k8s sees strings and values with `:`/`@` stay literal.
fn yaml_quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Render Helm `values.yaml` for the `helm` compute target.
fn render_helm_values(a: &Answers) -> String {
    let (env, secret) = engine_env(a);
    let ingress_enabled = !a.ingress_host.is_empty();
    let mut s = String::from(
        "# Generated by `kyma deploy init` (compute=helm). Contains secrets — keep private.\n",
    );
    s.push_str(&format!(
        "image:\n  repository: {}\n  tag: {}\n  pullPolicy: IfNotPresent\n",
        DEFAULT_IMAGE_REPO, a.image_tag
    ));
    // The engine is single-writer per catalog — keep one replica.
    s.push_str("replicaCount: 1\n");
    s.push_str("service:\n  type: ClusterIP\n  port: 8080\n");
    s.push_str(&format!(
        "ingress:\n  enabled: {}\n  className: \"\"\n  host: {}\n  tls: {}\n  annotations: {{}}\n",
        ingress_enabled,
        yaml_quote(&a.ingress_host),
        ingress_enabled
    ));
    // serviceAccount.annotations is where the EKS module injects the IRSA role ARN.
    s.push_str("serviceAccount:\n  create: true\n  name: kyma-engine\n  annotations: {}\n");
    s.push_str("resources: {}\n");
    s.push_str("env:\n");
    for (k, v) in &env {
        s.push_str(&format!("  {}: {}\n", k, yaml_quote(v)));
    }
    s.push_str("secretEnv:\n");
    for (k, v) in &secret {
        s.push_str(&format!("  {}: {}\n", k, yaml_quote(v)));
    }
    s
}

/// Render `local.env` for the docker target from full Answers — covers the BYO
/// Postgres / external storage / token / OIDC combinations. The Supabase-
/// provisioned path uses [`render_local_env`].
fn render_local_env_from(a: &Answers, secret_key: &str) -> String {
    let (env, secret) = engine_env(a);
    let mut s = String::from(
        "# Generated by `kyma deploy init` (compute=local). Contains secrets — keep private.\n",
    );
    s.push_str(&format!("KYMA_SECRET_KEY={secret_key}\n"));
    for (k, v) in env.iter().chain(secret.iter()) {
        s.push_str(&format!("{k}={v}\n"));
    }
    s
}

/// Supabase Storage S3-protocol credentials (extents object store).
#[derive(Debug, Clone)]
struct SupabaseS3 {
    endpoint: String,
    region: String,
    bucket: String,
    access_key_id: String,
    secret_access_key: String,
}

/// Render the env file for the local (docker) target: Supabase catalog +
/// Supabase Auth. Extents default to **Supabase Storage** (S3 protocol)
/// when credentials were provisioned; otherwise the container volume, with
/// the S3 lines included commented-out as the upgrade path.
fn render_local_env(
    db_url: &str,
    supabase_url: &str,
    anon_key: &str,
    admin_emails: &[String],
    secret_key: &str,
    s3: Option<&SupabaseS3>,
) -> String {
    let storage = match s3 {
        Some(s3) => format!(
            r#"# Extents live in Supabase Storage via its S3-compatible endpoint.
KYMA_S3_ENDPOINT={endpoint}
KYMA_S3_BUCKET={bucket}
KYMA_S3_REGION={region}
KYMA_S3_ACCESS_KEY_ID={ak}
KYMA_S3_SECRET_ACCESS_KEY={sk}
KYMA_S3_PATH_STYLE=true
KYMA_S3_ALLOW_HTTP=false"#,
            endpoint = s3.endpoint,
            bucket = s3.bucket,
            region = s3.region,
            ak = s3.access_key_id,
            sk = s3.secret_access_key,
        ),
        None => r#"# Extents stay on the container volume (KYMA_LOCAL_DATA). To use Supabase
# Storage's S3-compatible endpoint instead, create S3 access keys in the
# dashboard (Project Settings → Storage) and fill in:
# KYMA_S3_ENDPOINT=https://<ref>.storage.supabase.co/storage/v1/s3
# KYMA_S3_BUCKET=kyma
# KYMA_S3_REGION=<project-region>
# KYMA_S3_ACCESS_KEY_ID=
# KYMA_S3_SECRET_ACCESS_KEY=
# KYMA_S3_PATH_STYLE=true"#
            .to_string(),
    };
    format!(
        r#"# Generated by `kyma deploy init --target local`. Contains secrets — keep private.
KYMA_CATALOG_URL={db_url}
KYMA_AUTH_BACKEND=supabase
KYMA_SUPABASE_URL={supabase_url}
KYMA_SUPABASE_ANON_KEY={anon_key}
KYMA_ADMIN_EMAILS={admin_emails}
KYMA_SECRET_KEY={secret_key}
KYMA_HTTP_ADDR=0.0.0.0:8080
KYMA_GRPC_ADDR=off
KYMA_OTLP_ADDR=off
{storage}
"#,
        db_url = db_url,
        supabase_url = supabase_url,
        anon_key = anon_key,
        admin_emails = admin_emails.join(","),
        secret_key = secret_key,
        storage = storage,
    )
}

fn random_token(len: usize) -> String {
    // Alphanumeric only: safe inside connection strings and env files.
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = vec![0u8; len];
    getrandom_fill(&mut bytes);
    bytes.iter().map(|b| CHARS[(*b as usize) % CHARS.len()] as char).collect()
}

/// CSPRNG fill via the `rand` crate (thread_rng → OS entropy).
fn getrandom_fill(buf: &mut [u8]) {
    use rand::RngCore;
    rand::thread_rng().fill_bytes(buf);
}

// ── prompting (stderr, so stdout stays scriptable) ───────────────────────────

fn prompt(question: &str, default: &str) -> Result<String> {
    let mut err = std::io::stderr();
    if default.is_empty() {
        write!(err, "{question}: ")?;
    } else {
        write!(err, "{question} [{default}]: ")?;
    }
    err.flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() { default.to_string() } else { trimmed.to_string() })
}

fn confirm(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    let answer = prompt(&format!("{question} ({hint})"), "")?;
    Ok(match answer.to_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    })
}

fn note(msg: &str) {
    eprintln!("{msg}");
}

// ── Supabase access-token acquisition ────────────────────────────────────────

fn supabase_api_base() -> String {
    std::env::var("KYMA_SUPABASE_API_BASE").unwrap_or_else(|_| DEFAULT_SUPABASE_API.to_string())
}

/// OAuth app registration used for the browser flow. Resolution order:
/// `KYMA_SUPABASE_OAUTH_CLIENT_ID`/`_SECRET` env vars, then
/// `~/.kyma/oauth-app.json` ({"client_id": …, "client_secret": …}).
#[derive(Debug, Clone)]
struct OAuthAppConfig {
    client_id: String,
    client_secret: Option<String>,
}

fn oauth_app_config() -> Option<OAuthAppConfig> {
    if let Ok(client_id) = std::env::var("KYMA_SUPABASE_OAUTH_CLIENT_ID") {
        if !client_id.is_empty() {
            return Some(OAuthAppConfig {
                client_id,
                client_secret: std::env::var("KYMA_SUPABASE_OAUTH_CLIENT_SECRET")
                    .ok()
                    .filter(|s| !s.is_empty()),
            });
        }
    }
    let path = crate::client::config_dir().ok()?.join("oauth-app.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let client_id = v.get("client_id")?.as_str()?.to_string();
    Some(OAuthAppConfig {
        client_id,
        client_secret: v
            .get("client_secret")
            .and_then(|x| x.as_str())
            .map(String::from),
    })
}

/// The Supabase CLI stores its login token at `~/.supabase/access-token`.
fn supabase_cli_token_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".supabase").join("access-token"))
}

fn read_supabase_cli_token() -> Option<String> {
    let p = supabase_cli_token_path()?;
    let raw = std::fs::read_to_string(p).ok()?;
    let tok = raw.trim().to_string();
    (!tok.is_empty()).then_some(tok)
}

/// Resolve a Supabase Management-API access token, interactively if allowed.
async fn resolve_supabase_token(interactive: bool) -> Result<String> {
    if let Ok(tok) = std::env::var("SUPABASE_ACCESS_TOKEN") {
        if !tok.trim().is_empty() {
            note("• Supabase token: using SUPABASE_ACCESS_TOKEN from the environment");
            return Ok(tok.trim().to_string());
        }
    }
    if let Some(tok) = read_supabase_cli_token() {
        note("• Supabase token: reusing your `supabase login` session (~/.supabase/access-token)");
        return Ok(tok);
    }
    if let Some(app) = oauth_app_config() {
        if interactive {
            note("• Supabase token: starting browser OAuth flow…");
            match oauth_flow(&app).await {
                Ok(tok) => return Ok(tok),
                Err(e) => note(&format!("  OAuth flow failed ({e}); falling back to manual entry")),
            }
        }
    }
    if !interactive {
        bail!(
            "no Supabase access token found — set SUPABASE_ACCESS_TOKEN, run `supabase login`, \
             or rerun without --yes for the interactive flow"
        );
    }
    note("• Supabase token: create one at https://supabase.com/dashboard/account/tokens");
    let tok = rpassword::prompt_password("  Paste your Supabase access token: ")
        .context("read token from terminal")?;
    let tok = tok.trim().to_string();
    if tok.is_empty() {
        bail!("empty token");
    }
    Ok(tok)
}

/// Browser OAuth (authorization-code + PKCE) against the Supabase Management
/// API. Requires a registered OAuth app (client id); listens on a localhost
/// callback, opens the browser, exchanges the code for an access token.
async fn oauth_flow(app: &OAuthAppConfig) -> Result<String> {
    use sha2::{Digest, Sha256};

    let client_id = app.client_id.as_str();
    let verifier = random_token(64);
    let challenge = {
        let digest = Sha256::digest(verifier.as_bytes());
        base64_url(&digest)
    };
    let state = random_token(24);

    // Fixed port: OAuth app registrations need a static redirect URI
    // (registered as http://localhost:53682/callback).
    let listener = std::net::TcpListener::bind("127.0.0.1:53682")
        .context("bind callback port 53682 (is another kyma deploy running?)")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{port}/callback");

    let base = supabase_api_base();
    let authorize_url = format!(
        "{base}/v1/oauth/authorize?client_id={client_id}&redirect_uri={redirect}&response_type=code&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        redirect = urlencode(&redirect_uri),
    );

    note(&format!("  Opening {authorize_url}"));
    let _ = open_browser(&authorize_url);
    note("  Waiting for the browser callback… (Ctrl-C to abort)");

    // One-shot callback accept, in a blocking task with a generous timeout.
    let expected_state = state.clone();
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        tokio::task::spawn_blocking(move || -> Result<String> {
            let (mut sock, _) = listener.accept()?;
            let mut reader = BufReader::new(&mut sock);
            let mut request_line = String::new();
            reader.read_line(&mut request_line)?;
            // GET /callback?code=…&state=… HTTP/1.1
            let query = request_line
                .split_whitespace()
                .nth(1)
                .and_then(|p| p.split_once('?'))
                .map(|(_, q)| q.to_string())
                .unwrap_or_default();
            let mut code = None;
            let mut got_state = None;
            for pair in query.split('&') {
                match pair.split_once('=') {
                    Some(("code", v)) => code = Some(v.to_string()),
                    Some(("state", v)) => got_state = Some(v.to_string()),
                    _ => {}
                }
            }
            let body = "<html><body><h3>kyma: authorization received — you can close this tab.</h3></body></html>";
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            if got_state.as_deref() != Some(expected_state.as_str()) {
                bail!("OAuth state mismatch");
            }
            code.ok_or_else(|| anyhow!("no code in OAuth callback"))
        }),
    )
    .await
    .context("timed out waiting for the OAuth callback")???;

    let client = reqwest::Client::new();
    let mut req = client.post(format!("{base}/v1/oauth/token")).form(&[
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect_uri),
    ]);
    // Confidential-client registrations also require the secret.
    if let Some(secret) = &app.client_secret {
        req = req.basic_auth(client_id, Some(secret));
    }
    let resp: serde_json::Value = req
        .send()
        .await?
        .error_for_status()
        .context("OAuth token exchange")?
        .json()
        .await?;
    resp.get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("token exchange response had no access_token"))
}

fn base64_url(bytes: &[u8]) -> String {
    // URL-safe base64 without padding (RFC 7636 code challenge).
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        }
    }
    out
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    Proc::new(opener).arg(url).spawn().context("open browser")?;
    Ok(())
}

// ── Supabase Management API (org picker + local-target provisioning) ────────

async fn supabase_get(token: &str, path: &str) -> Result<serde_json::Value> {
    let resp = reqwest::Client::new()
        .get(format!("{}{}", supabase_api_base(), path))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("GET {path}"))?;
    Ok(resp.json().await?)
}

async fn supabase_post(token: &str, path: &str, body: serde_json::Value) -> Result<serde_json::Value> {
    let resp = reqwest::Client::new()
        .post(format!("{}{}", supabase_api_base(), path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("POST {path}"))?;
    Ok(resp.json().await?)
}

/// Pull a session-mode pooler connection string out of a Management-API
/// response (shape varies: array of pooler configs, or mode→url map) and
/// substitute the dashboard's `[YOUR-PASSWORD]` placeholder. The direct
/// `db.<ref>` host is IPv6-only on current Supabase projects, so IPv4
/// networks must connect through the pooler.
fn extract_pooler_url(v: &serde_json::Value, password: &str) -> Option<String> {
    fn collect(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) if s.contains(".pooler.supabase.com") => {
                out.push(s.clone())
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| collect(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| collect(i, out)),
            _ => {}
        }
    }
    let mut urls = Vec::new();
    collect(v, &mut urls);
    // Session mode (port 5432) supports prepared statements — required by the
    // engine; transaction mode (6543) does not.
    let chosen = urls
        .iter()
        .find(|u| u.contains(":5432/"))
        .or_else(|| urls.first())?;
    Some(chosen.replace("[YOUR-PASSWORD]", password))
}

/// Fetch the pooler config for a project. Requires the `Database: Read`
/// OAuth scope (`database_pooling_config_read`); returns None when the
/// token lacks it or the endpoint is unavailable.
async fn fetch_pooler_url(token: &str, project_ref: &str, password: &str) -> Option<String> {
    let v = supabase_get(token, &format!("/v1/projects/{project_ref}/config/database/pooler"))
        .await
        .ok()?;
    extract_pooler_url(&v, password)
}

/// `[(id, name)]` of organizations the token can access.
async fn fetch_orgs(token: &str) -> Result<Vec<(String, String)>> {
    let v = supabase_get(token, "/v1/organizations").await?;
    let orgs = v
        .as_array()
        .ok_or_else(|| anyhow!("unexpected /v1/organizations response"))?
        .iter()
        .filter_map(|o| {
            Some((
                o.get("id")?.as_str()?.to_string(),
                o.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string(),
            ))
        })
        .collect();
    Ok(orgs)
}

/// Interactive org selection (single org auto-selects).
async fn pick_org(token: &str) -> Result<String> {
    let orgs = fetch_orgs(token).await.context("list Supabase organizations")?;
    match orgs.as_slice() {
        [] => bail!("this Supabase token has no organizations — create one in the dashboard first"),
        [(id, name)] => {
            note(&format!("• Supabase organization: {name} ({id})"));
            Ok(id.clone())
        }
        many => {
            note("• Choose a Supabase organization:");
            for (i, (id, name)) in many.iter().enumerate() {
                note(&format!("    {}. {name} ({id})", i + 1));
            }
            loop {
                let raw = prompt("  Organization #", "1")?;
                if let Ok(idx) = raw.parse::<usize>() {
                    if idx >= 1 && idx <= many.len() {
                        return Ok(many[idx - 1].0.clone());
                    }
                }
                note("  Enter a number from the list.");
            }
        }
    }
}

// ── local target: provision a Supabase project directly ─────────────────────

struct LocalProject {
    project_ref: String,
    db_url: String,
    supabase_url: String,
    anon_key: String,
    service_role_key: Option<String>,
    s3: Option<SupabaseS3>,
}

/// Try to mint Supabase Storage S3 access keys via the Management API.
/// The endpoint is not in the public docs — treat absence (4xx) as
/// "unsupported" and fall back gracefully.
async fn try_create_storage_keys(token: &str, project_ref: &str) -> Option<(String, String)> {
    let v = supabase_post(
        token,
        &format!("/v1/projects/{project_ref}/storage/credentials"),
        serde_json::json!({ "description": "kyma extents" }),
    )
    .await
    .ok()?;
    let ak = v
        .get("access_key")
        .or_else(|| v.get("access_key_id"))
        .and_then(|x| x.as_str())?
        .to_string();
    let sk = v
        .get("secret_key")
        .or_else(|| v.get("secret_access_key"))
        .and_then(|x| x.as_str())?
        .to_string();
    Some((ak, sk))
}

/// Create the extents bucket (private) via the project's storage API.
async fn ensure_bucket(supabase_url: &str, service_role: &str, bucket: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .post(format!("{supabase_url}/storage/v1/bucket"))
        .bearer_auth(service_role)
        .header("apikey", service_role)
        .json(&serde_json::json!({ "id": bucket, "name": bucket, "public": false }))
        .send()
        .await?;
    // 200/201 created; 400 with "already exists" is fine on re-runs.
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if body.contains("already exists") {
        return Ok(());
    }
    bail!("create bucket {bucket}: {status} {body}")
}

/// Interactive fallback for Supabase Storage S3 keys: Supabase has no public
/// API to mint them, so open the dashboard page and let the user paste the
/// pair. Returns None when the user skips (extents fall back to local disk).
fn prompt_storage_keys(project_ref: &str) -> Option<(String, String)> {
    let url = format!("https://supabase.com/dashboard/project/{project_ref}/storage/s3");
    note("");
    note("• Supabase Storage is the default extents store, but Supabase has no");
    note("  API to create S3 access keys — one manual step:");
    note(&format!("    1. Open {url}"));
    note("    2. \"New access key\" → copy both values (shown once)");
    let _ = open_browser(&url);
    let ak = prompt("  Access key ID (empty = skip, use local disk)", "").ok()?;
    if ak.trim().is_empty() {
        return None;
    }
    let sk = rpassword::prompt_password("  Secret access key: ").ok()?;
    if sk.trim().is_empty() {
        return None;
    }
    Some((ak.trim().to_string(), sk.trim().to_string()))
}

async fn provision_local_project(
    token: &str,
    name: &str,
    org_id: &str,
    region: &str,
    db_password: &str,
    interactive: bool,
) -> Result<LocalProject> {
    note(&format!("• Creating Supabase project '{name}' in {region}… (takes a few minutes)"));
    let created = supabase_post(
        token,
        "/v1/projects",
        serde_json::json!({
            "name": name,
            "organization_id": org_id,
            "region": region,
            "db_pass": db_password,
        }),
    )
    .await
    .context("create Supabase project")?;
    let project_ref = created
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("create-project response had no id"))?
        .to_string();

    // Poll until the project is healthy (status: ACTIVE_HEALTHY).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let p = supabase_get(token, &format!("/v1/projects/{project_ref}")).await?;
        let status = p.get("status").and_then(|s| s.as_str()).unwrap_or("");
        note(&format!("  …status: {status}"));
        if status == "ACTIVE_HEALTHY" {
            break;
        }
        if std::time::Instant::now() > deadline {
            bail!("Supabase project {project_ref} did not become healthy within 10 minutes");
        }
    }

    // anon + service_role keys from the api-keys listing.
    let keys = supabase_get(token, &format!("/v1/projects/{project_ref}/api-keys")).await?;
    let find_key = |name: &str| {
        keys.as_array()
            .into_iter()
            .flatten()
            .find(|k| k.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|k| k.get("api_key").and_then(|v| v.as_str()))
            .map(String::from)
    };
    let anon_key = find_key("anon").ok_or_else(|| anyhow!("could not find the anon api key"))?;
    let service_role_key = find_key("service_role");
    let supabase_url = format!("https://{project_ref}.supabase.co");

    // Supabase Storage as the extents store (default when Supabase is the
    // DB): try the Management API for S3 keys (undocumented — may not
    // exist), else walk the user through the dashboard. Skipping leaves
    // extents on the local volume.
    let keys = match try_create_storage_keys(token, &project_ref).await {
        Some(pair) => Some(pair),
        None if interactive => prompt_storage_keys(&project_ref),
        None => {
            note("• Extents: no S3-key API and non-interactive run — extents stay on the local volume (see local.env to wire keys manually)");
            None
        }
    };
    let mut s3 = None;
    if let Some((ak, sk)) = keys {
        let bucket = "kyma".to_string();
        let storage_ok = match &service_role_key {
            Some(sr) => match ensure_bucket(&supabase_url, sr, &bucket).await {
                Ok(()) => true,
                Err(e) => {
                    note(&format!("  could not create the storage bucket ({e}); extents stay on the local volume"));
                    false
                }
            },
            None => false,
        };
        if storage_ok {
            note("• Extents: Supabase Storage (S3 protocol, bucket 'kyma')");
            s3 = Some(SupabaseS3 {
                endpoint: format!("https://{project_ref}.storage.supabase.co/storage/v1/s3"),
                region: region.to_string(),
                bucket,
                access_key_id: ak,
                secret_access_key: sk,
            });
        }
    }

    // Prefer the IPv4-friendly session pooler; the direct host is IPv6-only.
    let db_url = match fetch_pooler_url(token, &project_ref, db_password).await {
        Some(url) => {
            note("• Catalog: connecting via the Supabase session pooler");
            url
        }
        None => {
            note("• Catalog: pooler config unreadable (token scope?) — using the direct host (requires IPv6)");
            format!("postgresql://postgres:{db_password}@db.{project_ref}.supabase.co:5432/postgres")
        }
    };

    Ok(LocalProject {
        db_url,
        supabase_url,
        anon_key,
        service_role_key,
        s3,
        project_ref,
    })
}

// ── external command helpers ─────────────────────────────────────────────────

fn have(binary: &str) -> bool {
    which_path(binary).is_some()
}

fn which_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(binary))
        .find(|p| p.is_file())
}

/// Run a command with inherited stdio (the user sees live output).
fn run_streamed(cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
    note(&format!("$ {program} {}", args.join(" ")));
    let status = Proc::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("run {program}"))?;
    if !status.success() {
        bail!("{program} {} failed ({status})", args.join(" "));
    }
    Ok(())
}

/// Run a command capturing stdout (for `terraform output -json` etc).
fn run_captured(cwd: &Path, program: &str, args: &[&str]) -> Result<String> {
    let out = Proc::new(program)
        .args(args)
        .current_dir(cwd)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("run {program}"))?;
    if !out.status.success() {
        bail!("{program} {} failed ({})", args.join(" "), out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn aws_credentials_present() -> bool {
    if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
    {
        return true;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let aws_dir = PathBuf::from(home).join(".aws");
        if aws_dir.join("credentials").exists() || aws_dir.join("config").exists() {
            return true;
        }
    }
    false
}

/// Resolve the engine image tag: the latest GitHub release (so image and
/// installed CLI track the same release train), falling back to `latest`.
async fn resolve_image_tag() -> String {
    match crate::update::latest_release_tag().await {
        Ok(tag) => tag,
        Err(_) => "latest".to_string(),
    }
}

// ── subcommand implementations ───────────────────────────────────────────────

pub(crate) async fn run(op: Op) -> Result<()> {
    match op {
        Op::Init {
            name,
            target,
            tool,
            region,
            supabase_org,
            domain,
            admin_email,
            yes,
            print_only,
            force,
        } => {
            cmd_init(
                &name, target, tool, region, supabase_org, domain, admin_email, yes, print_only,
                force,
            )
            .await
        }
        Op::Up { name, auto_approve } => cmd_up(&name, auto_approve).await,
        Op::Status { name } => cmd_status(&name).await,
        Op::Destroy { name, yes } => cmd_destroy(&name, yes).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn cmd_init(
    name: &str,
    target: Target,
    tool: IacTool,
    region: Option<String>,
    supabase_org: Option<String>,
    domain: Option<String>,
    admin_email: Option<String>,
    yes: bool,
    print_only: bool,
    force: bool,
) -> Result<()> {
    let interactive = !yes;
    let dir = workspace_dir(name)?;
    if dir.join("deploy.json").exists() && !force && !print_only {
        bail!(
            "workspace '{name}' already exists at {} — rerun with --force to regenerate, or use `kyma deploy up`",
            dir.display()
        );
    }

    // ── prereq checks ──
    note("kyma production deployment");
    note("");
    match target {
        Target::Aws => {
            let tool_bin = match tool {
                IacTool::Terraform => {
                    if have("terraform") {
                        "terraform"
                    } else if have("tofu") {
                        "tofu"
                    } else if print_only {
                        "terraform"
                    } else {
                        bail!(
                            "terraform (or tofu) not found — install from \
                             https://developer.hashicorp.com/terraform/install and rerun"
                        );
                    }
                }
                IacTool::Pulumi => {
                    if have("pulumi") {
                        "pulumi"
                    } else if print_only {
                        "pulumi"
                    } else {
                        bail!("pulumi not found — install from https://www.pulumi.com/docs/install/ and rerun");
                    }
                }
            };
            note(&format!("• IaC tool: {tool_bin}"));
            if !aws_credentials_present() && !print_only {
                bail!(
                    "no AWS credentials detected — run `aws configure` (or `aws sso login`), \
                     or export AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, then rerun"
                );
            }
            note("• AWS credentials: found");
        }
        Target::Local => {
            if !have("docker") && !print_only {
                bail!("docker not found — the local target runs the engine container with docker");
            }
            note("• docker: found");
        }
    }

    // ── credentials + answers ──
    let token = if print_only {
        std::env::var("SUPABASE_ACCESS_TOKEN").unwrap_or_else(|_| "sbp_PRINT_ONLY".into())
    } else {
        resolve_supabase_token(interactive).await?
    };

    let org_id = match supabase_org {
        Some(o) => o,
        None if print_only => "org-print-only".to_string(),
        None if interactive => pick_org(&token).await?,
        None => bail!("--supabase-org is required with --yes"),
    };

    let aws_region = match region {
        Some(r) => r,
        None if interactive && !print_only => prompt("AWS region", "us-east-1")?,
        None => "us-east-1".to_string(),
    };

    let domain = match domain {
        Some(d) => d,
        None if interactive && !print_only && target == Target::Aws => prompt(
            "Custom domain (empty = plain HTTP on the ALB DNS name)",
            "",
        )?,
        None => String::new(),
    };
    let route53_zone_id = if !domain.is_empty() && interactive && !print_only {
        prompt("Route53 zone id for that domain (empty = manual DNS validation)", "")?
    } else {
        String::new()
    };

    let admin_emails: Vec<String> = match admin_email {
        Some(e) => e.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        None if interactive && !print_only => {
            let raw = prompt("Admin email(s) (comma-separated — get the kyma admin role)", "")?;
            raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        }
        None => Vec::new(),
    };
    let allowed_email_domains: Vec<String> = admin_emails
        .iter()
        .filter_map(|e| e.rsplit_once('@').map(|(_, d)| d.to_lowercase()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let image_tag = resolve_image_tag().await;
    let db_password = random_token(24);

    // Extents storage: Supabase Storage is the default whenever Supabase is
    // the DB provider (always, in this stack); native S3 is the opt-out for
    // the fully-keyless AWS path. Keys are pasted later (after the project
    // exists) — `kyma deploy up` walks through it.
    // (The full interactive axis flow — db/storage/auth selection + BYO inputs
    // and the eks/helm targets — is layered on in cmd_init's Phase-5 rewrite.)
    let storage = if target == Target::Aws && interactive && !print_only {
        if confirm(
            "Store extents in Supabase Storage (default; one manual key-paste step) instead of native S3 (fully automated)?",
            true,
        )? {
            Storage::Supabase
        } else {
            Storage::S3
        }
    } else {
        Storage::Supabase
    };
    let compute = match target {
        Target::Aws => Compute::Fargate,
        Target::Local => Compute::Local,
    };

    let answers = Answers {
        name: name.to_string(),
        project_name: format!("kyma-{name}"),
        compute,
        database: Database::Supabase,
        storage,
        auth: Auth::Supabase,
        aws_region: aws_region.clone(),
        image_tag: image_tag.clone(),
        domain,
        route53_zone_id,
        supabase_org_id: org_id.clone(),
        supabase_region: aws_region.clone(),
        supabase_db_password: db_password.clone(),
        supabase_s3_access_key_id: String::new(),
        supabase_s3_secret_access_key: String::new(),
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        database_url: String::new(),
        external_storage: None,
        admin_emails: admin_emails.clone(),
        allowed_email_domains,
        oauth_providers: vec![],
        admin_token: String::new(),
        oidc_issuer: String::new(),
        oidc_client_id: String::new(),
        kube_context: String::new(),
        ingress_host: String::new(),
    };

    // ── materialize ──
    if print_only {
        note("");
        note("── print-only: rendered terraform.tfvars ──");
        println!("{}", render_tfvars(&answers));
        note("── planned commands ──");
        match target {
            Target::Aws => match tool {
                IacTool::Terraform => {
                    println!("cd {}/terraform && terraform init && terraform apply", dir.display());
                }
                IacTool::Pulumi => {
                    println!(
                        "cd {}/pulumi/typescript && pulumi package add terraform-module ../../terraform/stack kymaengine && pulumi up",
                        dir.display()
                    );
                }
            },
            Target::Local => {
                println!(
                    "docker run -d --name kyma-{name} --env-file {}/local.env -p 8080:8080 {DEFAULT_IMAGE_REPO}:{image_tag}",
                    dir.display()
                );
            }
        }
        return Ok(());
    }

    std::fs::create_dir_all(&dir)?;
    materialize(&dir)?;

    let mut state = DeployState {
        target: match target {
            Target::Aws => "aws".into(),
            Target::Local => "local".into(),
        },
        tool: match tool {
            IacTool::Terraform => "terraform".into(),
            IacTool::Pulumi => "pulumi".into(),
        },
        project_name: answers.project_name.clone(),
        aws_region,
        image_tag: image_tag.clone(),
        ..Default::default()
    };

    match target {
        Target::Aws => {
            let tfvars = dir.join("terraform").join("terraform.tfvars");
            if tfvars.exists() && !force {
                note("• terraform.tfvars exists — keeping it (use --force to regenerate)");
            } else {
                write_private(&tfvars, &render_tfvars(&answers))?;
                note(&format!("• Wrote {}", tfvars.display()));
            }
            // The Supabase provider reads SUPABASE_ACCESS_TOKEN; stash it for `up`.
            write_private(&dir.join("supabase-token"), &token)?;
            save_state(&dir, &state)?;
            note("");
            note(&format!("Workspace ready: {}", dir.display()));
            note("Next: kyma deploy up");
        }
        Target::Local => {
            // Provision the Supabase project right away (it's the only cloud
            // resource the local target needs).
            let project = provision_local_project(
                &token,
                &answers.project_name,
                &org_id,
                &answers.supabase_region,
                &db_password,
                interactive,
            )
            .await?;
            let env_file = dir.join("local.env");
            write_private(
                &env_file,
                &render_local_env(
                    &project.db_url,
                    &project.supabase_url,
                    &project.anon_key,
                    &admin_emails,
                    &random_token(48),
                    project.s3.as_ref(),
                ),
            )?;
            write_private(&dir.join("supabase-token"), &token)?;
            state.supabase_project_ref = Some(project.project_ref.clone());
            state.container_name = Some(format!("kyma-{name}"));
            save_state(&dir, &state)?;
            note("");
            note(&format!("Supabase project ready: {}", project.supabase_url));
            note(&format!("Env file: {}", env_file.display()));
            note("Next: kyma deploy up   (runs the engine container locally)");
        }
    }
    Ok(())
}

async fn cmd_up(name: &str, auto_approve: bool) -> Result<()> {
    let dir = workspace_dir(name)?;
    let mut state = load_state(&dir)?;

    // Supabase token for the provider/API (stashed by init).
    let token_path = dir.join("supabase-token");
    if let Ok(tok) = std::fs::read_to_string(&token_path) {
        std::env::set_var("SUPABASE_ACCESS_TOKEN", tok.trim());
    }

    match state.target.as_str() {
        "aws" => {
            let tf_dir = dir.join("terraform");
            match state.tool.as_str() {
                "terraform" => {
                    let bin = if have("terraform") { "terraform" } else { "tofu" };
                    run_streamed(&tf_dir, bin, &["init"])?;
                    // Supabase-storage backend with no keys yet: two-phase.
                    // Phase 1 creates just the Supabase project, then the
                    // user pastes S3 keys from the dashboard (no API for
                    // them); phase 2 applies the full stack.
                    let tfvars_path = tf_dir.join("terraform.tfvars");
                    let tfvars = std::fs::read_to_string(&tfvars_path).unwrap_or_default();
                    if tfvars.contains("storage_backend       = \"supabase\"")
                        && tfvars.contains("supabase_s3_access_key_id     = \"\"")
                    {
                        note("• Supabase Storage backend: creating the Supabase project first…");
                        run_streamed(
                            &tf_dir,
                            bin,
                            &["apply", "-target=module.kyma.module.supabase", "-auto-approve"],
                        )?;
                        let raw = run_captured(&tf_dir, bin, &["output", "-json"])?;
                        let outputs: serde_json::Value = serde_json::from_str(&raw)?;
                        let project_ref = outputs
                            .get("supabase_project_ref")
                            .and_then(|v| v.get("value"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("_");
                        match prompt_storage_keys(project_ref) {
                            Some((ak, sk)) => {
                                let updated = tfvars
                                    .replace(
                                        "supabase_s3_access_key_id     = \"\"",
                                        &format!("supabase_s3_access_key_id     = \"{ak}\""),
                                    )
                                    .replace(
                                        "supabase_s3_secret_access_key = \"\"",
                                        &format!("supabase_s3_secret_access_key = \"{sk}\""),
                                    );
                                write_private(&tfvars_path, &updated)?;
                                note("• Keys saved to terraform.tfvars — applying the full stack");
                            }
                            None => bail!(
                                "Supabase Storage keys are required for storage_backend=\"supabase\" — \
                                 paste them when prompted, or switch to storage_backend=\"s3\" in terraform.tfvars"
                            ),
                        }
                    }
                    let mut args = vec!["apply"];
                    if auto_approve {
                        args.push("-auto-approve");
                    }
                    run_streamed(&tf_dir, bin, &args)?;
                    let raw = run_captured(&tf_dir, bin, &["output", "-json"])?;
                    let outputs: serde_json::Value = serde_json::from_str(&raw)?;
                    let get = |k: &str| {
                        outputs
                            .get(k)
                            .and_then(|v| v.get("value"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    };
                    let engine_url = get("engine_url")
                        .ok_or_else(|| anyhow!("terraform outputs missing engine_url"))?;
                    state.engine_url = Some(engine_url.clone());
                    state.supabase_project_ref = get("supabase_project_ref");
                    save_state(&dir, &state)?;
                    // Supabase Storage backend: make sure the extents bucket
                    // exists (Terraform can't create Supabase buckets).
                    if let (Some(project_ref), Ok(token)) = (
                        state.supabase_project_ref.clone(),
                        std::fs::read_to_string(dir.join("supabase-token")),
                    ) {
                        let token = token.trim().to_string();
                        let keys = supabase_get(&token, &format!("/v1/projects/{project_ref}/api-keys")).await;
                        if let Ok(keys) = keys {
                            let service_role = keys
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|k| k.get("name").and_then(|n| n.as_str()) == Some("service_role"))
                                .and_then(|k| k.get("api_key").and_then(|v| v.as_str()))
                                .map(String::from);
                            if let Some(sr) = service_role {
                                let supabase_url = format!("https://{project_ref}.supabase.co");
                                match ensure_bucket(&supabase_url, &sr, "kyma").await {
                                    Ok(()) => note("• Extents bucket 'kyma' ready in Supabase Storage"),
                                    Err(e) => note(&format!("• Could not create the extents bucket ({e}) — create 'kyma' in Supabase Storage manually")),
                                }
                            }
                        }
                    }
                    finish_up(&engine_url, get("admin_password").as_deref());
                }
                "pulumi" => {
                    let pl_dir = dir.join("pulumi").join("typescript");
                    run_streamed(&pl_dir, "npm", &["install", "--no-fund", "--no-audit"])?;
                    run_streamed(
                        &pl_dir,
                        "pulumi",
                        &["package", "add", "terraform-module", "../../terraform/stack", "kymaengine"],
                    )
                    .or_else(|_| {
                        // pnpm's missing `pkg set` breaks only the link step;
                        // the SDK is generated — link manually.
                        run_streamed(
                            &pl_dir,
                            "npm",
                            &["pkg", "set", "dependencies.@pulumi/kymaengine=file:sdks/kymaengine"],
                        )
                        .and_then(|()| run_streamed(&pl_dir, "npm", &["install", "--no-fund", "--no-audit"]))
                    })?;
                    let mut args = vec!["up"];
                    if auto_approve {
                        args.push("--yes");
                    }
                    run_streamed(&pl_dir, "pulumi", &args)?;
                    let raw = run_captured(&pl_dir, "pulumi", &["stack", "output", "--json"])?;
                    let outputs: serde_json::Value = serde_json::from_str(&raw)?;
                    let engine_url = outputs
                        .get("engineUrl")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("pulumi outputs missing engineUrl"))?
                        .to_string();
                    state.engine_url = Some(engine_url.clone());
                    save_state(&dir, &state)?;
                    finish_up(&engine_url, None);
                }
                other => bail!("unknown tool {other:?} in deploy.json"),
            }
        }
        "local" => {
            let container = state
                .container_name
                .clone()
                .unwrap_or_else(|| format!("kyma-{name}"));
            let image = format!("{DEFAULT_IMAGE_REPO}:{}", state.image_tag);
            let env_file = dir.join("local.env");
            // Replace any previous container of the same name.
            let _ = Proc::new("docker").args(["rm", "-f", &container]).output();
            run_streamed(
                &dir,
                "docker",
                &[
                    "run",
                    "-d",
                    "--name",
                    &container,
                    "--env-file",
                    env_file.to_str().unwrap(),
                    "-p",
                    "8080:8080",
                    "-v",
                    &format!("{container}-data:/root/.kyma"),
                    &image,
                ],
            )?;
            let engine_url = "http://localhost:8080".to_string();
            state.engine_url = Some(engine_url.clone());
            save_state(&dir, &state)?;
            finish_up(&engine_url, None);
        }
        other => bail!("unknown target {other:?} in deploy.json"),
    }
    Ok(())
}

fn finish_up(engine_url: &str, admin_password: Option<&str>) {
    note("");
    note("──────────────────────────────────────────────");
    note(&format!("kyma is deploying at: {engine_url}"));
    note("");
    note("Sign in with your Supabase account (the first matching");
    note("KYMA_ADMIN_EMAILS sign-in gets the admin role).");
    if let Some(pw) = admin_password {
        note(&format!("Fallback admin user: admin / {pw}"));
    }
    note("");
    note(&format!("Connect the CLI:  kyma connect {engine_url} --token <api-token>"));
    note("(mint an API token in the web UI under Settings → API tokens)");
    note("──────────────────────────────────────────────");
}

async fn cmd_status(name: &str) -> Result<()> {
    let dir = workspace_dir(name)?;
    let state = load_state(&dir)?;
    println!("workspace:  {}", dir.display());
    println!("target:     {}", state.target);
    println!("tool:       {}", state.tool);
    println!("image tag:  {}", state.image_tag);
    if let Some(r) = &state.supabase_project_ref {
        println!("supabase:   https://supabase.com/dashboard/project/{r}");
    }
    match &state.engine_url {
        Some(url) => {
            println!("engine url: {url}");
            let health = reqwest::Client::new()
                .get(format!("{}/health", url.trim_end_matches('/')))
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await;
            match health {
                Ok(r) if r.status().is_success() => println!("health:     ok"),
                Ok(r) => println!("health:     {}", r.status()),
                Err(e) => println!("health:     unreachable ({e})"),
            }
        }
        None => println!("engine url: (not deployed yet — run `kyma deploy up`)"),
    }
    Ok(())
}

async fn cmd_destroy(name: &str, yes: bool) -> Result<()> {
    let dir = workspace_dir(name)?;
    let state = load_state(&dir)?;
    if !yes
        && !confirm(
            &format!("Destroy deployment '{name}' ({} target) and all its data?", state.target),
            false,
        )?
    {
        note("aborted");
        return Ok(());
    }

    if let Ok(tok) = std::fs::read_to_string(dir.join("supabase-token")) {
        std::env::set_var("SUPABASE_ACCESS_TOKEN", tok.trim());
    }

    match state.target.as_str() {
        "aws" => match state.tool.as_str() {
            "terraform" => {
                let bin = if have("terraform") { "terraform" } else { "tofu" };
                run_streamed(&dir.join("terraform"), bin, &["destroy", "-auto-approve"])?;
            }
            "pulumi" => {
                run_streamed(&dir.join("pulumi").join("typescript"), "pulumi", &["destroy", "--yes"])?;
            }
            other => bail!("unknown tool {other:?}"),
        },
        "local" => {
            if let Some(container) = &state.container_name {
                let _ = Proc::new("docker").args(["rm", "-f", container]).output();
                let _ = Proc::new("docker")
                    .args(["volume", "rm", &format!("{container}-data")])
                    .output();
                note(&format!("• Removed container {container}"));
            }
            if let Some(project_ref) = &state.supabase_project_ref {
                let token = std::fs::read_to_string(dir.join("supabase-token"))
                    .map(|t| t.trim().to_string())
                    .context("supabase token missing — delete the project in the dashboard")?;
                reqwest::Client::new()
                    .delete(format!("{}/v1/projects/{project_ref}", supabase_api_base()))
                    .bearer_auth(&token)
                    .send()
                    .await?
                    .error_for_status()
                    .context("delete Supabase project")?;
                note(&format!("• Deleted Supabase project {project_ref}"));
            }
        }
        other => bail!("unknown target {other:?}"),
    }
    note("Deployment destroyed. Workspace files kept for reference; delete with:");
    note(&format!("  rm -rf {}", dir.display()));
    Ok(())
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_parses_and_target_alias_maps() {
        assert_eq!(Compute::from_arg("fargate").unwrap(), Compute::Fargate);
        assert_eq!(Compute::from_arg("eks").unwrap(), Compute::Eks);
        assert_eq!(Compute::from_arg("helm").unwrap(), Compute::Helm);
        assert_eq!(Compute::from_arg("local").unwrap(), Compute::Local);
        assert_eq!(Compute::from_target("aws"), Some(Compute::Fargate));
        assert_eq!(Compute::from_target("local"), Some(Compute::Local));
        assert!(Compute::from_arg("ec2").is_err());
        // round-trips
        for c in [Compute::Fargate, Compute::Eks, Compute::Helm, Compute::Local] {
            assert_eq!(Compute::from_arg(c.as_str()).unwrap(), c);
        }
    }

    #[test]
    fn database_storage_auth_parse_round_trip() {
        for d in [Database::Supabase, Database::Rds, Database::External] {
            assert_eq!(Database::from_arg(d.as_str()).unwrap(), d);
        }
        for s in [Storage::S3, Storage::Supabase, Storage::External] {
            assert_eq!(Storage::from_arg(s.as_str()).unwrap(), s);
        }
        for a in [Auth::Supabase, Auth::Token, Auth::Oidc] {
            assert_eq!(Auth::from_arg(a.as_str()).unwrap(), a);
        }
        assert!(Database::from_arg("mysql").is_err());
        assert!(Storage::from_arg("gcs").is_err());
        assert!(Auth::from_arg("saml").is_err());
    }

    #[test]
    fn valid_combos_pass() {
        let ok = |c, d, s, a| validate_combo(c, d, s, a).is_ok();
        assert!(ok(Compute::Fargate, Database::Supabase, Storage::Supabase, Auth::Supabase));
        assert!(ok(Compute::Fargate, Database::Rds, Storage::S3, Auth::Token));
        assert!(ok(Compute::Eks, Database::Rds, Storage::S3, Auth::Oidc));
        assert!(ok(Compute::Helm, Database::External, Storage::External, Auth::Token));
        assert!(ok(Compute::Local, Database::External, Storage::External, Auth::Token));
        assert!(ok(Compute::Local, Database::Supabase, Storage::Supabase, Auth::Supabase));
        assert!(ok(Compute::Eks, Database::External, Storage::S3, Auth::Oidc));
    }

    #[test]
    fn invalid_combos_rejected_with_reason() {
        // native S3 needs an AWS compute target
        let e = validate_combo(Compute::Helm, Database::External, Storage::S3, Auth::Token)
            .unwrap_err()
            .to_string();
        assert!(e.contains("storage=s3") && e.contains("external"), "{e}");
        // RDS needs an AWS compute target
        assert!(validate_combo(Compute::Local, Database::Rds, Storage::External, Auth::Token).is_err());
        // supabase auth needs supabase db
        let e2 = validate_combo(Compute::Fargate, Database::Rds, Storage::S3, Auth::Supabase)
            .unwrap_err()
            .to_string();
        assert!(e2.contains("auth=supabase"), "{e2}");
        // supabase storage needs supabase db
        assert!(validate_combo(Compute::Eks, Database::Rds, Storage::Supabase, Auth::Token).is_err());
    }

    #[test]
    fn storage_and_auth_defaults() {
        assert_eq!(default_storage(Compute::Fargate, Database::Supabase), Storage::Supabase);
        assert_eq!(default_storage(Compute::Fargate, Database::Rds), Storage::S3);
        assert_eq!(default_storage(Compute::Eks, Database::External), Storage::S3);
        assert_eq!(default_storage(Compute::Helm, Database::External), Storage::External);
        assert_eq!(default_storage(Compute::Local, Database::External), Storage::External);
        assert_eq!(default_auth(Database::Supabase), Auth::Supabase);
        assert_eq!(default_auth(Database::Rds), Auth::Token);
        assert_eq!(default_auth(Database::External), Auth::Token);
    }

    fn answers() -> Answers {
        Answers {
            name: "prod".into(),
            project_name: "kyma-prod".into(),
            compute: Compute::Fargate,
            database: Database::Supabase,
            storage: Storage::Supabase,
            auth: Auth::Supabase,
            aws_region: "eu-central-1".into(),
            image_tag: "v0.1.0".into(),
            domain: "kyma.corp.com".into(),
            route53_zone_id: "Z123".into(),
            supabase_org_id: "org-123".into(),
            supabase_region: "eu-central-1".into(),
            supabase_db_password: "s3cret".into(),
            supabase_s3_access_key_id: "AKTEST".into(),
            supabase_s3_secret_access_key: "SKTEST".into(),
            supabase_url: "https://ref.supabase.co".into(),
            supabase_anon_key: "anon".into(),
            database_url: String::new(),
            external_storage: None,
            admin_emails: vec!["a@corp.com".into(), "b@corp.com".into()],
            allowed_email_domains: vec!["corp.com".into()],
            oauth_providers: vec![],
            admin_token: String::new(),
            oidc_issuer: String::new(),
            oidc_client_id: String::new(),
            kube_context: String::new(),
            ingress_host: String::new(),
        }
    }

    fn parse_tfvars(s: &str) -> std::collections::HashMap<String, String> {
        s.lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
            .collect()
    }

    #[test]
    fn pooler_url_extracted_and_password_substituted() {
        // Supavisor session-mode string with the dashboard placeholder.
        let v = serde_json::json!([
            {"database_type": "PRIMARY", "db_port": 6543, "pool_mode": "transaction",
             "connection_string": "postgres://postgres.ref:[YOUR-PASSWORD]@aws-1-eu-central-1.pooler.supabase.com:6543/postgres"},
            {"database_type": "PRIMARY", "db_port": 5432, "pool_mode": "session",
             "connection_string": "postgres://postgres.ref:[YOUR-PASSWORD]@aws-1-eu-central-1.pooler.supabase.com:5432/postgres"}
        ]);
        let url = extract_pooler_url(&v, "pw123").expect("session url");
        assert_eq!(
            url,
            "postgres://postgres.ref:pw123@aws-1-eu-central-1.pooler.supabase.com:5432/postgres"
        );

        // Map shape (TF provider style) also works.
        let v2 = serde_json::json!({
            "session": "postgres://postgres.ref:[YOUR-PASSWORD]@aws-1-x.pooler.supabase.com:5432/postgres",
            "transaction": "postgres://postgres.ref:[YOUR-PASSWORD]@aws-1-x.pooler.supabase.com:6543/postgres"
        });
        let url2 = extract_pooler_url(&v2, "pw").expect("map shape");
        assert!(url2.contains(":5432/"));
        assert!(url2.contains(":pw@"));

        // No pooler strings → None.
        assert!(extract_pooler_url(&serde_json::json!({"x": 1}), "pw").is_none());
    }

    #[test]
    fn tfvars_renders_every_answer() {
        let rendered = render_tfvars(&answers());
        let m = parse_tfvars(&rendered);
        assert_eq!(m["project_name"], "kyma-prod");
        assert_eq!(m["aws_region"], "eu-central-1");
        assert_eq!(m["compute_backend"], "fargate");
        assert_eq!(m["database_backend"], "supabase");
        assert_eq!(m["storage_backend"], "supabase");
        assert_eq!(m["auth_backend"], "supabase");
        assert_eq!(m["supabase_org_id"], "org-123");
        assert_eq!(m["supabase_db_password"], "s3cret");
        assert_eq!(m["domain"], "kyma.corp.com");
        assert_eq!(m["route53_zone_id"], "Z123");
        assert_eq!(m["image_tag"], "v0.1.0");
        assert_eq!(m["supabase_s3_access_key_id"], "AKTEST");
        assert_eq!(m["supabase_s3_secret_access_key"], "SKTEST");
        assert_eq!(m["admin_emails"], r#"["a@corp.com", "b@corp.com"]"#);
        assert_eq!(m["allowed_email_domains"], r#"["corp.com"]"#);
        assert_eq!(m["oauth_providers"], "[]");
    }

    #[test]
    fn tfvars_renders_rds_s3_token() {
        let mut a = answers();
        a.database = Database::Rds;
        a.storage = Storage::S3;
        a.auth = Auth::Token;
        a.admin_token = "tok123".into();
        a.supabase_org_id = String::new();
        a.supabase_db_password = String::new();
        let m = parse_tfvars(&render_tfvars(&a));
        assert_eq!(m["compute_backend"], "fargate");
        assert_eq!(m["database_backend"], "rds");
        assert_eq!(m["storage_backend"], "s3");
        assert_eq!(m["auth_backend"], "token");
        assert_eq!(m["admin_token"], "tok123");
        assert_eq!(m["supabase_org_id"], "");
    }

    #[test]
    fn helm_values_render_external_db_and_storage() {
        let mut a = answers();
        a.compute = Compute::Helm;
        a.database = Database::External;
        a.storage = Storage::External;
        a.auth = Auth::Token;
        a.database_url = "postgresql://u:p@host:5432/db".into();
        a.admin_token = "tok".into();
        a.ingress_host = "kyma.example.com".into();
        a.external_storage = Some(ExternalStorage {
            endpoint: "https://minio:9000".into(),
            bucket: "kyma".into(),
            region: "us-east-1".into(),
            access_key_id: "AK".into(),
            secret_access_key: "SK".into(),
            path_style: true,
        });
        let y = render_helm_values(&a);
        assert!(y.contains("repository: ghcr.io/shakedaskayo/kyma-engine"), "{y}");
        assert!(y.contains(r#"KYMA_CATALOG_URL: "postgresql://u:p@host:5432/db""#), "{y}");
        assert!(y.contains(r#"KYMA_AUTH_BACKEND: "token""#), "{y}");
        assert!(y.contains(r#"KYMA_AUTH_TOKENS: "tok:admin""#), "{y}");
        assert!(y.contains(r#"KYMA_S3_ENDPOINT: "https://minio:9000""#), "{y}");
        assert!(y.contains(r#"KYMA_S3_PATH_STYLE: "true""#), "{y}");
        assert!(y.contains(r#"host: "kyma.example.com""#), "{y}");
    }

    #[test]
    fn local_env_external_db_storage_token() {
        let mut a = answers();
        a.compute = Compute::Local;
        a.database = Database::External;
        a.storage = Storage::External;
        a.auth = Auth::Token;
        a.database_url = "postgresql://u:p@h:5432/db".into();
        a.admin_token = "tok".into();
        a.external_storage = Some(ExternalStorage {
            endpoint: "https://minio:9000".into(),
            bucket: "kyma".into(),
            region: "us-east-1".into(),
            access_key_id: "AK".into(),
            secret_access_key: "SK".into(),
            path_style: true,
        });
        let env = render_local_env_from(&a, "sk");
        assert!(env.contains("KYMA_CATALOG_URL=postgresql://u:p@h:5432/db"), "{env}");
        assert!(env.contains("KYMA_AUTH_BACKEND=token"));
        assert!(env.contains("KYMA_AUTH_TOKENS=tok:admin"));
        assert!(env.contains("KYMA_S3_ENDPOINT=https://minio:9000"));
        assert!(env.contains("KYMA_SECRET_KEY=sk"));
    }

    #[test]
    fn local_env_wires_supabase_catalog_and_auth_without_storage_keys() {
        let env = render_local_env(
            "postgresql://postgres:pw@db.ref.supabase.co:5432/postgres",
            "https://ref.supabase.co",
            "anon-key",
            &["a@corp.com".into()],
            "sk",
            None,
        );
        assert!(env.contains("KYMA_CATALOG_URL=postgresql://postgres:pw@db.ref.supabase.co"));
        assert!(env.contains("KYMA_AUTH_BACKEND=supabase"));
        assert!(env.contains("KYMA_SUPABASE_URL=https://ref.supabase.co"));
        assert!(env.contains("KYMA_SUPABASE_ANON_KEY=anon-key"));
        assert!(env.contains("KYMA_ADMIN_EMAILS=a@corp.com"));
        // No storage keys → S3 stays commented (filesystem fallback).
        assert!(env.contains("# KYMA_S3_ENDPOINT="));
    }

    #[test]
    fn local_env_defaults_extents_to_supabase_storage_when_keys_present() {
        let s3 = SupabaseS3 {
            endpoint: "https://ref.storage.supabase.co/storage/v1/s3".into(),
            region: "us-east-1".into(),
            bucket: "kyma".into(),
            access_key_id: "AK".into(),
            secret_access_key: "SK".into(),
        };
        let env = render_local_env(
            "postgresql://postgres:pw@db.ref.supabase.co:5432/postgres",
            "https://ref.supabase.co",
            "anon-key",
            &[],
            "sk",
            Some(&s3),
        );
        assert!(env.contains("KYMA_S3_ENDPOINT=https://ref.storage.supabase.co/storage/v1/s3"));
        assert!(env.contains("KYMA_S3_BUCKET=kyma"));
        assert!(env.contains("KYMA_S3_REGION=us-east-1"));
        assert!(env.contains("KYMA_S3_ACCESS_KEY_ID=AK"));
        assert!(env.contains("KYMA_S3_SECRET_ACCESS_KEY=SK"));
        assert!(env.contains("KYMA_S3_PATH_STYLE=true"));
        assert!(!env.contains("# KYMA_S3_ENDPOINT="), "no commented block when active");
    }

    #[test]
    fn oauth_app_config_reads_env_then_file() {
        // env wins
        std::env::set_var("KYMA_SUPABASE_OAUTH_CLIENT_ID", "cid-env");
        std::env::set_var("KYMA_SUPABASE_OAUTH_CLIENT_SECRET", "sec-env");
        let cfg = oauth_app_config().expect("config from env");
        assert_eq!(cfg.client_id, "cid-env");
        assert_eq!(cfg.client_secret.as_deref(), Some("sec-env"));
        std::env::remove_var("KYMA_SUPABASE_OAUTH_CLIENT_ID");
        std::env::remove_var("KYMA_SUPABASE_OAUTH_CLIENT_SECRET");
    }

    #[test]
    fn materialize_writes_all_templates_and_never_tfvars() {
        let dir = std::env::temp_dir().join(format!("kyma-deploy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        materialize(&dir).unwrap();
        for (rel, _) in DEPLOY_FILES {
            assert!(dir.join(rel).exists(), "missing {rel}");
        }
        // A user-edited tfvars must survive re-materialization.
        let tfvars = dir.join("terraform").join("terraform.tfvars");
        std::fs::write(&tfvars, "user edit").unwrap();
        materialize(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&tfvars).unwrap(), "user edit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_url_matches_rfc7636_shape() {
        // No padding, URL-safe alphabet.
        let out = base64_url(b"any carnal pleasure.");
        assert!(!out.contains('='));
        assert!(!out.contains('+'));
        assert!(!out.contains('/'));
        assert_eq!(out, "YW55IGNhcm5hbCBwbGVhc3VyZS4");
    }

    #[test]
    fn urlencode_escapes_reserved() {
        assert_eq!(urlencode("http://localhost:1234/cb"), "http%3A%2F%2Flocalhost%3A1234%2Fcb");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn random_tokens_are_alnum_and_unique() {
        let a = random_token(24);
        let b = random_token(24);
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[tokio::test]
    async fn fetch_orgs_parses_management_api_shape() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/organizations"))
            .and(header("authorization", "Bearer sbp_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": "org-1", "name": "Acme" },
                { "id": "org-2", "name": "Beta" },
            ])))
            .mount(&server)
            .await;

        std::env::set_var("KYMA_SUPABASE_API_BASE", server.uri());
        let orgs = fetch_orgs("sbp_test").await.unwrap();
        std::env::remove_var("KYMA_SUPABASE_API_BASE");
        assert_eq!(
            orgs,
            vec![("org-1".into(), "Acme".into()), ("org-2".into(), "Beta".into())]
        );
    }
}
