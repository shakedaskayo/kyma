//! `pensieve brain …` — publish pensieve's memory as Git-clonable Obsidian vaults.
//!
//! The clone IS the interface: everything here is possible with plain git
//! (`git clone http://host:7777/git/<name>.git`, password = pensieve token).
//! These subcommands wrap the `/v1/brain` management API plus a safe
//! clone flow (`pensieve git-credential` keeps tokens out of URLs and shell
//! history). Deliberately no `brain grep`/`cat` — `rg`, Obsidian, and any
//! agent's file tools already do that better on the clone.

use crate::client::{self, ClientConfig};
use crate::ux;
use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};

#[derive(Debug, Subcommand)]
pub(crate) enum Op {
    /// List published brains.
    List,
    /// Publish a new brain (git repo rendered from memory realms).
    Create {
        /// Brain name (lowercase, digits, `-`/`_`; also the repo name).
        name: String,
        /// Realm(s) to include (repeatable). Omit with --all-realms.
        #[arg(long = "realm")]
        realms: Vec<String>,
        /// Include every realm.
        #[arg(long)]
        all_realms: bool,
        /// Export interval: 30m | 1h | 6h | daily | manual (default 15m).
        #[arg(long)]
        schedule: Option<String>,
        /// Enable the agentic wiki gardener for this brain.
        #[arg(long)]
        gardener: bool,
    },
    /// Show one brain's config, stats, and clone command.
    Show { name: String },
    /// Trigger an export now.
    Export { name: String },
    /// Kick off a wiki-gardener run (agentic curation of wiki/ pages).
    Garden { name: String },
    /// Recent export / push-ingest runs.
    Runs {
        name: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Print the clone URL (token only with --with-token).
    Url {
        name: String,
        /// Embed the configured token in the URL (lands in shell history!).
        #[arg(long)]
        with_token: bool,
    },
    /// Clone a brain with credentials wired via `pensieve git-credential`.
    Clone {
        name: String,
        /// Target directory (default: the brain name).
        dir: Option<String>,
    },
    /// Delete a brain's registry entry + served repo (memories untouched).
    Delete {
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },
}

fn schedule_secs(s: &str) -> Result<u64> {
    Ok(match s {
        "manual" | "0" => 0,
        "15m" => 900,
        "30m" => 1800,
        "1h" | "hourly" => 3600,
        "6h" => 21600,
        "daily" | "24h" => 86400,
        other => other
            .parse::<u64>()
            .map_err(|_| anyhow!("schedule must be one of manual|15m|30m|1h|6h|daily or seconds"))?,
    })
}

fn clone_url(cfg: &ClientConfig, name: &str) -> String {
    format!("{}/git/{name}.git", cfg.endpoint.trim_end_matches('/'))
}

fn realms_label(config: &Value) -> String {
    match config["realms"]["kind"].as_str() {
        Some("all") => "*".to_string(),
        _ => config["realms"]["realms"]
            .as_array()
            .map(|a| {
                a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
            })
            .unwrap_or_default(),
    }
}

fn short_sha(v: &Value) -> String {
    v.as_str().map_or_else(|| "-".to_string(), |s| s.chars().take(8).collect())
}

fn rel_time(v: &Value) -> String {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(
            || "-".to_string(),
            |t| ux::format::relative_time(t.with_timezone(&chrono::Utc), chrono::Utc::now()),
        )
}

pub(crate) async fn run(op: Op) -> Result<()> {
    let cfg = client::effective_config()?;
    match op {
        Op::List => {
            let resp = client::get_json(&cfg, "/v1/brain").await?;
            let brains = resp["brains"].as_array().cloned().unwrap_or_default();
            if brains.is_empty() {
                println!("{}", ux::theme::muted("No brains published yet."));
                println!(
                    "  {} pensieve brain create <name> --realm <realm>",
                    ux::theme::accent("create one:")
                );
                return Ok(());
            }
            let mut t = ux::table::table(vec![
                "NAME", "REALMS", "NOTES", "LAST EXPORT", "HEAD", "INTERVAL",
            ]);
            for b in &brains {
                let (c, r) = (&b["config"], &b["runtime"]);
                let interval = match c["export_interval_secs"].as_u64().unwrap_or(0) {
                    0 => "manual".to_string(),
                    s => format!("{}m", s / 60),
                };
                t.add_row(vec![
                    c["name"].as_str().unwrap_or("?").to_string(),
                    realms_label(c),
                    r["note_count"].as_u64().unwrap_or(0).to_string(),
                    rel_time(&r["last_export_at"]),
                    short_sha(&r["last_commit"]),
                    interval,
                ]);
            }
            println!("{t}");
            println!(
                "\n  {} git clone {}   {}",
                ux::theme::muted("clone:"),
                clone_url(&cfg, "<name>"),
                ux::theme::muted("(password = pensieve token)")
            );
        }
        Op::Create { name, realms, all_realms, schedule, gardener } => {
            if realms.is_empty() && !all_realms {
                bail!("pass --realm <realm> (repeatable) or --all-realms");
            }
            let mut body = json!({ "name": name, "realms": realms, "all_realms": all_realms });
            if let Some(s) = schedule {
                body["export_interval_secs"] = json!(schedule_secs(&s)?);
            }
            if gardener {
                body["gardener"] = json!({ "enabled": true });
            }
            let sp = ux::spinner::spinner(format!("Publishing brain `{name}` (first export)…"));
            match client::post_json(&cfg, "/v1/brain", body).await {
                Ok(resp) => {
                    let fe = &resp["first_export"];
                    sp.finish_success(&format!(
                        "brain `{name}` published — {} notes, commit {}",
                        fe["notes"].as_u64().unwrap_or(0),
                        short_sha(&fe["commit"]),
                    ));
                    println!("\n  git clone {}", ux::theme::accent(&clone_url(&cfg, &name)));
                    println!("  {}", ux::theme::muted("password = your pensieve API token"));
                    println!("  {} pensieve brain clone {name}", ux::theme::muted("or:"));
                }
                Err(e) => {
                    sp.finish_error("publish failed");
                    return Err(e);
                }
            }
        }
        Op::Show { name } => {
            let b = client::get_json(&cfg, &format!("/v1/brain/{name}")).await?;
            let (c, r) = (&b["config"], &b["runtime"]);
            println!("{}", ux::theme::accent(&name));
            let kv = |k: &str, v: String| println!("  {:<14} {v}", ux::theme::muted(k));
            kv("realms", realms_label(c));
            kv("layout", c["layout"].as_str().unwrap_or("-").to_string());
            kv("notes", r["note_count"].as_u64().unwrap_or(0).to_string());
            kv("last export", rel_time(&r["last_export_at"]));
            kv("head", short_sha(&r["last_commit"]));
            kv(
                "interval",
                match c["export_interval_secs"].as_u64().unwrap_or(0) {
                    0 => "manual".into(),
                    s => format!("{s}s"),
                },
            );
            kv(
                "gardener",
                if c["gardener"]["enabled"].as_bool().unwrap_or(false) { "on".into() } else { "off".into() },
            );
            if let Some(err) = r["last_error"].as_str() {
                kv("last error", ux::theme::error(err));
            }
            println!("\n  git clone {}", ux::theme::accent(&clone_url(&cfg, &name)));
            println!("  {}", ux::theme::muted("password = your pensieve API token"));
        }
        Op::Export { name } => {
            let sp = ux::spinner::spinner(format!("Exporting `{name}`…"));
            match client::post_json(&cfg, &format!("/v1/brain/{name}/export"), json!({})).await {
                Ok(resp) => {
                    if resp["noop"].as_bool().unwrap_or(false) {
                        sp.finish_success("no changes — vault already up to date");
                    } else {
                        sp.finish_success(&format!(
                            "{} notes → commit {}",
                            resp["notes"].as_u64().unwrap_or(0),
                            short_sha(&resp["commit"]),
                        ));
                    }
                }
                Err(e) => {
                    sp.finish_error("export failed");
                    return Err(e);
                }
            }
        }
        Op::Garden { name } => {
            let resp = client::post_json(&cfg, &format!("/v1/brain/{name}/garden"), json!({})).await?;
            if resp["deduped"].as_bool().unwrap_or(false) {
                println!("{}", ux::theme::warn("a dreaming run is already in flight — try again later"));
            } else {
                println!(
                    "{} gardener run started — watch it under Memory → Dreaming; wiki/ updates land on the next export",
                    ux::theme::success("✓")
                );
            }
        }
        Op::Runs { name, limit } => {
            let resp = client::get_json(&cfg, &format!("/v1/brain/{name}/runs")).await?;
            let runs = resp["runs"].as_array().cloned().unwrap_or_default();
            if runs.is_empty() {
                println!("{}", ux::theme::muted("No runs yet."));
                return Ok(());
            }
            let mut t = ux::table::table(vec!["KIND", "WHEN", "RESULT", "DETAIL"]);
            for run in runs.iter().take(limit) {
                let status = if run["error"].is_string() {
                    "error"
                } else if run["noop"].as_bool().unwrap_or(false) {
                    "noop"
                } else {
                    "ok"
                };
                let detail = if let Some(e) = run["error"].as_str() {
                    ux::format::truncate(e, 60)
                } else if run["kind"] == "push_ingest" {
                    format!("{} notes ingested", run["notes_ingested"].as_u64().unwrap_or(0))
                } else {
                    format!(
                        "{} files, commit {}",
                        run["files_written"].as_u64().unwrap_or(0),
                        short_sha(&run["commit"]),
                    )
                };
                t.add_row(vec![
                    comfy_table::Cell::new(run["kind"].as_str().unwrap_or("?")),
                    comfy_table::Cell::new(rel_time(&run["started_at"])),
                    ux::table::status_cell(status),
                    comfy_table::Cell::new(detail),
                ]);
            }
            println!("{t}");
        }
        Op::Url { name, with_token } => {
            // Verify it exists (and 404 nicely) before printing.
            let _ = client::get_json(&cfg, &format!("/v1/brain/{name}")).await?;
            if with_token {
                let token = cfg
                    .token
                    .clone()
                    .ok_or_else(|| anyhow!("no token configured (pensieve connect / PENSIEVE_TOKEN)"))?;
                eprintln!(
                    "{}",
                    ux::theme::warn("warning: URL contains your token — it will land in shell history")
                );
                let base = cfg.endpoint.trim_end_matches('/');
                let with_creds = base.replacen("://", &format!("://pensieve:{token}@"), 1);
                println!("{with_creds}/git/{name}.git");
            } else {
                println!("{}", clone_url(&cfg, &name));
            }
        }
        Op::Clone { name, dir } => {
            let _ = client::get_json(&cfg, &format!("/v1/brain/{name}")).await?;
            let target = dir.unwrap_or_else(|| name.clone());
            let url = clone_url(&cfg, &name);
            // Repo-local credential helper: token never lands in .git/config
            // or shell history; pull/push keep working from this clone.
            let helper = format!(
                "!{} git-credential",
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.to_str().map(str::to_string))
                    .unwrap_or_else(|| "pensieve".to_string())
            );
            let status = std::process::Command::new("git")
                .args(["clone", "--config", &format!("credential.helper={helper}"), &url, &target])
                .status()
                .context("running git clone (is git installed?)")?;
            if !status.success() {
                bail!("git clone failed");
            }
            println!(
                "{} cloned into ./{target} — plain `git pull`/`git push` work from here",
                ux::theme::success("✓")
            );
        }
        Op::Delete { name, yes } => {
            if !yes {
                eprintln!(
                    "This deletes the published repo `{name}` (memories are NOT deleted)."
                );
                eprint!("Type the brain name to confirm: ");
                use std::io::BufRead as _;
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                if line.trim() != name {
                    bail!("aborted");
                }
            }
            let resp =
                client::delete_json(&cfg, &format!("/v1/brain/{name}?purge=true")).await?;
            println!(
                "{} brain `{name}` deleted (repo purged: {}) — memories untouched",
                ux::theme::success("✓"),
                resp["repo_purged"].as_bool().unwrap_or(false)
            );
        }
    }
    Ok(())
}

/// `pensieve git-credential` — a git credential helper (the `get` action of the
/// git-credential protocol). Answers with the configured pensieve endpoint's
/// token when the requested host matches, so clones wired with
/// `credential.helper=!pensieve git-credential` authenticate without tokens in
/// URLs. Non-`get` actions (store/erase) are accepted and ignored.
pub(crate) async fn run_git_credential(action: Option<String>) -> Result<()> {
    if action.as_deref() != Some("get") {
        return Ok(());
    }
    use std::io::BufRead as _;
    let mut host = String::new();
    let mut protocol = String::new();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("host=") {
            host = v.to_string();
        } else if let Some(v) = line.strip_prefix("protocol=") {
            protocol = v.to_string();
        }
    }
    let cfg = client::effective_config()?;
    let endpoint =
        reqwest::Url::parse(&cfg.endpoint).context("parsing configured endpoint")?;
    let cfg_host = match (endpoint.host_str(), endpoint.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => bail!("configured endpoint has no host"),
    };
    if host != cfg_host || endpoint.scheme() != protocol {
        // Not our server — stay silent so git falls through to other helpers.
        return Ok(());
    }
    let Some(token) = cfg.token else { return Ok(()) };
    println!("username=pensieve");
    println!("password={token}");
    Ok(())
}
