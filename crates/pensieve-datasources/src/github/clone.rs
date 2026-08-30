//! Shallow `git clone` source for deep code parsing.
//!
//! Cloning the repo to a temp dir lets the parser read **every** source file
//! from disk in one shot — removing the trees+blobs REST API's per-file cost
//! and per-tick cap. The PAT is supplied to git through an env-backed
//! credential helper, so it never appears in `argv`/process listings.
//!
//! Requires the `git` binary on PATH. Falls back (caller's choice) to the API
//! path when cloning fails.

use std::path::{Path, PathBuf};

use crate::types::DataSourceError;

/// A temporary checkout that removes its directory on drop.
pub struct Checkout {
    pub root: PathBuf,
}

impl Drop for Checkout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Clone `owner/repo` (shallow, single-branch) into a fresh temp directory.
///
/// Blocking (spawns `git`); call from `spawn_blocking` in async code.
pub fn clone_repo(
    token: &str,
    owner: &str,
    repo: &str,
    branch: Option<&str>,
) -> Result<Checkout, DataSourceError> {
    let root = std::env::temp_dir().join(format!(
        "pensieve-gh-{}-{}-{:016x}",
        owner.replace(['/', '\\'], "_"),
        repo.replace(['/', '\\'], "_"),
        fastrand::u64(..),
    ));
    let url = format!("https://github.com/{owner}/{repo}.git");

    let mut cmd = std::process::Command::new("git");
    cmd.env("PENSIEVE_GH_PAT", token)
        // Credential helper reads the PAT from the env — keeps it out of argv.
        .arg("-c")
        .arg("credential.helper=")
        .arg("-c")
        .arg("credential.helper=!f() { echo username=x-access-token; echo \"password=$PENSIEVE_GH_PAT\"; }; f")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--single-branch");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    cmd.arg(&url).arg(&root);

    let out = cmd
        .output()
        .map_err(|e| DataSourceError::Config(format!("git clone spawn failed (is `git` installed?): {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Surface the last, most-specific line; never echo the token.
        let msg = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return Err(DataSourceError::Transient(format!("git clone failed: {msg}")));
    }
    Ok(Checkout { root })
}

/// A source file read from the checkout.
pub struct DiskFile {
    /// Repo-relative path with forward slashes (e.g. `src/main.rs`).
    pub rel_path: String,
    pub size: usize,
    pub content: String,
}

/// Recursively collect files under `root` that `keep(rel_path, size)` accepts.
/// Skips the `.git` directory and stops at `max_files`.
pub fn walk_source_files(
    root: &Path,
    mut keep: impl FnMut(&str, usize) -> bool,
    max_files: usize,
) -> Vec<DiskFile> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let fname = e.file_name();
            if fname == ".git" {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(p);
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            let size = e.metadata().map(|m| m.len() as usize).unwrap_or(0);
            if !keep(&rel, size) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&p) {
                out.push(DiskFile { rel_path: rel, size, content });
                if out.len() >= max_files {
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_filters_and_skips_git() {
        let dir = std::env::temp_dir().join(format!("pensieve-walk-test-{:016x}", fastrand::u64(..)));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/main.py"), "print('hi')\n").unwrap();
        std::fs::write(dir.join("src/util.py"), "x=1\n").unwrap();
        std::fs::write(dir.join("README.md"), "# readme\n").unwrap();
        std::fs::write(dir.join(".git/config"), "secret\n").unwrap();

        let files = walk_source_files(&dir, |p, _sz| p.ends_with(".py"), 100);
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(paths.contains(&"src/main.py"), "got: {:?}", paths);
        assert!(paths.contains(&"src/util.py"), "got: {:?}", paths);
        assert!(!paths.iter().any(|p| p.contains("README")), "md excluded");
        assert!(!paths.iter().any(|p| p.contains(".git")), "git skipped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn walk_respects_max_files() {
        let dir = std::env::temp_dir().join(format!("pensieve-walk-max-{:016x}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..10 {
            std::fs::write(dir.join(format!("f{i}.py")), "x\n").unwrap();
        }
        let files = walk_source_files(&dir, |_, _| true, 3);
        assert_eq!(files.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
