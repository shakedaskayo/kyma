//! Optional OS background worker: `pensieve worker install` registers a user
//! service that runs `pensieve sync --watch` with no terminal or session —
//! launchd LaunchAgent on macOS, systemd user unit on Linux.
//!
//! Strictly opt-in (nothing is installed by default), fully reversible
//! (`pensieve worker uninstall`), and inspectable (`pensieve worker status`). The
//! service writes its output to `~/.pensieve/logs/worker.log`; the sync loop
//! itself is the same one the CLI runs, with all `PENSIEVE_CC_*` knobs honored.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// Service label / unit name.
const LABEL: &str = "dev.getpensieve.pensieve-sync";
const UNIT: &str = "pensieve-sync.service";

/// What the worker should run.
#[derive(Debug, Clone, Default)]
pub struct WorkerOptions {
    /// Poll interval override (`PENSIEVE_CC_SYNC_POLL_SECS` for the service).
    pub interval_secs: Option<u64>,
    /// Only the Claude Code file phase.
    pub cc_only: bool,
    /// Only the control-plane push/pull.
    pub cloud_only: bool,
    /// Store location pinned into the service env (services don't inherit
    /// the shell's `PENSIEVE_HOME`). Filled by [`install`] when unset.
    pub pensieve_home: Option<String>,
}

/// `(key, value)` env pairs the service needs.
fn service_env(opts: &WorkerOptions) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(h) = &opts.pensieve_home {
        env.push(("PENSIEVE_HOME".to_string(), h.clone()));
    }
    if let Some(secs) = opts.interval_secs {
        env.push(("PENSIEVE_CC_SYNC_POLL_SECS".to_string(), secs.to_string()));
    }
    env
}

/// The `pensieve` argv the service runs (after the binary path).
pub(crate) fn sync_args(opts: &WorkerOptions) -> Vec<String> {
    let mut args = vec!["sync".to_string(), "--watch".to_string()];
    if opts.cc_only {
        args.push("--cc-only".to_string());
    }
    if opts.cloud_only {
        args.push("--cloud-only".to_string());
    }
    args
}

/// Render the macOS LaunchAgent plist.
pub(crate) fn launchd_plist(exe: &str, opts: &WorkerOptions, log_path: &str) -> String {
    use std::fmt::Write as _;
    let mut argv = String::new();
    let _ = writeln!(argv, "    <string>{exe}</string>");
    for a in sync_args(opts) {
        let _ = writeln!(argv, "    <string>{a}</string>");
    }
    let pairs = service_env(opts);
    let env = if pairs.is_empty() {
        String::new()
    } else {
        let mut block = String::from("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (k, v) in &pairs {
            let _ = writeln!(block, "    <key>{k}</key>\n    <string>{v}</string>");
        }
        block.push_str("  </dict>\n");
        block
    };
    // Working dir = the store: relative caches (e.g. fastembed's model dir)
    // land somewhere stable and writable, not launchd's default `/`.
    let cwd = opts.pensieve_home.clone().unwrap_or_else(home);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{argv}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>WorkingDirectory</key>
  <string>{cwd}</string>
  <key>StandardOutPath</key>
  <string>{log_path}</string>
  <key>StandardErrorPath</key>
  <string>{log_path}</string>
{env}</dict>
</plist>
"#
    )
}

/// Render the Linux systemd user unit.
pub(crate) fn systemd_unit(exe: &str, opts: &WorkerOptions) -> String {
    let env: String = service_env(opts)
        .iter()
        .map(|(k, v)| format!("Environment={k}={v}\n"))
        .collect();
    let cwd = opts.pensieve_home.clone().unwrap_or_else(home);
    format!(
        "[Unit]\nDescription=pensieve memory sync worker (Claude Code files + control plane)\n\n\
         [Service]\nExecStart={exe} {}\nWorkingDirectory={cwd}\nRestart=on-failure\nRestartSec=10\n{env}\n\
         [Install]\nWantedBy=default.target\n",
        sync_args(opts).join(" "),
    )
}

/// `~/Library/LaunchAgents/<label>.plist`.
pub(crate) fn plist_path(home: &str) -> PathBuf {
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// `~/.config/systemd/user/pensieve-sync.service`.
pub(crate) fn unit_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(".config/systemd/user").join(UNIT)
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

fn log_path() -> String {
    let pensieve_home = std::env::var("PENSIEVE_HOME").unwrap_or_else(|_| format!("{}/.pensieve", home()));
    format!("{pensieve_home}/logs/worker.log")
}

/// Best-effort command; returns whether it succeeded.
fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Install + activate the worker for the current OS.
pub fn install(opts: &WorkerOptions) -> Result<()> {
    let exe = std::env::current_exe()
        .context("resolving the pensieve binary path")?
        .to_string_lossy()
        .to_string();
    let mut opts = opts.clone();
    if opts.pensieve_home.is_none() {
        // Pin the resolved store so the service sees the same data the CLI
        // does, custom PENSIEVE_HOME included.
        opts.pensieve_home =
            Some(std::env::var("PENSIEVE_HOME").unwrap_or_else(|_| format!("{}/.pensieve", home())));
    }
    let opts = &opts;
    let log = log_path();
    if let Some(parent) = std::path::Path::new(&log).parent() {
        std::fs::create_dir_all(parent).ok();
    }

    match std::env::consts::OS {
        "macos" => {
            let path = plist_path(&home());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, launchd_plist(&exe, opts, &log))
                .with_context(|| format!("writing {}", path.display()))?;
            let target = format!("gui/{}", uid());
            // Re-registering an existing agent needs a bootout first (ignore
            // failures: it may simply not be loaded yet).
            run("launchctl", &["bootout", &format!("{target}/{LABEL}")]);
            let loaded = run(
                "launchctl",
                &["bootstrap", &target, &path.display().to_string()],
            ) || run("launchctl", &["load", "-w", &path.display().to_string()]);
            // RunAtLoad isn't always honored on re-bootstrap — start it now.
            run("launchctl", &["kickstart", &format!("{target}/{LABEL}")]);
            eprintln!("worker installed: {}", path.display());
            if loaded {
                eprintln!("worker running (launchd, label {LABEL}); logs: {log}");
            } else {
                eprintln!(
                    "couldn't activate automatically — run:\n  launchctl bootstrap {target} {}",
                    path.display()
                );
            }
        }
        "linux" => {
            let path = unit_path(&home());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, systemd_unit(&exe, opts))
                .with_context(|| format!("writing {}", path.display()))?;
            run("systemctl", &["--user", "daemon-reload"]);
            let started = run("systemctl", &["--user", "enable", "--now", UNIT]);
            eprintln!("worker installed: {}", path.display());
            if started {
                eprintln!("worker running (systemd --user, {UNIT}); logs: {log}");
            } else {
                eprintln!(
                    "couldn't activate automatically — run:\n  systemctl --user enable --now {UNIT}"
                );
            }
        }
        other => {
            eprintln!(
                "no service installer for {other} — run the loop yourself:\n  {exe} {}",
                sync_args(opts).join(" ")
            );
        }
    }
    Ok(())
}

/// Deactivate + remove the worker.
pub fn uninstall() -> Result<()> {
    match std::env::consts::OS {
        "macos" => {
            let path = plist_path(&home());
            run("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
            run("launchctl", &["unload", "-w", &path.display().to_string()]);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                eprintln!("worker removed: {}", path.display());
            } else {
                eprintln!("worker not installed");
            }
        }
        "linux" => {
            let path = unit_path(&home());
            run("systemctl", &["--user", "disable", "--now", UNIT]);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                run("systemctl", &["--user", "daemon-reload"]);
                eprintln!("worker removed: {}", path.display());
            } else {
                eprintln!("worker not installed");
            }
        }
        other => eprintln!("no service installer for {other} — nothing to remove"),
    }
    Ok(())
}

/// Installed/running state of the background worker — the programmatic form
/// of [`status`], reused by `pensieve status` (top-level) so worker state isn't
/// only discoverable via the separate `pensieve worker status` command.
#[derive(Debug, Clone, Default)]
pub struct WorkerState {
    /// `None` on platforms without a service installer.
    pub installed: Option<bool>,
    pub running: bool,
    /// Where the service definition lives (plist/unit path), when supported.
    pub service_path: Option<PathBuf>,
}

/// Probe the current OS's service manager for the worker's state.
pub fn probe() -> WorkerState {
    match std::env::consts::OS {
        "macos" => {
            let path = plist_path(&home());
            let installed = path.exists();
            // `print` succeeds for any loaded label — parse the actual state.
            let running = Command::new("launchctl")
                .args(["print", &format!("gui/{}/{LABEL}", uid())])
                .output()
                .map(|o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout).contains("state = running")
                })
                .unwrap_or(false);
            WorkerState {
                installed: Some(installed),
                running,
                service_path: Some(path),
            }
        }
        "linux" => {
            let path = unit_path(&home());
            let installed = path.exists();
            let running = run("systemctl", &["--user", "is-active", "--quiet", UNIT]);
            WorkerState {
                installed: Some(installed),
                running,
                service_path: Some(path),
            }
        }
        _ => WorkerState::default(),
    }
}

/// Report whether the worker is installed/running and where it logs.
pub fn status() -> Result<()> {
    let state = probe();
    match (state.installed, &state.service_path) {
        (Some(installed), Some(path)) => eprintln!(
            "worker: {} ({}), {}",
            if installed { "installed" } else { "not installed" },
            path.display(),
            if state.running { "running" } else { "not running" },
        ),
        _ => eprintln!("no service installer for {}", std::env::consts::OS),
    }
    eprintln!("logs: {}", log_path());
    Ok(())
}
