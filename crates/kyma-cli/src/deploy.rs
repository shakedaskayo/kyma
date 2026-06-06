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

#[derive(Debug, Clone)]
struct Answers {
    project_name: String,
    aws_region: String,
    supabase_org_id: String,
    supabase_region: String,
    supabase_db_password: String,
    admin_emails: Vec<String>,
    allowed_email_domains: Vec<String>,
    domain: String,
    route53_zone_id: String,
    image_tag: String,
}

fn hcl_string_list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Render terraform.tfvars from the wizard answers. Contains the Supabase DB
/// password — written 0600, never committed (workspace is outside any repo).
fn render_tfvars(a: &Answers) -> String {
    format!(
        r#"# Generated by `kyma deploy init` — edit freely; `init --force` regenerates.
project_name          = "{project_name}"
aws_region            = "{aws_region}"
supabase_org_id       = "{supabase_org_id}"
supabase_region       = "{supabase_region}"
supabase_db_password  = "{supabase_db_password}"
admin_emails          = {admin_emails}
allowed_email_domains = {allowed_email_domains}
domain                = "{domain}"
route53_zone_id       = "{route53_zone_id}"
image_tag             = "{image_tag}"
"#,
        project_name = a.project_name,
        aws_region = a.aws_region,
        supabase_org_id = a.supabase_org_id,
        supabase_region = a.supabase_region,
        supabase_db_password = a.supabase_db_password,
        admin_emails = hcl_string_list(&a.admin_emails),
        allowed_email_domains = hcl_string_list(&a.allowed_email_domains),
        domain = a.domain,
        route53_zone_id = a.route53_zone_id,
        image_tag = a.image_tag,
    )
}

/// Render the env file for the local (docker) target: Supabase catalog +
/// Supabase Auth, extents on the local filesystem inside the container
/// volume. KYMA_S3_* lines are included commented-out as the upgrade path.
fn render_local_env(
    db_url: &str,
    supabase_url: &str,
    anon_key: &str,
    admin_emails: &[String],
    secret_key: &str,
) -> String {
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
# Extents stay on the container volume (KYMA_LOCAL_DATA). To use S3-compatible
# storage instead (e.g. Supabase Storage's S3 endpoint), uncomment and fill:
# KYMA_S3_ENDPOINT=https://<ref>.supabase.co/storage/v1/s3
# KYMA_S3_BUCKET=kyma
# KYMA_S3_REGION=us-east-1
# KYMA_S3_ACCESS_KEY_ID=
# KYMA_S3_SECRET_ACCESS_KEY=
"#,
        db_url = db_url,
        supabase_url = supabase_url,
        anon_key = anon_key,
        admin_emails = admin_emails.join(","),
        secret_key = secret_key,
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
    if let Ok(client_id) = std::env::var("KYMA_SUPABASE_OAUTH_CLIENT_ID") {
        if !client_id.is_empty() && interactive {
            note("• Supabase token: starting browser OAuth flow…");
            match oauth_flow(&client_id).await {
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
async fn oauth_flow(client_id: &str) -> Result<String> {
    use sha2::{Digest, Sha256};

    let verifier = random_token(64);
    let challenge = {
        let digest = Sha256::digest(verifier.as_bytes());
        base64_url(&digest)
    };
    let state = random_token(24);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("bind callback port")?;
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
    let resp: serde_json::Value = client
        .post(format!("{base}/v1/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect_uri),
        ])
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
}

async fn provision_local_project(
    token: &str,
    name: &str,
    org_id: &str,
    region: &str,
    db_password: &str,
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

    // anon key from the api-keys listing.
    let keys = supabase_get(token, &format!("/v1/projects/{project_ref}/api-keys")).await?;
    let anon_key = keys
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k.get("name").and_then(|n| n.as_str()) == Some("anon"))
        .and_then(|k| k.get("api_key").and_then(|v| v.as_str()))
        .ok_or_else(|| anyhow!("could not find the anon api key"))?
        .to_string();

    Ok(LocalProject {
        db_url: format!(
            "postgresql://postgres:{db_password}@db.{project_ref}.supabase.co:5432/postgres"
        ),
        supabase_url: format!("https://{project_ref}.supabase.co"),
        anon_key,
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

    let answers = Answers {
        project_name: format!("kyma-{name}"),
        aws_region: aws_region.clone(),
        supabase_org_id: org_id.clone(),
        supabase_region: aws_region.clone(),
        supabase_db_password: db_password.clone(),
        admin_emails: admin_emails.clone(),
        allowed_email_domains,
        domain,
        route53_zone_id,
        image_tag: image_tag.clone(),
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

    fn answers() -> Answers {
        Answers {
            project_name: "kyma-prod".into(),
            aws_region: "eu-central-1".into(),
            supabase_org_id: "org-123".into(),
            supabase_region: "eu-central-1".into(),
            supabase_db_password: "s3cret".into(),
            admin_emails: vec!["a@corp.com".into(), "b@corp.com".into()],
            allowed_email_domains: vec!["corp.com".into()],
            domain: "kyma.corp.com".into(),
            route53_zone_id: "Z123".into(),
            image_tag: "v0.1.0".into(),
        }
    }

    #[test]
    fn tfvars_renders_every_answer() {
        let rendered = render_tfvars(&answers());
        for needle in [
            r#"project_name          = "kyma-prod""#,
            r#"aws_region            = "eu-central-1""#,
            r#"supabase_org_id       = "org-123""#,
            r#"supabase_db_password  = "s3cret""#,
            r#"admin_emails          = ["a@corp.com", "b@corp.com"]"#,
            r#"allowed_email_domains = ["corp.com"]"#,
            r#"domain                = "kyma.corp.com""#,
            r#"route53_zone_id       = "Z123""#,
            r#"image_tag             = "v0.1.0""#,
        ] {
            assert!(rendered.contains(needle), "missing {needle} in:\n{rendered}");
        }
    }

    #[test]
    fn local_env_wires_supabase_catalog_and_auth() {
        let env = render_local_env(
            "postgresql://postgres:pw@db.ref.supabase.co:5432/postgres",
            "https://ref.supabase.co",
            "anon-key",
            &["a@corp.com".into()],
            "sk",
        );
        assert!(env.contains("KYMA_CATALOG_URL=postgresql://postgres:pw@db.ref.supabase.co"));
        assert!(env.contains("KYMA_AUTH_BACKEND=supabase"));
        assert!(env.contains("KYMA_SUPABASE_URL=https://ref.supabase.co"));
        assert!(env.contains("KYMA_SUPABASE_ANON_KEY=anon-key"));
        assert!(env.contains("KYMA_ADMIN_EMAILS=a@corp.com"));
        // S3 stays commented (local filesystem default), shown as upgrade path.
        assert!(env.contains("# KYMA_S3_ENDPOINT="));
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
