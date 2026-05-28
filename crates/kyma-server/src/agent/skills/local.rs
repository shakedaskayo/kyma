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

fn collect_plugins(root: &Path, out: &mut Vec<SkillSpec>, seen: &mut HashSet<String>) {
    // root/<plugin>/<version>/skills/
    let Ok(plugins) = std::fs::read_dir(root) else {
        return;
    };
    for plugin in plugins.flatten() {
        let Ok(versions) = std::fs::read_dir(plugin.path()) else {
            continue;
        };
        for version in versions.flatten() {
            let skills_dir = version.path().join("skills");
            collect_dir(&skills_dir, SkillSource::Plugin, out, seen);
        }
    }
}

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
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            // Look for SKILL.md inside.
            let canonical = path.join("SKILL.md");
            if canonical.is_file() {
                if let Some(spec) = parse_skill(&canonical, source) {
                    if seen.insert(spec.name.clone()) {
                        out.push(spec);
                    }
                }
            }
        } else if ft.is_file()
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

/// Parse a single `.md` file. Returns None if it has no recognisable
/// frontmatter and no usable filename stem.
fn parse_skill(path: &Path, source: SkillSource) -> Option<SkillSpec> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (name_fm, description, body) = extract_frontmatter(&raw);

    let name = name_fm
        .or_else(|| {
            // Fall back to the parent directory name if the file is SKILL.md,
            // otherwise the file stem.
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

/// Returns (name, description, body_without_frontmatter). Frontmatter is
/// delimited by `---` lines. We don't pull in a full YAML parser — just
/// look for `name:` and `description:` in the obvious place.
fn extract_frontmatter(raw: &str) -> (Option<String>, Option<String>, String) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (None, None, raw.to_string());
    }
    // Find the second `---` line.
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
}
