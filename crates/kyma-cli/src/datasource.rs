//! `kyma data source` + `kyma ingest` — data source lifecycle subcommands.
//!
//! Wraps the server's `/v1/data sources` + `/v1/credentials` HTTP API so a user
//! can stand up a GitHub/GitLab/Bitbucket repo ingestion in one command:
//!
//! ```text
//! export GITHUB_TOKEN=ghp_…
//! kyma data source add github shakedaskayo/kyma --start
//! ```
//!
//! Token discovery order (matches the server's [`CredentialResolver`]):
//!   1. `--token <value>` explicit flag
//!   2. `--credential-id <uuid>` flag → reused as-is
//!   3. Env: `$GITHUB_TOKEN` / `$GH_TOKEN` for github
//!          `$GITLAB_TOKEN` / `$GL_TOKEN` for gitlab
//!          `$BITBUCKET_TOKEN` / `$BB_TOKEN` for bitbucket
//!   4. (future) `gh auth token` shell-out if `gh` is on PATH

use crate::client::{self, ClientConfig};
use crate::ux;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use comfy_table::{Cell, Color};
use serde_json::{json, Value};

// ── argument parsing ─────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub(crate) enum Op {
    /// List data sources registered on the server.
    List,
    /// Register a new data source and (optionally) trigger the first run.
    Add {
        #[command(subcommand)]
        source: Source,
    },
    /// Show one data source's config + last-run status.
    Show {
        /// DataSource name or UUID.
        name_or_id: String,
    },
    /// Pause a data source (stops the scheduler from triggering ticks).
    Pause { name_or_id: String },
    /// Resume a paused data source.
    Resume { name_or_id: String },
    /// Delete a data source.
    Remove {
        name_or_id: String,
        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },
    /// Queue a single tick now (independent of the schedule).
    Trigger { name_or_id: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum IngestOp {
    /// Show last_run_at / last_success_at / last_error for each data source
    /// (or just the one named).
    Status {
        /// Limit to a single data source by name or UUID.
        #[arg(long = "datasource")]
        data_source: Option<String>,
    },
    /// Poll status forever and print runs as they complete.
    Tail {
        #[arg(long = "datasource")]
        data_source: Option<String>,
        /// Polling interval in seconds.
        #[arg(long, default_value_t = 3)]
        interval: u64,
    },
    /// Push NDJSON rows from stdin straight into a table (`POST /v1/ingest`,
    /// auto-create + schema-evolve on). Used by the kyma-memory plugin's hooks.
    Push {
        /// Target table (created on first write).
        #[arg(long)]
        table: String,
        /// Target database. Defaults to `default`.
        #[arg(long)]
        db: Option<String>,
        /// Idempotency key — replays with the same key are deduplicated server-side.
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum Source {
    /// GitHub repository — ingests metadata (repos, branches, pulls, issues,
    /// contributors) AND a structural code graph (files, functions, classes,
    /// calls, imports parsed via tree-sitter). Codebase parsing is ON by
    /// default — disable with `--no-codebase` for metadata-only.
    Github {
        /// `owner/repo`, e.g. `shakedaskayo/kyma`. Pass multiple comma-separated
        /// to bundle them under one data source.
        repos: String,
        #[command(flatten)]
        common: CommonAdd,
        #[command(flatten)]
        modules: ModuleFlags,
        #[command(flatten)]
        code: CodeOpts,
        /// Cap on API pages fetched per module per tick (GitHub serves up to
        /// 100 items per page).
        #[arg(long)]
        max_pages: Option<usize>,
    },
    /// GitLab project — metadata (projects, branches, MRs, issues, members)
    /// plus a code graph (when supported server-side; the `--codebase` flag
    /// reserves the surface today).
    Gitlab {
        /// `group/project`, e.g. `gitlab-org/gitlab`. Multiple comma-separated OK.
        projects: String,
        #[command(flatten)]
        common: CommonAdd,
        #[command(flatten)]
        modules: ModuleFlags,
        #[command(flatten)]
        code: CodeOpts,
        /// Self-hosted GitLab API URL. Defaults to https://gitlab.com/api/v4.
        #[arg(long)]
        api_url: Option<String>,
    },
    /// Bitbucket repository (Bitbucket Cloud) — metadata plus a code graph
    /// (when supported server-side; the `--codebase` flag reserves the surface
    /// today).
    Bitbucket {
        /// `workspace/repo_slug`, e.g. `atlassian/python-bitbucket`. Multiple OK.
        repos: String,
        #[command(flatten)]
        common: CommonAdd,
        #[command(flatten)]
        modules: ModuleFlags,
        #[command(flatten)]
        code: CodeOpts,
        /// Atlassian username (paired with app-password).
        #[arg(long)]
        username: Option<String>,
        /// Bitbucket app password (paired with username). If both username and
        /// app-password are present they're stored as a `basic` credential
        /// instead of a `pat`.
        #[arg(long)]
        app_password: Option<String>,
    },
}

/// Per-module toggles shared across git sources. All modules are ON by default;
/// `--no-<module>` disables one (e.g. `--no-codebase` for metadata-only).
#[derive(Debug, Args)]
pub(crate) struct ModuleFlags {
    /// Disable the `repos` (or `projects`) module.
    #[arg(long = "no-repos")]
    pub no_repos: bool,
    /// Disable the `branches` module.
    #[arg(long = "no-branches")]
    pub no_branches: bool,
    /// Disable the `pulls` (or `merge_requests`) module.
    #[arg(long = "no-pulls")]
    pub no_pulls: bool,
    /// Disable the `issues` module.
    #[arg(long = "no-issues")]
    pub no_issues: bool,
    /// Disable the `contributors` (or `members`) module.
    #[arg(long = "no-contributors")]
    pub no_contributors: bool,
    /// Disable the `codebase` module (structural code graph). Default ON.
    #[arg(long = "no-codebase")]
    pub no_codebase: bool,
}

impl ModuleFlags {
    fn into_object(self) -> serde_json::Map<String, Value> {
        let mut m = serde_json::Map::new();
        m.insert("repos".into(), Value::Bool(!self.no_repos));
        m.insert("branches".into(), Value::Bool(!self.no_branches));
        m.insert("pulls".into(), Value::Bool(!self.no_pulls));
        m.insert("issues".into(), Value::Bool(!self.no_issues));
        m.insert(
            "contributors".into(),
            Value::Bool(!self.no_contributors),
        );
        m.insert("codebase".into(), Value::Bool(!self.no_codebase));
        m
    }
}

/// Fine-grained controls for the structural code-graph module. All have sane
/// defaults; pass to tune resource use or scope.
#[derive(Debug, Args)]
pub(crate) struct CodeOpts {
    /// Languages to parse, comma-separated (e.g. `rust,python,typescript`).
    /// Defaults to all supported (rust, python, typescript, javascript, go).
    #[arg(long)]
    pub languages: Option<String>,
    /// Skip files larger than this many bytes. Default 1 MiB.
    #[arg(long)]
    pub max_bytes: Option<usize>,
    /// Cap on the number of files fetched + parsed per tick. Default 300.
    #[arg(long)]
    pub max_files: Option<usize>,
    /// Glob patterns to exclude, comma-separated (e.g.
    /// `vendor/**,**/*_test.go,dist/**`).
    #[arg(long)]
    pub exclude: Option<String>,
}

impl CodeOpts {
    fn into_object(self) -> Option<serde_json::Map<String, Value>> {
        if self.languages.is_none()
            && self.max_bytes.is_none()
            && self.max_files.is_none()
            && self.exclude.is_none()
        {
            return None;
        }
        let mut m = serde_json::Map::new();
        if let Some(langs) = self.languages {
            m.insert("languages".into(), Value::Array(split_csv(&langs).into_iter().map(Value::String).collect()));
        }
        if let Some(b) = self.max_bytes {
            m.insert("max_file_bytes".into(), Value::Number(b.into()));
        }
        if let Some(f) = self.max_files {
            m.insert("max_files_per_tick".into(), Value::Number(f.into()));
        }
        if let Some(ex) = self.exclude {
            m.insert("exclude_globs".into(), Value::Array(split_csv(&ex).into_iter().map(Value::String).collect()));
        }
        Some(m)
    }
}

#[derive(Debug, Args)]
pub(crate) struct CommonAdd {
    /// Human-readable data source name. Defaults to `<source>-<owner>-<repo>`.
    #[arg(long)]
    pub name: Option<String>,
    /// Target database (created with `kyma create-database`). Defaults to the
    /// source kind ("github", "gitlab", "bitbucket").
    #[arg(long)]
    pub db: Option<String>,
    /// Explicit token. Otherwise read from the source's env vars (see help).
    #[arg(long)]
    pub token: Option<String>,
    /// Reuse an existing credential by id (skips creating a new one).
    #[arg(long)]
    pub credential_id: Option<String>,
    /// Schedule interval in milliseconds. Default 300000 (5 minutes).
    #[arg(long, default_value_t = 300_000)]
    pub schedule_ms: i64,
    /// Trigger an immediate first run after creating the data source.
    #[arg(long)]
    pub start: bool,
}

// ── dispatch ─────────────────────────────────────────────────────────────────

pub(crate) async fn run(op: Op) -> Result<()> {
    let cfg = client::effective_config()?;
    match op {
        Op::List => cmd_list(&cfg).await,
        Op::Add { source } => cmd_add(&cfg, source).await,
        Op::Show { name_or_id } => cmd_show(&cfg, &name_or_id).await,
        Op::Pause { name_or_id } => cmd_simple_op(&cfg, &name_or_id, "pause").await,
        Op::Resume { name_or_id } => cmd_simple_op(&cfg, &name_or_id, "resume").await,
        Op::Trigger { name_or_id } => cmd_simple_op(&cfg, &name_or_id, "trigger").await,
        Op::Remove { name_or_id, yes } => cmd_remove(&cfg, &name_or_id, yes).await,
    }
}

pub(crate) async fn run_ingest(op: IngestOp) -> Result<()> {
    // `push` resolves its own config (and is used by hooks), so handle it before
    // requiring a config the status/tail subcommands need.
    if let IngestOp::Push {
        table,
        db,
        idempotency_key,
    } = op
    {
        return crate::plugin::ingest_push(table, db, idempotency_key).await;
    }
    let cfg = client::effective_config()?;
    match op {
        IngestOp::Status { data_source } => cmd_ingest_status(&cfg, data_source.as_deref()).await,
        IngestOp::Tail {
            data_source,
            interval,
        } => cmd_ingest_tail(&cfg, data_source.as_deref(), interval).await,
        IngestOp::Push { .. } => unreachable!("handled above"),
    }
}

// ── list ────────────────────────────────────────────────────────────────────

async fn cmd_list(cfg: &ClientConfig) -> Result<()> {
    let items = list_data_sources(cfg).await?;
    if items.is_empty() {
        println!("(no data sources registered)");
        return Ok(());
    }
    let mut t = ux::table::table(vec!["TYPE", "NAME", "STATUS", "LAST"]);
    for c in &items {
        let [kind, name, status, last] = data_source_row(c);
        t.add_row(vec![
            Cell::new(kind),
            Cell::new(name),
            ux::table::status_cell(&status),
            Cell::new(last),
        ]);
    }
    println!("{t}");
    Ok(())
}

/// Extracts the four display fields for one `datasource list` row. Pure and
/// tested directly — `cmd_list` just formats what this returns.
fn data_source_row(c: &Value) -> [String; 4] {
    let kind = c
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let name = c
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let status = if c.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
        "active"
    } else {
        "paused"
    }
    .to_string();
    let last = c
        .get("last_success_at")
        .and_then(Value::as_str)
        .or_else(|| c.get("last_run_at").and_then(Value::as_str))
        .unwrap_or("never")
        .to_string();
    [kind, name, status, last]
}

// ── add ─────────────────────────────────────────────────────────────────────

async fn cmd_add(cfg: &ClientConfig, source: Source) -> Result<()> {
    let (kind, name, db, schedule_ms, start, config, credential_id) = match source {
        Source::Github {
            repos,
            common,
            modules,
            code,
            max_pages,
        } => {
            let credential_id = resolve_credential(
                cfg,
                &common,
                &["GITHUB_TOKEN", "GH_TOKEN"],
                "github",
                &repos,
                |token| json!({ "kind": "pat", "token": token }),
            )
            .await?;
            let mut config = serde_json::Map::new();
            config.insert("credential_id".into(), Value::String(credential_id.clone()));
            config.insert(
                "repos".into(),
                Value::Array(split_csv(&repos).into_iter().map(Value::String).collect()),
            );
            config.insert("modules".into(), Value::Object(modules.into_object()));
            if let Some(p) = max_pages {
                config.insert("max_pages_per_tick".into(), Value::Number(p.into()));
            }
            if let Some(c) = code.into_object() {
                config.insert("code".into(), Value::Object(c));
            }
            (
                "github",
                common.name.unwrap_or_else(|| default_name("gh", &repos)),
                common.db.unwrap_or_else(|| "github".to_string()),
                common.schedule_ms,
                common.start,
                Value::Object(config),
                credential_id,
            )
        }
        Source::Gitlab {
            projects,
            common,
            modules,
            code,
            api_url,
        } => {
            let credential_id = resolve_credential(
                cfg,
                &common,
                &["GITLAB_TOKEN", "GL_TOKEN"],
                "gitlab",
                &projects,
                |token| json!({ "kind": "pat", "token": token }),
            )
            .await?;
            let mut config = serde_json::Map::new();
            config.insert("credential_id".into(), Value::String(credential_id.clone()));
            config.insert(
                "projects".into(),
                Value::String(projects.clone()),
            );
            if let Some(url) = api_url {
                config.insert("api_url".into(), Value::String(url));
            }
            // Forward-compatibility: server doesn't read these yet, but the
            // surface is reserved so today's `kyma data source add gitlab …
            // --no-codebase` keeps working when codebase parsing lands.
            config.insert("modules".into(), Value::Object(modules.into_object()));
            if let Some(c) = code.into_object() {
                config.insert("code".into(), Value::Object(c));
            }
            (
                "gitlab",
                common.name.unwrap_or_else(|| default_name("gl", &projects)),
                common.db.unwrap_or_else(|| "gitlab".to_string()),
                common.schedule_ms,
                common.start,
                Value::Object(config),
                credential_id,
            )
        }
        Source::Bitbucket {
            repos,
            common,
            modules,
            code,
            username,
            app_password,
        } => {
            let credential_id = if let (Some(u), Some(p)) = (&username, &app_password) {
                // Username + app-password → store as `basic` credential.
                let u = u.clone();
                let p = p.clone();
                create_credential(
                    cfg,
                    &format!("bitbucket-{}", short_label(&repos)),
                    json!({ "kind": "basic", "username": u, "password": p }),
                )
                .await?
            } else {
                resolve_credential(
                    cfg,
                    &common,
                    &["BITBUCKET_TOKEN", "BB_TOKEN"],
                    "bitbucket",
                    &repos,
                    |token| json!({ "kind": "pat", "token": token }),
                )
                .await?
            };
            let mut config = serde_json::Map::new();
            config.insert("credential_id".into(), Value::String(credential_id.clone()));
            config.insert("repos".into(), Value::String(repos.clone()));
            // Reserved knobs — see Gitlab branch comment.
            config.insert("modules".into(), Value::Object(modules.into_object()));
            if let Some(c) = code.into_object() {
                config.insert("code".into(), Value::Object(c));
            }
            (
                "bitbucket",
                common.name.unwrap_or_else(|| default_name("bb", &repos)),
                common.db.unwrap_or_else(|| "bitbucket".to_string()),
                common.schedule_ms,
                common.start,
                Value::Object(config),
                credential_id,
            )
        }
    };

    let create_body = json!({
        "name": name,
        "type": kind,
        "target_database": db,
        "target_table": "",
        "schedule_ms": schedule_ms,
        "config": config,
    });
    let resp = http_post(cfg, "/v1/data-sources", &create_body).await?;
    let id = resp
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("server didn't return an id: {resp}"))?
        .to_string();

    println!("{}", add_success_line(&name, kind, &id));
    println!("  database:      {db}");
    println!("  credential:    {credential_id}");
    println!("  schedule:      every {}ms", schedule_ms);

    if start {
        println!("\nTriggering first run...");
        http_post(cfg, &format!("/v1/data-sources/{id}/trigger"), &json!({})).await?;
        poll_status(cfg, &id, 30).await?;
    } else {
        println!("\nRun `kyma datasource trigger {id}` to start a manual tick.");
    }
    Ok(())
}

/// Builds the headline success message for `datasource add`. Pure and
/// tested directly.
fn add_success_line(name: &str, kind: &str, id: &str) -> String {
    ux::theme::ok(&format!("Created data source {name} ({kind}) → id={id}"))
}

fn default_name(prefix: &str, items_csv: &str) -> String {
    let first = items_csv.split(',').next().unwrap_or("").trim();
    let mangled: String = first
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mangled = mangled.trim_matches('-').to_lowercase();
    if mangled.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}-{mangled}")
    }
}

fn short_label(items_csv: &str) -> String {
    items_csv
        .split(',')
        .next()
        .unwrap_or("conn")
        .trim()
        .replace('/', "-")
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

// ── credential resolution ───────────────────────────────────────────────────

async fn resolve_credential(
    cfg: &ClientConfig,
    common: &CommonAdd,
    env_vars: &[&str],
    source_kind: &str,
    repos_csv: &str,
    build_value: impl Fn(&str) -> Value,
) -> Result<String> {
    if let Some(id) = &common.credential_id {
        return Ok(id.clone());
    }
    let token = if let Some(t) = &common.token {
        t.clone()
    } else if let Some(t) = env_vars
        .iter()
        .filter_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .next()
    {
        t
    } else if let Some(t) = gh_cli_token(source_kind) {
        eprintln!("(using token from `gh auth token`)");
        t
    } else {
        bail!(
            "no token — pass --token, --credential-id, or set one of: {} (also tried `gh auth token`)",
            env_vars.join(", ")
        );
    };
    // Append a short timestamp suffix so re-runs don't collide on the
    // (tenant, label) unique key.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{:x}", d.as_secs() & 0xffff))
        .unwrap_or_else(|_| "0".into());
    let label = format!("{source_kind}-{}-{suffix}", short_label(repos_csv));
    create_credential(cfg, &label, build_value(&token)).await
}

/// Best-effort shell-out to `gh auth token` (GitHub CLI). Only invoked for the
/// `github` source kind. Silently returns None on any failure.
fn gh_cli_token(source_kind: &str) -> Option<String> {
    if source_kind != "github" {
        return None;
    }
    let out = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let t = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

async fn create_credential(cfg: &ClientConfig, label: &str, value: Value) -> Result<String> {
    let body = json!({ "label": label, "value": value });
    let resp = http_post(cfg, "/v1/credentials", &body).await?;
    resp.get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow!("credentials endpoint didn't return an id: {resp}"))
}

// ── show / status / pause / resume / trigger / remove ───────────────────────

async fn cmd_show(cfg: &ClientConfig, name_or_id: &str) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    let body = http_get(cfg, &format!("/v1/data-sources/{id}")).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn cmd_simple_op(cfg: &ClientConfig, name_or_id: &str, op: &str) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    http_post(cfg, &format!("/v1/data-sources/{id}/{op}"), &json!({})).await?;
    println!("{}", simple_op_success_line(op, &id));
    Ok(())
}

/// Builds the success message for pause/resume/trigger. Pure and tested
/// directly.
fn simple_op_success_line(op: &str, id: &str) -> String {
    // Past-tense per op — "{op}d" only reads right for pause/resume.
    let past = match op {
        "trigger" => "triggered a run for",
        "pause" => "paused",
        "resume" => "resumed",
        other => other,
    };
    ux::theme::ok(&format!("{past} data source {id}"))
}

async fn cmd_remove(cfg: &ClientConfig, name_or_id: &str, yes: bool) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    if !yes {
        use std::io::{stdin, stdout, Write};
        print!("Delete data source {id}? [y/N] ");
        stdout().flush().ok();
        let mut s = String::new();
        stdin().read_line(&mut s).ok();
        if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("{}", ux::theme::muted("aborted"));
            return Ok(());
        }
    }
    http_delete(cfg, &format!("/v1/data-sources/{id}")).await?;
    println!("{}", ux::theme::ok(&format!("removed data source {id}")));
    Ok(())
}

async fn cmd_ingest_status(cfg: &ClientConfig, only: Option<&str>) -> Result<()> {
    let items = if let Some(name) = only {
        let id = resolve_id(cfg, name).await?;
        vec![http_get(cfg, &format!("/v1/data-sources/{id}")).await?]
    } else {
        // The list endpoint returns sparse rows; fetch detail per data source
        // to populate last_run_at / last_success_at / last_error.
        let shallow = list_data_sources(cfg).await?;
        let mut out = Vec::with_capacity(shallow.len());
        for row in shallow {
            if let Some(id) = row.get("id").and_then(Value::as_str) {
                if let Ok(detail) = http_get(cfg, &format!("/v1/data-sources/{id}")).await {
                    out.push(detail);
                } else {
                    out.push(row);
                }
            } else {
                out.push(row);
            }
        }
        out
    };
    if items.is_empty() {
        println!("(no data sources)");
        return Ok(());
    }
    let mut t = ux::table::table(vec!["NAME", "TYPE", "LAST_RUN", "LAST_SUCCESS", "LAST_ERROR"]);
    for c in &items {
        let [name, kind, lr, ls, le] = ingest_status_row(c);
        let le_cell = if le == "-" {
            Cell::new(le)
        } else {
            Cell::new(le).fg(Color::Red)
        };
        t.add_row(vec![
            Cell::new(name),
            Cell::new(kind),
            Cell::new(lr),
            Cell::new(ls),
            le_cell,
        ]);
    }
    println!("{t}");
    Ok(())
}

/// Extracts the five display fields for one `ingest status` row. Pure and
/// tested directly.
fn ingest_status_row(c: &Value) -> [String; 5] {
    let name = c
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let kind = c
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let lr = c
        .get("last_run_at")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let ls = c
        .get("last_success_at")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    let le = c
        .get("last_error")
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string();
    [name, kind, lr, ls, le]
}

async fn cmd_ingest_tail(
    cfg: &ClientConfig,
    only: Option<&str>,
    interval: u64,
) -> Result<()> {
    let mut last_seen: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    loop {
        let items = if let Some(name) = only {
            let id = resolve_id(cfg, name).await?;
            vec![http_get(cfg, &format!("/v1/data-sources/{id}")).await?]
        } else {
            list_data_sources(cfg).await?
        };
        for c in items {
            let id = c.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
            let lr = c
                .get("last_run_at")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let le = c
                .get("last_error")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let prev = last_seen.get(&id).cloned();
            if prev.as_ref() != Some(&(lr.clone(), le.clone())) && !lr.is_empty() {
                last_seen.insert(id.clone(), (lr.clone(), le.clone()));
                if le.is_empty() {
                    println!("[{lr}] {name}: ok");
                } else {
                    println!("[{lr}] {name}: ERROR {le}");
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
}

async fn poll_status(cfg: &ClientConfig, id: &str, timeout_secs: u64) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed().as_secs() < timeout_secs {
        let body = http_get(cfg, &format!("/v1/data-sources/{id}")).await?;
        let lr = body
            .get("last_run_at")
            .and_then(Value::as_str)
            .unwrap_or("");
        let ls = body
            .get("last_success_at")
            .and_then(Value::as_str)
            .unwrap_or("");
        let le = body.get("last_error").and_then(Value::as_str).unwrap_or("");
        if !lr.is_empty() {
            if le.is_empty() {
                println!("  [{lr}] success — last_success_at={ls}");
                return Ok(());
            } else {
                println!("  [{lr}] ERROR: {le}");
                return Err(anyhow!("data source run failed"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!("  (no run completed within {timeout_secs}s — check `kyma ingest status` later)");
    Ok(())
}

// ── id resolution ───────────────────────────────────────────────────────────

async fn resolve_id(cfg: &ClientConfig, name_or_id: &str) -> Result<String> {
    if looks_like_uuid(name_or_id) {
        return Ok(name_or_id.to_string());
    }
    let items = list_data_sources(cfg).await?;
    for c in items {
        if c.get("name").and_then(Value::as_str) == Some(name_or_id) {
            if let Some(id) = c.get("id").and_then(Value::as_str) {
                return Ok(id.to_string());
            }
        }
    }
    bail!("no data source with name '{name_or_id}'")
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars()
            .filter(|c| *c == '-')
            .count()
            == 4
        && s.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
}

// ── HTTP plumbing ───────────────────────────────────────────────────────────

async fn list_data_sources(cfg: &ClientConfig) -> Result<Vec<Value>> {
    let v = http_get(cfg, "/v1/data-sources").await?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("/v1/data-sources: missing items array"))?;
    Ok(items)
}

async fn http_get(cfg: &ClientConfig, path: &str) -> Result<Value> {
    let url = format!("{}{path}", cfg.endpoint.trim_end_matches('/'));
    let mut req = client::http_client().get(url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.with_context(|| format!("GET {path}"))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("GET {path}: {status}: {body}");
    }
    serde_json::from_str(&body).with_context(|| format!("parse JSON from {path}: {body}"))
}

async fn http_post(cfg: &ClientConfig, path: &str, body: &Value) -> Result<Value> {
    let url = format!("{}{path}", cfg.endpoint.trim_end_matches('/'));
    let mut req = client::http_client().post(url).json(body);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.with_context(|| format!("POST {path}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("POST {path}: {status}: {text}");
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).with_context(|| format!("parse JSON from {path}: {text}"))
}

async fn http_delete(cfg: &ClientConfig, path: &str) -> Result<()> {
    let url = format!("{}{path}", cfg.endpoint.trim_end_matches('/'));
    let mut req = client::http_client().delete(url);
    if let Some(t) = &cfg.token {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.with_context(|| format!("DELETE {path}"))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        bail!("DELETE {path}: {status}: {body}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_source_row_active_with_last_success() {
        let c = json!({
            "type": "github",
            "name": "kyma",
            "enabled": true,
            "last_success_at": "2026-07-01T00:00:00Z",
        });
        assert_eq!(
            data_source_row(&c),
            [
                "github".to_string(),
                "kyma".to_string(),
                "active".to_string(),
                "2026-07-01T00:00:00Z".to_string(),
            ]
        );
    }

    #[test]
    fn data_source_row_paused_falls_back_to_last_run() {
        let c = json!({
            "type": "gitlab",
            "name": "proj",
            "enabled": false,
            "last_run_at": "2026-06-01T00:00:00Z",
        });
        assert_eq!(
            data_source_row(&c),
            [
                "gitlab".to_string(),
                "proj".to_string(),
                "paused".to_string(),
                "2026-06-01T00:00:00Z".to_string(),
            ]
        );
    }

    #[test]
    fn data_source_row_missing_fields_use_placeholders() {
        let c = json!({});
        assert_eq!(
            data_source_row(&c),
            [
                "?".to_string(),
                "?".to_string(),
                "active".to_string(),
                "never".to_string(),
            ]
        );
    }

    #[test]
    fn add_success_line_includes_name_kind_and_id() {
        let line = add_success_line("kyma", "github", "abc-123");
        assert!(line.contains("Created data source kyma (github) → id=abc-123"));
        assert!(line.contains(ux::theme::CHECK));
    }

    #[test]
    fn simple_op_success_line_pause() {
        let line = simple_op_success_line("pause", "abc");
        assert!(line.contains("paused data source abc"));
        assert!(line.contains(ux::theme::CHECK));
    }

    #[test]
    fn simple_op_success_line_resume() {
        let line = simple_op_success_line("resume", "abc");
        assert!(line.contains("resumed data source abc"));
    }

    #[test]
    fn simple_op_success_line_trigger() {
        let line = simple_op_success_line("trigger", "abc");
        assert!(line.contains("triggered a run for data source abc"));
    }

    #[test]
    fn ingest_status_row_extracts_all_fields() {
        let c = json!({
            "name": "kyma",
            "type": "github",
            "last_run_at": "t1",
            "last_success_at": "t2",
            "last_error": "boom",
        });
        assert_eq!(
            ingest_status_row(&c),
            [
                "kyma".to_string(),
                "github".to_string(),
                "t1".to_string(),
                "t2".to_string(),
                "boom".to_string(),
            ]
        );
    }

    #[test]
    fn ingest_status_row_missing_fields_use_placeholders() {
        let c = json!({});
        assert_eq!(
            ingest_status_row(&c),
            [
                "?".to_string(),
                "?".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
            ]
        );
    }
}
