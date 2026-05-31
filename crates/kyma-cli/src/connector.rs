//! `kyma connector` + `kyma ingest` — connector lifecycle subcommands.
//!
//! Wraps the server's `/v1/connectors` + `/v1/credentials` HTTP API so a user
//! can stand up a GitHub/GitLab/Bitbucket repo ingestion in one command:
//!
//! ```text
//! export GITHUB_TOKEN=ghp_…
//! kyma connector add github shakedaskayo/kyma --start
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
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::{json, Value};

// ── argument parsing ─────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub(crate) enum Op {
    /// List connectors registered on the server.
    List,
    /// Register a new connector and (optionally) trigger the first run.
    Add {
        #[command(subcommand)]
        source: Source,
    },
    /// Show one connector's config + last-run status.
    Show {
        /// Connector name or UUID.
        name_or_id: String,
    },
    /// Pause a connector (stops the scheduler from triggering ticks).
    Pause { name_or_id: String },
    /// Resume a paused connector.
    Resume { name_or_id: String },
    /// Delete a connector.
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
    /// Show last_run_at / last_success_at / last_error for each connector
    /// (or just the one named).
    Status {
        /// Limit to a single connector by name or UUID.
        #[arg(long)]
        connector: Option<String>,
    },
    /// Poll status forever and print runs as they complete.
    Tail {
        #[arg(long)]
        connector: Option<String>,
        /// Polling interval in seconds.
        #[arg(long, default_value_t = 3)]
        interval: u64,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum Source {
    /// GitHub repository — pulls repos, branches, pulls, issues, contributors,
    /// and (optionally) a code graph parsed from source.
    Github {
        /// `owner/repo`, e.g. `shakedaskayo/kyma`. Pass multiple comma-separated
        /// to bundle them under one connector.
        repos: String,
        #[command(flatten)]
        common: CommonAdd,
        /// Pull source files and parse a structural code graph (functions,
        /// classes, calls, imports). Slower; off by default.
        #[arg(long)]
        codebase: bool,
        /// Comma-separated list of modules to enable. Defaults to
        /// "repos,branches,pulls,issues,contributors" plus "codebase" if
        /// `--codebase` is set.
        #[arg(long)]
        modules: Option<String>,
    },
    /// GitLab project.
    Gitlab {
        /// `group/project`, e.g. `gitlab-org/gitlab`. Multiple comma-separated OK.
        projects: String,
        #[command(flatten)]
        common: CommonAdd,
        /// Self-hosted GitLab API URL. Defaults to https://gitlab.com/api/v4.
        #[arg(long)]
        api_url: Option<String>,
    },
    /// Bitbucket repository (Bitbucket Cloud).
    Bitbucket {
        /// `workspace/repo_slug`, e.g. `atlassian/python-bitbucket`. Multiple OK.
        repos: String,
        #[command(flatten)]
        common: CommonAdd,
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

#[derive(Debug, Args)]
pub(crate) struct CommonAdd {
    /// Human-readable connector name. Defaults to `<source>-<owner>-<repo>`.
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
    /// Trigger an immediate first run after creating the connector.
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
    let cfg = client::effective_config()?;
    match op {
        IngestOp::Status { connector } => cmd_ingest_status(&cfg, connector.as_deref()).await,
        IngestOp::Tail {
            connector,
            interval,
        } => cmd_ingest_tail(&cfg, connector.as_deref(), interval).await,
    }
}

// ── list ────────────────────────────────────────────────────────────────────

async fn cmd_list(cfg: &ClientConfig) -> Result<()> {
    let items = list_connectors(cfg).await?;
    if items.is_empty() {
        println!("(no connectors registered)");
        return Ok(());
    }
    println!(
        "{:<14}  {:<22}  {:<10}  {}",
        "TYPE", "NAME", "STATUS", "LAST"
    );
    for c in items {
        let kind = c.get("type").and_then(Value::as_str).unwrap_or("?");
        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
        let status = if c.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
            "enabled"
        } else {
            "paused"
        };
        let last = c
            .get("last_success_at")
            .and_then(Value::as_str)
            .or_else(|| c.get("last_run_at").and_then(Value::as_str))
            .unwrap_or("never");
        println!("{kind:<14}  {name:<22}  {status:<10}  {last}");
    }
    Ok(())
}

// ── add ─────────────────────────────────────────────────────────────────────

async fn cmd_add(cfg: &ClientConfig, source: Source) -> Result<()> {
    let (kind, name, db, schedule_ms, start, config, credential_id) = match source {
        Source::Github {
            repos,
            common,
            codebase,
            modules,
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
            let modules_obj = build_github_modules(modules.as_deref(), codebase)?;
            let config = json!({
                "credential_id": credential_id,
                "repos": split_csv(&repos),
                "modules": modules_obj,
            });
            (
                "github",
                common.name.unwrap_or_else(|| default_name("gh", &repos)),
                common.db.unwrap_or_else(|| "github".to_string()),
                common.schedule_ms,
                common.start,
                config,
                credential_id,
            )
        }
        Source::Gitlab {
            projects,
            common,
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
            let mut config = json!({
                "credential_id": credential_id,
                "projects": split_csv(&projects),
            });
            if let Some(url) = api_url {
                config["api_url"] = Value::String(url);
            }
            (
                "gitlab",
                common.name.unwrap_or_else(|| default_name("gl", &projects)),
                common.db.unwrap_or_else(|| "gitlab".to_string()),
                common.schedule_ms,
                common.start,
                config,
                credential_id,
            )
        }
        Source::Bitbucket {
            repos,
            common,
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
            (
                "bitbucket",
                common.name.unwrap_or_else(|| default_name("bb", &repos)),
                common.db.unwrap_or_else(|| "bitbucket".to_string()),
                common.schedule_ms,
                common.start,
                json!({
                    "credential_id": credential_id,
                    "repos": split_csv(&repos),
                }),
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
    let resp = http_post(cfg, "/v1/connectors", &create_body).await?;
    let id = resp
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("server didn't return an id: {resp}"))?
        .to_string();

    println!("Created connector {name} ({kind}) → id={id}");
    println!("  database:      {db}");
    println!("  credential:    {credential_id}");
    println!("  schedule:      every {}ms", schedule_ms);

    if start {
        println!("\nTriggering first run...");
        http_post(cfg, &format!("/v1/connectors/{id}/trigger"), &json!({})).await?;
        poll_status(cfg, &id, 30).await?;
    } else {
        println!("\nRun `kyma connector trigger {id}` to start a manual tick.");
    }
    Ok(())
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

fn build_github_modules(modules: Option<&str>, codebase: bool) -> Result<Value> {
    let names: Vec<String> = if let Some(m) = modules {
        split_csv(m)
    } else {
        let mut v = vec![
            "repos".into(),
            "branches".into(),
            "pulls".into(),
            "issues".into(),
            "contributors".into(),
        ];
        if codebase {
            v.push("codebase".into());
        }
        v
    };
    let mut obj = serde_json::Map::new();
    for n in &["repos", "branches", "pulls", "issues", "contributors", "codebase"] {
        obj.insert(n.to_string(), Value::Bool(names.iter().any(|m| m == n)));
    }
    Ok(Value::Object(obj))
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
    let label = format!("{source_kind}-{}", short_label(repos_csv));
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
    let body = http_get(cfg, &format!("/v1/connectors/{id}")).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

async fn cmd_simple_op(cfg: &ClientConfig, name_or_id: &str, op: &str) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    http_post(cfg, &format!("/v1/connectors/{id}/{op}"), &json!({})).await?;
    println!("{op}d connector {id}");
    Ok(())
}

async fn cmd_remove(cfg: &ClientConfig, name_or_id: &str, yes: bool) -> Result<()> {
    let id = resolve_id(cfg, name_or_id).await?;
    if !yes {
        use std::io::{stdin, stdout, Write};
        print!("Delete connector {id}? [y/N] ");
        stdout().flush().ok();
        let mut s = String::new();
        stdin().read_line(&mut s).ok();
        if !matches!(s.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("aborted");
            return Ok(());
        }
    }
    http_delete(cfg, &format!("/v1/connectors/{id}")).await?;
    println!("removed connector {id}");
    Ok(())
}

async fn cmd_ingest_status(cfg: &ClientConfig, only: Option<&str>) -> Result<()> {
    let items = if let Some(name) = only {
        let id = resolve_id(cfg, name).await?;
        vec![http_get(cfg, &format!("/v1/connectors/{id}")).await?]
    } else {
        // The list endpoint returns sparse rows; fetch detail per connector
        // to populate last_run_at / last_success_at / last_error.
        let shallow = list_connectors(cfg).await?;
        let mut out = Vec::with_capacity(shallow.len());
        for row in shallow {
            if let Some(id) = row.get("id").and_then(Value::as_str) {
                if let Ok(detail) = http_get(cfg, &format!("/v1/connectors/{id}")).await {
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
        println!("(no connectors)");
        return Ok(());
    }
    println!(
        "{:<22}  {:<14}  {:<30}  {:<30}  {}",
        "NAME", "TYPE", "LAST_RUN", "LAST_SUCCESS", "LAST_ERROR"
    );
    for c in items {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
        let kind = c.get("type").and_then(Value::as_str).unwrap_or("?");
        let lr = c.get("last_run_at").and_then(Value::as_str).unwrap_or("-");
        let ls = c
            .get("last_success_at")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let le = c.get("last_error").and_then(Value::as_str).unwrap_or("-");
        println!("{name:<22}  {kind:<14}  {lr:<30}  {ls:<30}  {le}");
    }
    Ok(())
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
            vec![http_get(cfg, &format!("/v1/connectors/{id}")).await?]
        } else {
            list_connectors(cfg).await?
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
        let body = http_get(cfg, &format!("/v1/connectors/{id}")).await?;
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
                return Err(anyhow!("connector run failed"));
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
    let items = list_connectors(cfg).await?;
    for c in items {
        if c.get("name").and_then(Value::as_str) == Some(name_or_id) {
            if let Some(id) = c.get("id").and_then(Value::as_str) {
                return Ok(id.to_string());
            }
        }
    }
    bail!("no connector with name '{name_or_id}'")
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

async fn list_connectors(cfg: &ClientConfig) -> Result<Vec<Value>> {
    let v = http_get(cfg, "/v1/connectors").await?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("/v1/connectors: missing items array"))?;
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
