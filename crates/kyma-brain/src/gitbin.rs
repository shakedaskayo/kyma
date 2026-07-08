//! Thin async wrapper around the `git` binary — the only git implementation
//! kyma uses (gitoxide has no server side; stateless-rpc + fast-import via
//! the battle-tested binary is the GitLab/gitea pattern).
//!
//! Every invocation gets a scrubbed environment, a wall-clock timeout, and
//! kill-on-drop. Commits are produced with `git fast-import` fed a
//! full-state `deleteall` stream: one child process, no worktrees, and the
//! resulting tree is a pure function of the emitted file set.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::VaultFile;
use crate::{BrainError, BRAIN_BRANCH};

/// Default timeout for plumbing commands.
const PLUMBING_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for fast-import (large first exports).
const IMPORT_TIMEOUT: Duration = Duration::from_secs(600);

/// Outcome of a fast-import commit attempt.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// The commit now at the branch tip (parent when `noop`).
    pub commit: String,
    /// True when the new tree equaled the parent tree and the ref was
    /// rolled back — no commit was published.
    pub noop: bool,
}

/// Change kinds from `git diff --name-status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// Located `git` binary.
#[derive(Debug, Clone)]
pub struct GitBin {
    pub program: PathBuf,
}

impl GitBin {
    /// Locate `git` (honors `KYMA_GIT_BIN`) and verify it runs. `None` when
    /// missing — the brain feature degrades gracefully.
    pub async fn detect() -> Option<Self> {
        let program = std::env::var("KYMA_GIT_BIN").map_or_else(|_| PathBuf::from("git"), PathBuf::from);
        let bin = Self { program };
        match bin.run(Path::new("."), &["version"], PLUMBING_TIMEOUT).await {
            Ok(v) => {
                tracing::info!(git = %String::from_utf8_lossy(&v).trim(), "git binary detected");
                Some(bin)
            }
            Err(e) => {
                tracing::warn!(error = %e, "git binary not found — brain repos disabled");
                None
            }
        }
    }

    fn command(&self, repo: &Path) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.env_clear();
        // git needs PATH (subcommands) and HOME is scrubbed on purpose so
        // user/global config never leaks into server-side repos.
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        cmd.env("GIT_CONFIG_NOSYSTEM", "1");
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GIT_DIR", repo);
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    async fn run(&self, repo: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>, BrainError> {
        self.run_with_stdin(repo, args, None, timeout).await
    }

    async fn run_with_stdin(
        &self,
        repo: &Path,
        args: &[&str],
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, BrainError> {
        let mut cmd = self.command(repo);
        cmd.args(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().map_err(|e| BrainError::Git {
            op: "spawn",
            detail: format!("{}: {e}", self.program.display()),
        })?;
        if let Some(bytes) = stdin {
            let mut pipe = child.stdin.take().ok_or(BrainError::Git {
                op: "spawn",
                detail: "stdin pipe missing".into(),
            })?;
            pipe.write_all(&bytes).await?;
            drop(pipe);
        }
        let out = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| BrainError::Git { op: "timeout", detail: format!("git {args:?}") })??;
        if !out.status.success() {
            return Err(BrainError::Git {
                op: "exec",
                detail: format!(
                    "git {:?} exited {}: {}",
                    args,
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            });
        }
        Ok(out.stdout)
    }

    /// `git init --bare` + the receive policy every brain repo gets.
    pub async fn init_bare(&self, repo: &Path) -> Result<(), BrainError> {
        tokio::fs::create_dir_all(repo).await?;
        // init reads the target as arg, not GIT_DIR
        let mut cmd = self.command(Path::new("."));
        cmd.env_remove("GIT_DIR");
        cmd.args([
            "init",
            "--bare",
            "--initial-branch",
            BRAIN_BRANCH,
            repo.to_str().ok_or_else(|| BrainError::InvalidPath(repo.display().to_string()))?,
        ]);
        let out = tokio::time::timeout(PLUMBING_TIMEOUT, cmd.output())
            .await
            .map_err(|_| BrainError::Git { op: "timeout", detail: "init --bare".into() })??;
        if !out.status.success() {
            return Err(BrainError::Git {
                op: "init",
                detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        for (k, v) in [
            ("receive.denyNonFastForwards", "true"),
            ("receive.denyDeletes", "true"),
            ("core.logAllRefUpdates", "true"),
            ("gc.auto", "0"),
        ] {
            self.set_config(repo, k, v).await?;
        }
        Ok(())
    }

    pub async fn set_config(&self, repo: &Path, key: &str, val: &str) -> Result<(), BrainError> {
        self.run(repo, &["config", key, val], PLUMBING_TIMEOUT).await.map(|_| ())
    }

    /// Resolve a rev to a full sha; `None` when it doesn't exist (fresh repo).
    pub async fn rev_parse(&self, repo: &Path, rev: &str) -> Result<Option<String>, BrainError> {
        match self.run(repo, &["rev-parse", "--verify", "--quiet", rev], PLUMBING_TIMEOUT).await {
            Ok(out) => Ok(Some(String::from_utf8_lossy(&out).trim().to_string())),
            Err(BrainError::Git { op: "exec", .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// All blob paths in a rev's tree.
    pub async fn ls_tree_paths(&self, repo: &Path, rev: &str) -> Result<Vec<String>, BrainError> {
        let out = self
            .run(repo, &["ls-tree", "-r", "-z", "--name-only", rev], PLUMBING_TIMEOUT)
            .await?;
        Ok(String::from_utf8_lossy(&out)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Bytes of a blob at `rev:path`.
    pub async fn cat_file(&self, repo: &Path, rev: &str, path: &str) -> Result<Vec<u8>, BrainError> {
        let spec = format!("{rev}:{path}");
        self.run(repo, &["cat-file", "blob", &spec], PLUMBING_TIMEOUT).await
    }

    /// `git diff --name-status` between two revs (renames disabled so a
    /// rename surfaces as delete + add — exactly what ingest wants).
    pub async fn diff_name_status(
        &self,
        repo: &Path,
        old: &str,
        new: &str,
    ) -> Result<Vec<(ChangeKind, String)>, BrainError> {
        let out = self
            .run(repo, &["diff", "--no-renames", "--name-status", "-z", old, new], PLUMBING_TIMEOUT)
            .await?;
        let text = String::from_utf8_lossy(&out);
        let mut parts = text.split('\0').filter(|s| !s.is_empty());
        let mut changes = Vec::new();
        while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
            let kind = match status.chars().next() {
                Some('A') => ChangeKind::Added,
                Some('M') | Some('T') => ChangeKind::Modified,
                Some('D') => ChangeKind::Deleted,
                _ => continue,
            };
            changes.push((kind, path.to_string()));
        }
        Ok(changes)
    }

    /// Compare-and-swap a ref (`git update-ref <ref> <new> <old>`).
    pub async fn update_ref_cas(
        &self,
        repo: &Path,
        r: &str,
        new: &str,
        old: &str,
    ) -> Result<(), BrainError> {
        self.run(repo, &["update-ref", r, new, old], PLUMBING_TIMEOUT).await.map(|_| ())
    }

    /// Commit a full tree state via one `git fast-import` stream. Returns
    /// `noop: true` (ref rolled back to parent) when the new tree is
    /// identical to the parent's.
    pub async fn fast_import_commit(
        &self,
        repo: &Path,
        branch: &str,
        files: &[VaultFile],
        parent: Option<&str>,
        message: &str,
        author_date_unix: i64,
    ) -> Result<CommitOutcome, BrainError> {
        let mut stream: Vec<u8> = Vec::with_capacity(files.iter().map(|f| f.bytes.len() + 64).sum());
        let push = |s: &mut Vec<u8>, text: &str| s.extend_from_slice(text.as_bytes());
        let refname = format!("refs/heads/{branch}");
        push(&mut stream, &format!("commit {refname}\n"));
        push(
            &mut stream,
            &format!("committer kyma-brain <brain@kyma.local> {author_date_unix} +0000\n"),
        );
        push(&mut stream, &format!("data {}\n{message}\n", message.len() + 1));
        if let Some(p) = parent {
            push(&mut stream, &format!("from {p}\n"));
        }
        push(&mut stream, "deleteall\n");
        for f in files {
            push(&mut stream, &format!("M 100644 inline {}\n", f.path));
            push(&mut stream, &format!("data {}\n", f.bytes.len()));
            stream.extend_from_slice(&f.bytes);
            stream.push(b'\n');
        }
        push(&mut stream, "done\n");

        self.run_with_stdin(repo, &["fast-import", "--quiet", "--done"], Some(stream), IMPORT_TIMEOUT)
            .await?;

        let new = self
            .rev_parse(repo, &refname)
            .await?
            .ok_or(BrainError::Git { op: "fast-import", detail: "branch missing after import".into() })?;

        if let Some(p) = parent {
            let new_tree = self.rev_parse(repo, &format!("{new}^{{tree}}")).await?;
            let old_tree = self.rev_parse(repo, &format!("{p}^{{tree}}")).await?;
            if new_tree.is_some() && new_tree == old_tree {
                self.update_ref_cas(repo, &refname, p, &new).await?;
                return Ok(CommitOutcome { commit: p.to_string(), noop: true });
            }
        }
        Ok(CommitOutcome { commit: new, noop: false })
    }

    /// Spawn a stateless-rpc service child (`upload-pack` / `receive-pack`)
    /// for the smart-HTTP handlers. The caller owns the pipes.
    pub fn spawn_service(
        &self,
        repo: &Path,
        service: &str,
        advertise_refs: bool,
        git_protocol: Option<&str>,
    ) -> Result<tokio::process::Child, BrainError> {
        let sub = match service {
            "git-upload-pack" => "upload-pack",
            "git-receive-pack" => "receive-pack",
            other => {
                return Err(BrainError::Git { op: "service", detail: format!("unsupported: {other}") })
            }
        };
        let mut cmd = Command::new(&self.program);
        cmd.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        cmd.env("GIT_CONFIG_NOSYSTEM", "1");
        if let Some(proto) = git_protocol {
            cmd.env("GIT_PROTOCOL", proto);
        }
        cmd.arg(sub).arg("--stateless-rpc");
        if advertise_refs {
            cmd.arg("--advertise-refs");
        }
        cmd.arg(repo);
        cmd.kill_on_drop(true);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.spawn().map_err(|e| BrainError::Git { op: "spawn", detail: e.to_string() })
    }
}
