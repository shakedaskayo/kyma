//! `kyma update` — self-update from GitHub releases, plus the passive
//! "a newer kyma is available" notice shown by `version`/`status`/`serve`.
//!
//! The web UI is embedded in the binary at compile time, so a stale binary
//! means a stale UI. `kyma update` closes the loop end to end: fetch the
//! latest release, swap the binary in place, and restart the local server
//! (when we started it — `~/.kyma/serve.pid`) so the new UI is live
//! immediately. Browsers revalidate `index.html` (`Cache-Control: no-cache`)
//! and assets are content-hashed, so no client-side cache clearing is needed.

use crate::client::{config_dir, http_client, load_config};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const REPO: &str = "shakedaskayo/kyma";
/// How long a passive check result stays fresh before we hit GitHub again.
const CHECK_TTL_SECS: u64 = 24 * 60 * 60;

// ── GitHub release lookup ───────────────────────────────────────────────────

/// Public wrapper: the latest GitHub release tag (e.g. "v0.1.0").
/// Used by `kyma deploy` to pin the engine image to the release train.
pub(crate) async fn latest_release_tag() -> Result<String> {
    fetch_latest_tag().await
}

async fn fetch_latest_tag() -> Result<String> {
    let mut req = http_client()
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(10));
    // Optional: raises the API rate limit / supports private forks.
    if let Ok(tok) = std::env::var("GITHUB_TOKEN") {
        if !tok.is_empty() {
            req = req.bearer_auth(tok);
        }
    }
    let resp = req.send().await.context("contact github releases API")?;
    if !resp.status().is_success() {
        bail!("GitHub releases API returned {}", resp.status());
    }
    let body: serde_json::Value = resp.json().await.context("parse releases response")?;
    body.get("tag_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("no tag_name in latest release"))
}

/// Parse `v0.1.2` / `0.1.2` / `0.1.2-rc.1` into a comparable key.
/// Pre-releases sort below the plain release of the same triple.
fn parse_version(s: &str) -> Option<(u64, u64, u64, bool)> {
    let s = s.trim().trim_start_matches('v');
    let (core, pre) = match s.split_once('-') {
        Some((c, _)) => (c, true),
        None => (s, false),
    };
    let mut it = core.split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.parse().ok()?;
    let pat = it.next().unwrap_or("0").parse().ok()?;
    Some((maj, min, pat, !pre))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

// ── artifact selection (must match install.sh / release.yml names) ──────────

fn platform_artifact() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => bail!("no prebuilt binaries for {other} — reinstall with install.sh --from-source"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => bail!("no prebuilt binaries for {other} — reinstall with install.sh --from-source"),
    };
    Ok(format!("kyma-{os}-{arch}"))
}

// ── the update itself ───────────────────────────────────────────────────────

pub(crate) async fn run(
    check: bool,
    version: Option<String>,
    force: bool,
    no_restart: bool,
) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let target = match version {
        Some(v) => v,
        None => fetch_latest_tag().await.context("resolve latest release")?,
    };

    if check {
        if is_newer(&target, current) {
            println!("kyma {target} is available (you have v{current}) — run `kyma update`");
        } else {
            println!("kyma v{current} is up to date (latest release: {target})");
        }
        return Ok(());
    }

    if !force && !is_newer(&target, current) {
        println!("kyma v{current} is already up to date (latest release: {target}); use --force to reinstall");
        // Even when the binary is current, a long-running server may predate it.
        if !no_restart {
            restart_stale_server(current).await?;
        }
        return Ok(());
    }

    let artifact = platform_artifact()?;
    let base = format!("https://github.com/{REPO}/releases/download/{target}/{artifact}.tar.gz");

    eprintln!("▸ downloading kyma {target} ({artifact})…");
    let tarball = http_client()
        .get(&base)
        .send()
        .await
        .with_context(|| format!("download {base}"))?
        .error_for_status()
        .with_context(|| format!("download {base}"))?
        .bytes()
        .await
        .context("read release artifact")?;

    // Verify against the published checksum when one exists.
    if let Ok(resp) = http_client().get(format!("{base}.sha256")).send().await {
        if resp.status().is_success() {
            let sum_line = resp.text().await.unwrap_or_default();
            let want = sum_line.split_whitespace().next().unwrap_or("").to_lowercase();
            let got = Sha256::digest(&tarball)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if !want.is_empty() && want != got {
                bail!("checksum mismatch for {artifact}.tar.gz (expected {want}, got {got})");
            }
            eprintln!("▸ checksum OK");
        }
    }

    let new_bin = extract_kyma(&tarball).context("extract kyma from release tarball")?;
    let dest = std::env::current_exe().context("locate current kyma binary")?;
    let dest = std::fs::canonicalize(&dest).unwrap_or(dest);
    replace_binary(&dest, &new_bin)
        .with_context(|| format!("replace {} (try: sudo kyma update)", dest.display()))?;
    println!("✓ kyma v{current} → {target}  ({})", dest.display());

    if !no_restart {
        restart_stale_server(target.trim_start_matches('v')).await?;
    } else {
        eprintln!("note: a running `kyma serve` keeps the old UI until restarted");
    }
    Ok(())
}

fn extract_kyma(tarball: &[u8]) -> Result<Vec<u8>> {
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().and_then(|n| n.to_str()) == Some("kyma") {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    bail!("no `kyma` binary inside the release tarball")
}

/// Atomic-as-possible swap: write next to the destination (same filesystem),
/// match permissions, then rename over. The running process keeps its old
/// inode, so updating a live binary is safe on unix.
fn replace_binary(dest: &Path, contents: &[u8]) -> Result<()> {
    #[cfg(not(unix))]
    bail!("self-update is only supported on macOS/Linux — re-run the installer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let staged = dest.with_file_name(".kyma.update");
        std::fs::write(&staged, contents)
            .with_context(|| format!("write {}", staged.display()))?;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&staged, dest).inspect_err(|_| {
            let _ = std::fs::remove_file(&staged);
        })?;
        Ok(())
    }
}

// ── restart the local server we own (install.sh / serve.pid contract) ──────

/// If a kyma server is reachable on the configured/default local endpoint and
/// reports an older version than `fresh`, restart it — but only when it's
/// loopback AND `~/.kyma/serve.pid` points at a live process (i.e. install.sh
/// or a previous update started it). Anything else gets a printed hint.
async fn restart_stale_server(fresh: &str) -> Result<()> {
    let endpoint = load_config()
        .map(|c| c.endpoint)
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:7777".to_string());

    let Some(running) = probe_version(&endpoint).await else {
        return Ok(()); // nothing running — nothing to restart
    };
    if running == fresh {
        return Ok(());
    }

    // Service-managed server (the default since install.sh switched to
    // `kyma service install`): a kick reloads the swapped binary. Killing the
    // pid instead would just make launchd/systemd respawn the OLD process
    // image's restart loop semantics — go through the supervisor.
    if let Some(ok) = kyma_local::server_service::restart_if_installed() {
        if ok {
            eprintln!("▸ restarted the supervised server (v{running} → v{fresh})");
            return Ok(());
        }
        eprintln!("! couldn't restart the server service — run: kyma service status");
        return Ok(());
    }

    let loopback = endpoint.contains("127.0.0.1") || endpoint.contains("localhost");
    let pid = read_live_serve_pid();
    if !loopback || pid.is_none() {
        eprintln!(
            "! a kyma server at {endpoint} is still running v{running} — restart it to load the new UI"
        );
        return Ok(());
    }
    let pid = pid.unwrap();

    eprintln!("▸ restarting the local server (v{running} → v{fresh})…");
    let _ = std::process::Command::new("kill")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .status();
    for _ in 0..20 {
        if !pid_alive(pid) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    if pid_alive(pid) {
        bail!("old server (pid {pid}) didn't exit; restart it manually");
    }

    // Mirror install.sh: background `kyma serve`, same addr, static token from
    // the saved CLI config so existing `kyma connect` credentials keep working.
    // `--addr` is a SocketAddr — it takes IPs, not hostnames.
    let addr = endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .replace("localhost", "127.0.0.1");
    let exe = std::env::current_exe().context("locate kyma binary")?;
    let dir = config_dir()?;
    let log = dir.join("serve.log");
    let mut env_prefix = String::new();
    if let Some(token) = load_config().ok().and_then(|c| c.token) {
        env_prefix = format!("KYMA_AUTH_TOKENS='{}:admin' ", token.replace('\'', ""));
    }
    let cmd = format!(
        "{env_prefix}nohup '{}' serve --addr '{addr}' >>'{}' 2>&1 & echo $!",
        exe.display(),
        log.display()
    );
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .context("spawn kyma serve")?;
    let new_pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !new_pid.is_empty() {
        let _ = std::fs::write(dir.join("serve.pid"), &new_pid);
    }
    let new_pid: Option<u32> = new_pid.parse().ok();

    for _ in 0..60 {
        if probe_version(&endpoint).await.as_deref() == Some(fresh) {
            println!("✓ local server restarted on {endpoint} (v{fresh}) — reload the web UI");
            return Ok(());
        }
        // Spawn died (bad flag, port conflict, …) — fail fast with the log.
        if let Some(p) = new_pid {
            if !pid_alive(p) {
                bail!(
                    "restarted server exited immediately:\n{}",
                    tail_of(&log, 5).unwrap_or_default()
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    bail!("restarted server didn't become healthy; see {}", log.display())
}

async fn probe_version(endpoint: &str) -> Option<String> {
    let url = format!("{}/health", endpoint.trim_end_matches('/'));
    let resp = http_client()
        .get(url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("version").and_then(|v| v.as_str()).map(str::to_string)
}

fn read_live_serve_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(config_dir().ok()?.join("serve.pid"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    pid_alive(pid).then_some(pid)
}

fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tail_of(path: &Path, n: usize) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = raw.lines().collect();
    let start = lines.len().saturating_sub(n);
    Some(lines[start..].join("\n"))
}

// ── passive update notice ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct CheckCache {
    checked_at: u64,
    latest: String,
}

fn cache_path() -> Option<PathBuf> {
    config_dir().ok().map(|d| d.join("update-check.json"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Print a one-line nudge on stderr when a newer release exists. Throttled to
/// one GitHub hit per day via `~/.kyma/update-check.json`; never blocks longer
/// than the short HTTP timeout; silent on any failure. Opt out with
/// `KYMA_NO_UPDATE_CHECK=1`.
pub(crate) async fn maybe_notify_update() {
    if std::env::var_os("KYMA_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let latest = match cached_or_fetch_latest().await {
        Some(v) => v,
        None => return,
    };
    let current = env!("CARGO_PKG_VERSION");
    if is_newer(&latest, current) {
        eprintln!("\n💡 kyma {latest} is available (you have v{current}) — run `kyma update`");
    }
}

async fn cached_or_fetch_latest() -> Option<String> {
    let path = cache_path()?;
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(c) = serde_json::from_str::<CheckCache>(&raw) {
            if now_secs().saturating_sub(c.checked_at) < CHECK_TTL_SECS && !c.latest.is_empty() {
                return Some(c.latest);
            }
        }
    }
    let latest = fetch_latest_tag().await.ok()?;
    let _ = std::fs::create_dir_all(path.parent()?);
    let _ = std::fs::write(
        &path,
        serde_json::to_string(&CheckCache {
            checked_at: now_secs(),
            latest: latest.clone(),
        })
        .ok()?,
    );
    Some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_and_ordering() {
        assert!(is_newer("v0.0.2", "0.0.1"));
        assert!(is_newer("0.1.0", "0.0.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("v0.0.1", "0.0.1"));
        assert!(!is_newer("v0.0.1", "0.0.2"));
        // Pre-releases sort below the release of the same triple.
        assert!(!is_newer("v0.0.2-rc.1", "0.0.2"));
        assert!(is_newer("v0.0.2", "0.0.2-rc.1"));
        // Garbage never triggers an update.
        assert!(!is_newer("nightly", "0.0.1"));
    }

    #[test]
    fn artifact_matches_release_naming() {
        // Whatever platform CI runs on, the name must match release.yml's
        // kyma-{darwin|linux}-{x64|arm64} scheme.
        let a = platform_artifact().unwrap();
        assert!(a.starts_with("kyma-"));
        assert!(a.ends_with("-x64") || a.ends_with("-arm64"));
    }
}
