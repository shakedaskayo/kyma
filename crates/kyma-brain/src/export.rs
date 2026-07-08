//! Export orchestration: plan the vault, preserve non-exporter files, and
//! publish one fast-import commit (or roll back to a no-op).
//!
//! The critical invariant: a full-state `deleteall` commit MUST carry every
//! file the exporter does not own, or user-pushed files (images, personal
//! notes) would be destroyed. Ownership is recorded in
//! `.kyma/manifest.json`; anything in HEAD but absent from the previous
//! manifest is preserved verbatim. On a path conflict the generated file
//! wins — the pushed content was already ingested into memory, which is the
//! source of truth.

use std::path::Path;

use crate::gitbin::GitBin;
use crate::registry::BrainConfig;
use crate::types::{EdgeRow, Manifest, NoteRow, VaultFile};
use crate::vault::{plan_vault, seed_files, VaultPlan};
use crate::{BrainError, BRAIN_BRANCH, MANIFEST_PATH};

/// Result of one export pass.
#[derive(Debug, Clone)]
pub struct ExportOutcome {
    pub commit: String,
    pub noop: bool,
    pub files_written: u64,
    pub note_count: u64,
    /// Non-exporter paths carried over from HEAD.
    pub files_preserved: u64,
}

/// Read the previous manifest from the branch tip. Missing branch or
/// missing/corrupt manifest → empty manifest (first run / recovery: the
/// next pass simply claims only what it generates and preserves the rest).
pub async fn read_prior_manifest(
    git: &GitBin,
    repo: &Path,
    head: Option<&str>,
) -> Result<Manifest, BrainError> {
    let Some(head) = head else { return Ok(Manifest::default()) };
    match git.cat_file(repo, head, MANIFEST_PATH).await {
        Ok(bytes) => Ok(Manifest::from_bytes(&bytes).unwrap_or_default()),
        Err(_) => Ok(Manifest::default()),
    }
}

/// Run one export pass. `nodes`/`edges` are the caller-fetched memory rows
/// for the brain's realm selection. `now_unix` stamps the committer date
/// (display-only; tree bytes carry no clock). `first_run` additionally
/// seeds `.obsidian/` + `.gitignore` + `inbox/` (never touched again).
pub async fn run_export(
    git: &GitBin,
    repo: &Path,
    cfg: &BrainConfig,
    nodes: &[NoteRow],
    edges: &[EdgeRow],
    now_unix: i64,
) -> Result<ExportOutcome, BrainError> {
    let refname = format!("refs/heads/{BRAIN_BRANCH}");
    let head = git.rev_parse(repo, &refname).await?;
    let prior = read_prior_manifest(git, repo, head.as_deref()).await?;
    let first_run = head.is_none();

    let VaultPlan { mut files, manifest, note_count } = plan_vault(cfg, &prior, nodes, edges)?;

    // Paths whose pushed file was already ingested into memory (topic_key
    // `brain:<name>:<path>`, minted by push-ingest for new files). The
    // canonical note now renders under notes/… so the original pushed file
    // (e.g. in inbox/) is re-filed, not preserved.
    let ingested_prefix = crate::topic_key(&cfg.name, "");
    let ingested_paths: std::collections::BTreeSet<&str> = nodes
        .iter()
        .filter_map(|n| n.topic_key.as_deref()?.strip_prefix(ingested_prefix.as_str()))
        .collect();

    // Preserve everything in HEAD that the previous export didn't own.
    let mut preserved = 0u64;
    if let Some(head) = &head {
        let owned_prev = prior.owned_paths();
        let generated: std::collections::BTreeSet<String> =
            files.iter().map(|f| f.path.clone()).collect();
        for path in git.ls_tree_paths(repo, head).await? {
            if owned_prev.contains(&path)
                || generated.contains(&path)
                || ingested_paths.contains(path.as_str())
            {
                continue;
            }
            let bytes = git.cat_file(repo, head, &path).await?;
            files.push(VaultFile { path, bytes });
            preserved += 1;
        }
    }

    if first_run {
        let generated: std::collections::BTreeSet<String> =
            files.iter().map(|f| f.path.clone()).collect();
        for f in seed_files(cfg) {
            if !generated.contains(&f.path) {
                files.push(f);
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    let message = format!("export: {note_count} notes ({})", cfg.name);
    let outcome = git
        .fast_import_commit(repo, BRAIN_BRANCH, &files, head.as_deref(), &message, now_unix)
        .await?;

    let _ = manifest; // already serialized inside the plan's files
    Ok(ExportOutcome {
        commit: outcome.commit,
        noop: outcome.noop,
        files_written: files.len() as u64,
        note_count,
        files_preserved: preserved,
    })
}
