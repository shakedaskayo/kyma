//! Apply curation [`FileAction`]s to a Claude Code project memory dir.
//!
//! The decision engine (`kyma_server::agent::cc_curate`) plans; this module
//! owns the only code that touches `~/.claude/projects/<slug>/memory/`:
//!
//! - **Atomic**: every write is temp-file + rename in the same directory; a
//!   Claude Code session reading mid-pass sees old or new, never torn.
//! - **Never deletes**: archiving moves a file into `memory/archive/` with a
//!   tombstone frontmatter (`archived_at`, `archived_reason`) and the full
//!   body preserved.
//! - **User edits win**: a kyma-authored file whose on-disk body no longer
//!   matches its stamped `content_hash` was edited by the user — it is never
//!   overwritten (ingest pulls the edit back instead).
//! - **Session-safe**: a fresh `.kyma-curate.lock` aborts a concurrent pass;
//!   an active Claude Code session for the project (per
//!   `~/.claude/sessions/*.json`) defers writeback to the next pass.
//! - **Audited**: every pass appends one JSONL line (plan + outcome) to the
//!   audit log; `dry_run` logs without touching anything.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use kyma_server::agent::cc_curate::FileAction;

/// Applier knobs, resolved from env by callers.
pub(crate) struct WritebackConfig {
    /// Plan + log only; no file writes.
    pub dry_run: bool,
    /// Skip writeback when a session for the project was active this
    /// recently.
    pub quiet_window: Duration,
    /// How long a lock file is honored before being considered stale.
    pub lock_ttl: Duration,
    /// `~/.claude/sessions` — active-session detection (None disables).
    pub sessions_dir: Option<PathBuf>,
    /// JSONL audit log path (None disables).
    pub audit_log: Option<PathBuf>,
}

impl Default for WritebackConfig {
    fn default() -> Self {
        WritebackConfig {
            dry_run: false,
            quiet_window: Duration::from_secs(300),
            lock_ttl: Duration::from_secs(300),
            sessions_dir: None,
            audit_log: None,
        }
    }
}

/// What one apply pass did (or would do, under `dry_run`).
#[derive(Debug, Default)]
pub(crate) struct ApplyReport {
    pub written: usize,
    pub archived: usize,
    pub index_updated: bool,
    /// Writes refused because the user edited the target file.
    pub skipped_user_edited: usize,
    /// Pass aborted: another curation pass holds the lock.
    pub skipped_locked: bool,
    /// Pass deferred: a Claude Code session for this project is active.
    pub skipped_quiet: bool,
}

/// Apply a curation plan to `memory_dir`. `project_path` is the project's
/// absolute path (for active-session detection); `now` is RFC3339.
pub(crate) fn apply_actions(
    memory_dir: &Path,
    project_path: Option<&Path>,
    actions: &[FileAction],
    cfg: &WritebackConfig,
    now: &str,
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();

    if session_active(cfg, project_path) {
        report.skipped_quiet = true;
        audit(cfg, memory_dir, actions, &report, now);
        return Ok(report);
    }
    let _lock = if cfg.dry_run {
        None
    } else {
        match acquire_lock(memory_dir, cfg.lock_ttl) {
            Some(guard) => Some(guard),
            None => {
                report.skipped_locked = true;
                audit(cfg, memory_dir, actions, &report, now);
                return Ok(report);
            }
        }
    };

    for action in actions {
        match action {
            FileAction::WriteMemoryFile { file, content, .. } => {
                let target = memory_dir.join(file);
                match write_guard(&target) {
                    WriteVerdict::UserOwned => report.skipped_user_edited += 1,
                    WriteVerdict::Unchanged(existing) if existing == *content => {}
                    _ => {
                        if !cfg.dry_run {
                            atomic_write(&target, content)?;
                        }
                        report.written += 1;
                    }
                }
            }
            FileAction::ArchiveFile { file, reason, .. } => {
                let src = memory_dir.join(file);
                if !src.is_file() {
                    continue; // already gone (deleted, or archived earlier)
                }
                if !cfg.dry_run {
                    archive_file(memory_dir, &src, reason, now)?;
                }
                report.archived += 1;
            }
            FileAction::SetIndex { entries } => {
                let path = memory_dir.join(kyma_ccmem::MEMORY_INDEX_FILE);
                let raw = std::fs::read_to_string(&path).ok();
                let mut idx = raw.as_deref().map_or_else(
                    kyma_ccmem::index::MemoryIndex::new_empty,
                    kyma_ccmem::index::MemoryIndex::parse,
                );
                // Defer to the user: a file they list themselves is not
                // double-listed in the managed region.
                let user_files = idx.user_files();
                let managed: Vec<kyma_ccmem::index::ManagedEntry> = entries
                    .iter()
                    .filter(|e| !user_files.contains(&e.file))
                    // No dead links: an indexed file the user deleted by hand
                    // stays gone. (Writes precede SetIndex in a plan, so
                    // freshly promoted files exist by now; in dry-run they
                    // don't, and nothing is written anyway.)
                    .filter(|e| cfg.dry_run || memory_dir.join(&e.file).is_file())
                    .map(|e| kyma_ccmem::index::ManagedEntry {
                        title: e.title.clone(),
                        file: e.file.clone(),
                        hook: e.hook.clone(),
                    })
                    .collect();
                // Nothing to manage and no markers yet → leave the file
                // alone (don't litter empty marker blocks everywhere).
                let has_markers = raw
                    .as_deref()
                    .is_some_and(|r| r.contains(kyma_ccmem::MANAGED_BEGIN));
                if managed.is_empty() && !has_markers {
                    continue;
                }
                idx.set_managed(managed);
                let rendered = idx.render();
                if Some(rendered.as_str()) != raw.as_deref() {
                    if !cfg.dry_run {
                        atomic_write(&path, &rendered)?;
                    }
                    report.index_updated = true;
                }
            }
        }
    }

    audit(cfg, memory_dir, actions, &report, now);
    Ok(report)
}

enum WriteVerdict {
    /// Target absent → plain write.
    Fresh,
    /// Target is an untouched kyma file with this exact content.
    Unchanged(String),
    /// Target was edited by the user (or isn't kyma's at all) — hands off.
    UserOwned,
}

/// The user-edit guard: only overwrite a file that kyma authored AND whose
/// on-disk body still matches its stamped `content_hash`.
fn write_guard(target: &Path) -> WriteVerdict {
    let Ok(raw) = std::fs::read_to_string(target) else {
        return WriteVerdict::Fresh;
    };
    let Some(parsed) = kyma_ccmem::frontmatter::parse(&raw) else {
        return WriteVerdict::UserOwned; // unparseable → assume user's
    };
    if !parsed.is_kyma_authored() {
        return WriteVerdict::UserOwned;
    }
    let on_disk = kyma_ccmem::hash::content_hash(
        parsed.front.name.as_deref().unwrap_or_default(),
        parsed.front.cc_type.as_deref(),
        &parsed.body,
    );
    if parsed.front.content_hash.as_deref() == Some(on_disk.as_str()) {
        WriteVerdict::Unchanged(raw)
    } else {
        WriteVerdict::UserOwned
    }
}

/// Move `src` into `memory/archive/` with tombstone frontmatter. The copy is
/// durably in place before the original is removed — never lossy.
fn archive_file(memory_dir: &Path, src: &Path, reason: &str, now: &str) -> Result<()> {
    let raw = std::fs::read_to_string(src)?;
    let tombstone = match kyma_ccmem::frontmatter::parse(&raw) {
        Some(mut parsed) => {
            parsed.front.archived_at = Some(now.to_string());
            parsed.front.archived_reason = Some(reason.to_string());
            kyma_ccmem::frontmatter::render(&parsed)
        }
        // Unparseable file: move it as-is rather than inventing frontmatter.
        None => raw,
    };
    let archive_dir = memory_dir.join(kyma_ccmem::ARCHIVE_DIR);
    std::fs::create_dir_all(&archive_dir)?;
    let dest = archive_dir.join(src.file_name().unwrap_or_default());
    atomic_write(&dest, &tombstone)?;
    std::fs::remove_file(src)?;
    Ok(())
}

/// Temp-file + rename in the same directory (atomic on POSIX), fsynced.
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension(format!("kyma-tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Advisory lock: exclusive-create `.kyma-curate.lock`; a fresh lock loses,
/// a stale one (older than the TTL) is reclaimed. Released on drop.
struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn acquire_lock(memory_dir: &Path, ttl: Duration) -> Option<LockGuard> {
    std::fs::create_dir_all(memory_dir).ok()?;
    let path = memory_dir.join(".kyma-curate.lock");
    let try_create = |p: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .is_ok()
    };
    if try_create(&path) {
        return Some(LockGuard(path));
    }
    let stale = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.elapsed().ok())
        .is_some_and(|age| age > ttl);
    if stale {
        let _ = std::fs::remove_file(&path);
        if try_create(&path) {
            return Some(LockGuard(path));
        }
    }
    None
}

/// True when a Claude Code session for `project_path` was active within the
/// quiet window (session metadata files in `~/.claude/sessions`).
fn session_active(cfg: &WritebackConfig, project_path: Option<&Path>) -> bool {
    let (Some(dir), Some(project)) = (cfg.sessions_dir.as_deref(), project_path) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    #[allow(clippy::cast_possible_truncation)]
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    #[allow(clippy::cast_possible_wrap)]
    let window_ms = cfg.quiet_window.as_millis() as i64;
    let project = project.display().to_string();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if v.get("cwd").and_then(serde_json::Value::as_str) != Some(project.as_str()) {
            continue;
        }
        let updated = v
            .get("updatedAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if now_ms.saturating_sub(updated) < window_ms {
            return true;
        }
    }
    false
}

/// Append one JSONL line per pass: the full plan + the outcome.
fn audit(
    cfg: &WritebackConfig,
    memory_dir: &Path,
    actions: &[FileAction],
    report: &ApplyReport,
    now: &str,
) {
    let Some(path) = cfg.audit_log.as_deref() else {
        return;
    };
    let line = serde_json::json!({
        "ts": now,
        "memory_dir": memory_dir.display().to_string(),
        "dry_run": cfg.dry_run,
        "actions": actions,
        "written": report.written,
        "archived": report.archived,
        "index_updated": report.index_updated,
        "skipped_user_edited": report.skipped_user_edited,
        "skipped_locked": report.skipped_locked,
        "skipped_quiet": report.skipped_quiet,
    });
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
