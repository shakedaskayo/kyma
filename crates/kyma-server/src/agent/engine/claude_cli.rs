//! Claude CLI engine — spawns the local `claude` binary per turn and
//! streams its answer back as SSE events.
//!
//! Unlike the other engines this one is *not* an `adk_rust::Llm` — Claude
//! Code already owns its own tool loop, MCPs, skills, OAuth, and CLAUDE.md
//! discovery. Asking adk-rust to wrap it would defeat the point. Instead
//! the agent route handler checks `EngineKind::ClaudeCli` first and routes
//! the request through [`run`] below; only for the other kinds does it fall
//! back to the adk-rust runner.
//!
//! Authentication: the CLI handles itself. On macOS it reads the
//! `Claude Code-credentials` Keychain entry that the user already populated
//! by running `claude` interactively at least once. We pass nothing.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Discover the CLI binary on `$PATH`. Returns `None` if `claude` isn't
/// installed — the catalogue uses this to decide whether to advertise the
/// engine in the picker.
pub fn locate_binary() -> Option<std::path::PathBuf> {
    which::which("claude").ok()
}

pub fn default_models() -> Vec<String> {
    // Same shortlist Claude Code itself accepts via --model. The string is
    // forwarded verbatim, so adding a new alias here is purely cosmetic.
    vec![
        "claude-opus-4-7".into(),
        "claude-sonnet-4-6".into(),
        "claude-haiku-4-5".into(),
        "default".into(),
    ]
}

/// One streaming chunk produced by [`run`].
#[derive(Debug)]
pub enum Chunk {
    Stdout(String),
    Stderr(String),
}

/// Spawn `claude --print` with the given prompt + model and stream stdout
/// line-by-line as `Chunk::Stdout`. Stderr is captured and surfaced via
/// `Chunk::Stderr` so transient errors (rate limits, auth failures) make it
/// back to the user instead of being silently dropped.
///
/// The returned receiver closes when the child exits. The exit code is
/// reported via the final `Result` returned by [`wait`] — callers usually
/// stream chunks until the channel closes, then await `wait()`.
pub fn run(
    question: &str,
    model: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> anyhow::Result<(tokio::sync::mpsc::Receiver<Chunk>, tokio::task::JoinHandle<anyhow::Result<i32>>)> {
    let binary = locate_binary()
        .ok_or_else(|| anyhow::anyhow!("`claude` not found on PATH — install Claude Code"))?;

    let mut cmd = Command::new(&binary);
    cmd.arg("--print");
    if let Some(m) = model {
        if m != "default" {
            cmd.arg("--model").arg(m);
        }
    }
    cmd.arg(question);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn `claude`: {e}"))?;

    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no stderr"))?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Chunk>(64);

    let tx_out = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx_out.send(Chunk::Stdout(line)).await.is_err() {
                break;
            }
        }
    });
    let tx_err = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx_err.send(Chunk::Stderr(line)).await.is_err() {
                break;
            }
        }
    });
    drop(tx);

    let wait = tokio::spawn(async move {
        let status = child.wait().await?;
        Ok(status.code().unwrap_or(-1))
    });

    Ok((rx, wait))
}
