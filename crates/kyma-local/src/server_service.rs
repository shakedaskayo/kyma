//! The local server as an OS background service: `kyma service install`
//! registers a user service that runs `kyma serve` with no terminal or
//! session — launchd LaunchAgent on macOS, systemd user unit on Linux.
//!
//! This is what makes a fresh install *stay* up: the nohup'd process the
//! installer used to spawn died on reboot and never restarted on crash.
//! The service starts at login (`RunAtLoad`), restarts on failure
//! (`KeepAlive` / `Restart=on-failure`), and logs to
//! `~/.kyma/logs/server.log`. Fully reversible (`kyma service uninstall`)
//! and inspectable (`kyma service status`).

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

/// Service label / unit name.
const LABEL: &str = "dev.getkyma.kyma-server";
const UNIT: &str = "kyma-server.service";

/// What the server service should run.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Listen address for `kyma serve`.
    pub addr: String,
    /// Static admin token (`KYMA_AUTH_TOKENS=<token>:admin` in the service
    /// env). None = auth-disabled local default.
    pub token: Option<String>,
    /// Store location pinned into the service env (services don't inherit
    /// the shell's `KYMA_HOME`). Filled by [`install`] when unset.
    pub kyma_home: Option<String>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:7777".to_string(),
            token: None,
            kyma_home: None,
        }
    }
}

/// `(key, value)` env pairs the service needs.
fn service_env(opts: &ServerOptions) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(h) = &opts.kyma_home {
        env.push(("KYMA_HOME".to_string(), h.clone()));
    }
    if let Some(tok) = &opts.token {
        env.push(("KYMA_AUTH_TOKENS".to_string(), format!("{tok}:admin")));
    }
    env
}

/// The `kyma` argv the service runs (after the binary path).
pub(crate) fn serve_args(opts: &ServerOptions) -> Vec<String> {
    vec![
        "serve".to_string(),
        "--addr".to_string(),
        opts.addr.clone(),
    ]
}

/// Render the macOS LaunchAgent plist.
pub(crate) fn launchd_plist(exe: &str, opts: &ServerOptions, log_path: &str) -> String {
    use std::fmt::Write as _;
    let mut argv = String::new();
    let _ = writeln!(argv, "    <string>{exe}</string>");
    for a in serve_args(opts) {
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
    let cwd = opts.kyma_home.clone().unwrap_or_else(home);
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
pub(crate) fn systemd_unit(exe: &str, opts: &ServerOptions) -> String {
    let env: String = service_env(opts)
        .iter()
        .map(|(k, v)| format!("Environment={k}={v}\n"))
        .collect();
    let cwd = opts.kyma_home.clone().unwrap_or_else(home);
    format!(
        "[Unit]\nDescription=kyma local server (web UI + API + background workers)\n\n\
         [Service]\nExecStart={exe} {}\nWorkingDirectory={cwd}\nRestart=on-failure\nRestartSec=5\n{env}\n\
         [Install]\nWantedBy=default.target\n",
        serve_args(opts).join(" "),
    )
}

/// `~/Library/LaunchAgents/<label>.plist`.
pub(crate) fn plist_path(home: &str) -> PathBuf {
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// `~/.config/systemd/user/kyma-server.service`.
pub(crate) fn unit_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(".config/systemd/user").join(UNIT)
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
}

fn log_path() -> String {
    let kyma_home = std::env::var("KYMA_HOME").unwrap_or_else(|_| format!("{}/.kyma", home()));
    format!("{kyma_home}/logs/server.log")
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

/// Install + activate the server service for the current OS. Returns
/// `Ok(true)` when a service manager took over, `Ok(false)` when this OS has
/// no supported one (caller may fall back to a plain background process).
pub fn install(opts: &ServerOptions) -> Result<bool> {
    let exe = std::env::current_exe()
        .context("resolving the kyma binary path")?
        .to_string_lossy()
        .to_string();
    let mut opts = opts.clone();
    if opts.kyma_home.is_none() {
        // Pin the resolved store so the service sees the same data the CLI
        // does, custom KYMA_HOME included.
        opts.kyma_home =
            Some(std::env::var("KYMA_HOME").unwrap_or_else(|_| format!("{}/.kyma", home())));
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
            // The plist carries the admin token — keep it user-readable only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
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
            eprintln!("server service installed: {}", path.display());
            if loaded {
                eprintln!("server running (launchd, label {LABEL}); survives crash + login. logs: {log}");
            } else {
                eprintln!(
                    "couldn't activate automatically — run:\n  launchctl bootstrap {target} {}",
                    path.display()
                );
            }
            Ok(true)
        }
        "linux" => {
            let path = unit_path(&home());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, systemd_unit(&exe, opts))
                .with_context(|| format!("writing {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            run("systemctl", &["--user", "daemon-reload"]);
            let started = run("systemctl", &["--user", "enable", "--now", UNIT]);
            eprintln!("server service installed: {}", path.display());
            if started {
                eprintln!("server running (systemd --user, {UNIT}); survives crash + login. logs: {log}");
                eprintln!("tip: `loginctl enable-linger $USER` keeps it running after logout too.");
            } else {
                eprintln!(
                    "couldn't activate automatically — run:\n  systemctl --user enable --now {UNIT}"
                );
            }
            Ok(true)
        }
        other => {
            eprintln!("no service manager support for {other} — falling back to a plain process");
            Ok(false)
        }
    }
}

/// Restart the service when (and only when) it is installed — used by
/// `kyma update` after swapping the binary so the new build goes live.
/// `None` = not service-managed; `Some(success)` = restart attempted.
pub fn restart_if_installed() -> Option<bool> {
    match std::env::consts::OS {
        "macos" => {
            if !plist_path(&home()).exists() {
                return None;
            }
            Some(run(
                "launchctl",
                &["kickstart", "-k", &format!("gui/{}/{LABEL}", uid())],
            ))
        }
        "linux" => {
            if !unit_path(&home()).exists() {
                return None;
            }
            Some(run("systemctl", &["--user", "restart", UNIT]))
        }
        _ => None,
    }
}

/// Deactivate + remove the server service.
pub fn uninstall() -> Result<()> {
    match std::env::consts::OS {
        "macos" => {
            let path = plist_path(&home());
            run("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
            run("launchctl", &["unload", "-w", &path.display().to_string()]);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                eprintln!("server service removed: {}", path.display());
            } else {
                eprintln!("server service not installed");
            }
        }
        "linux" => {
            let path = unit_path(&home());
            run("systemctl", &["--user", "disable", "--now", UNIT]);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
                run("systemctl", &["--user", "daemon-reload"]);
                eprintln!("server service removed: {}", path.display());
            } else {
                eprintln!("server service not installed");
            }
        }
        other => eprintln!("no service manager support for {other} — nothing to remove"),
    }
    Ok(())
}

/// Report whether the server service is installed/running and where it logs.
pub fn status() -> Result<()> {
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
            eprintln!(
                "server service: {} ({}), {}",
                if installed { "installed" } else { "not installed" },
                path.display(),
                if running { "running" } else { "not running" },
            );
        }
        "linux" => {
            let path = unit_path(&home());
            let installed = path.exists();
            let running = run("systemctl", &["--user", "is-active", "--quiet", UNIT]);
            eprintln!(
                "server service: {} ({}), {}",
                if installed { "installed" } else { "not installed" },
                path.display(),
                if running { "running" } else { "not running" },
            );
        }
        other => eprintln!("no service manager support for {other}"),
    }
    eprintln!("logs: {}", log_path());
    Ok(())
}
