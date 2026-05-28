//! Walk the three local skill directories and return parsed SkillSpecs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{SkillSource, SkillSpec};

pub struct LocalSkillSource {
    pub project_dir: Option<PathBuf>,
    pub user_dir: Option<PathBuf>,
    pub plugin_root: Option<PathBuf>,
}

impl LocalSkillSource {
    pub fn new() -> Self {
        let cwd = std::env::current_dir().ok();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self {
            project_dir: cwd.map(|p| p.join(".claude").join("skills")),
            user_dir: home.as_ref().map(|h| h.join(".claude").join("skills")),
            plugin_root: home.map(|h| h.join(".claude").join("plugins").join("cache")),
        }
    }

    pub fn discover(&self) -> Vec<SkillSpec> {
        let mut out: Vec<SkillSpec> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        if let Some(p) = &self.project_dir {
            collect_dir(p, SkillSource::Project, &mut out, &mut seen);
        }
        if let Some(p) = &self.user_dir {
            collect_dir(p, SkillSource::User, &mut out, &mut seen);
        }
        if let Some(root) = &self.plugin_root {
            collect_plugins(root, &mut out, &mut seen);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// Walk `~/.claude/plugins/cache/<vendor>/<plugin>/<version>/skills/`.
///
/// The Claude Code plugin cache nests three levels deep — vendor (e.g.
/// `claude-plugins-official`), plugin (e.g. `superpowers`), version (e.g.
/// `5.1.0`). Underneath each version is a `skills/` directory that follows
/// the same layout as `~/.claude/skills/`.
fn collect_plugins(root: &Path, out: &mut Vec<SkillSpec>, seen: &mut HashSet<String>) {
    let Ok(vendors) = std::fs::read_dir(root) else {
        return;
    };
    for vendor in vendors.flatten() {
        let vendor_path = vendor.path();
        if !is_dir_following_symlinks(&vendor_path) {
            continue;
        }
        let Ok(plugins) = std::fs::read_dir(&vendor_path) else {
            continue;
        };
        for plugin in plugins.flatten() {
            let plugin_path = plugin.path();
            if !is_dir_following_symlinks(&plugin_path) {
                continue;
            }
            let Ok(versions) = std::fs::read_dir(&plugin_path) else {
                continue;
            };
            for version in versions.flatten() {
                let skills_dir = version.path().join("skills");
                collect_dir(&skills_dir, SkillSource::Plugin, out, seen);
            }
        }
    }
}

/// Walk a single `skills/` directory.
///
/// Entries can be:
/// - A directory with a `SKILL.md` inside (the canonical layout used by
///   Claude Code plugins).
/// - A flat `.md` file (occasional simpler layout used by user-installed
///   skills).
/// - A symlink to either of the above (very common in `~/.claude/skills/`
///   — Claude Code links into `~/.agents/skills/<name>` and elsewhere).
///
/// We follow symlinks deliberately: `std::fs::metadata` resolves the
/// target, so `is_dir()` / `is_file()` work for both real entries and
/// symlinks. `read_dir().file_type()` does NOT follow symlinks on macOS
/// and was the cause of the original bug.
fn collect_dir(
    dir: &Path,
    source: SkillSource,
    out: &mut Vec<SkillSpec>,
    seen: &mut HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Use metadata (follows symlinks) instead of file_type (doesn't).
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let canonical = path.join("SKILL.md");
            if canonical.is_file() {
                if let Some(spec) = parse_skill(&canonical, source) {
                    if seen.insert(spec.name.clone()) {
                        out.push(spec);
                    }
                }
            }
        } else if meta.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("md"))
                .unwrap_or(false)
        {
            if let Some(spec) = parse_skill(&path, source) {
                if seen.insert(spec.name.clone()) {
                    out.push(spec);
                }
            }
        }
    }
}

fn is_dir_following_symlinks(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/// Parse a single `.md` file. Returns None if it has no recognisable
/// frontmatter and no usable filename stem.
fn parse_skill(path: &Path, source: SkillSource) -> Option<SkillSpec> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (name_fm, description, body) = extract_frontmatter(&raw);

    let name = name_fm
        .or_else(|| {
            let stem = path.file_stem().and_then(|s| s.to_str())?;
            if stem.eq_ignore_ascii_case("skill") {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                Some(stem.to_string())
            }
        })?;

    Some(SkillSpec {
        name,
        description: description.unwrap_or_default(),
        body,
        source,
        path: path.to_string_lossy().to_string(),
    })
}

fn extract_frontmatter(raw: &str) -> (Option<String>, Option<String>, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (None, None, raw.to_string());
    }
    let after_first = match trimmed.strip_prefix("---") {
        Some(s) => s.trim_start_matches(['\r', '\n']),
        None => return (None, None, raw.to_string()),
    };
    let end_idx = match after_first.find("\n---") {
        Some(i) => i,
        None => return (None, None, raw.to_string()),
    };
    let frontmatter = &after_first[..end_idx];
    let body = &after_first[end_idx + 4..];
    let body = body.trim_start_matches(['\r', '\n']).to_string();

    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("name:") {
            name = Some(rest.trim().trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("description:") {
            description = Some(rest.trim().trim_matches('"').to_string());
        }
    }
    (name, description, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parses_name_and_description() {
        let raw = "---\nname: my-skill\ndescription: when to use it\n---\nbody here\n";
        let (n, d, b) = extract_frontmatter(raw);
        assert_eq!(n.as_deref(), Some("my-skill"));
        assert_eq!(d.as_deref(), Some("when to use it"));
        assert_eq!(b.trim(), "body here");
    }

    #[test]
    fn missing_frontmatter_returns_raw() {
        let raw = "no frontmatter here\nbody\n";
        let (n, d, b) = extract_frontmatter(raw);
        assert_eq!(n, None);
        assert_eq!(d, None);
        assert_eq!(b, raw);
    }

    #[test]
    fn parse_skill_falls_back_to_directory_name_for_skill_md() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("foo-skill");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("SKILL.md");
        std::fs::write(&p, "no frontmatter").unwrap();
        let spec = parse_skill(&p, SkillSource::User).unwrap();
        assert_eq!(spec.name, "foo-skill");
    }

    #[test]
    fn discover_follows_symlinks_to_skill_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        // Real skill dir.
        let real = tmp.path().join("real-skill");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(
            real.join("SKILL.md"),
            "---\nname: real-skill\ndescription: yes\n---\nbody\n",
        )
        .unwrap();
        // Skills index dir containing a SYMLINK to the real skill.
        let index = tmp.path().join("skills");
        std::fs::create_dir_all(&index).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, index.join("via-symlink")).unwrap();
        #[cfg(not(unix))]
        return; // Test skipped on non-unix.

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        collect_dir(&index, SkillSource::User, &mut out, &mut seen);
        assert_eq!(out.len(), 1, "expected exactly one skill via symlink");
        assert_eq!(out[0].name, "real-skill");
    }
}
