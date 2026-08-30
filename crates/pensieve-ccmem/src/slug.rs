//! Claude Code project path slugs and kyma file naming.
//!
//! Claude Code stores per-project state under
//! `~/.claude/projects/<path-slug>/` where the slug is the absolute project
//! path with every non-alphanumeric character replaced by `-`
//! (`/Users/x/proj.app` → `-Users-x-proj-app`). The mapping is lossy, so
//! resolving a slug back to a path goes through the list of known project
//! paths (from `~/.claude.json`), not string surgery.

use std::path::{Path, PathBuf};

/// Slugify an absolute project path the way Claude Code does.
pub fn path_slug(abs: &Path) -> String {
    abs.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Resolve a project-dir slug back to the absolute project path by matching
/// against known project paths (the keys of `~/.claude.json`'s `projects`).
pub fn resolve_project_path(slug_dir_name: &str, known_paths: &[String]) -> Option<PathBuf> {
    known_paths
        .iter()
        .find(|p| path_slug(Path::new(p)) == slug_dir_name)
        .map(PathBuf::from)
}

/// The memory realm for a project path — its basename, matching the
/// `kyma_realm()` convention in the Claude Code plugin hooks.
pub fn realm_for_path(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| "default".to_string(), |s| s.to_string_lossy().into_owned())
}

/// Filename for a kyma-promoted memory: `kyma-<kebab-title>.md`. The `kyma-`
/// prefix namespaces managed files so curation can pre-filter without
/// reading frontmatter.
pub fn memory_filename(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut prev_dash = true; // swallow leading separators
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    format!("kyma-{slug}.md")
}
