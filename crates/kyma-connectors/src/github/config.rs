//! GitHub connector configuration.

use serde::{Deserialize, Serialize};

/// Which data modules to fetch per repo.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Modules {
    #[serde(default = "bool_true")]
    pub repos: bool,
    #[serde(default = "bool_true")]
    pub branches: bool,
    #[serde(default = "bool_true")]
    pub pulls: bool,
    #[serde(default = "bool_true")]
    pub issues: bool,
    #[serde(default = "bool_true")]
    pub contributors: bool,
    /// Enable structural code-graph ingestion (file tree + imports via
    /// tree-sitter).  Defaults to `true` but only has effect when the
    /// connector is compiled with the `github` feature (which brings in the
    /// tree-sitter grammar crates).
    #[serde(default = "bool_true")]
    pub codebase: bool,
}

impl Default for Modules {
    fn default() -> Self {
        Self {
            repos: true,
            branches: true,
            pulls: true,
            issues: true,
            contributors: true,
            codebase: true,
        }
    }
}

fn bool_true() -> bool {
    true
}

fn default_max_pages() -> usize {
    10
}

// ── CodeOpts defaults ─────────────────────────────────────────────────────────

fn default_languages() -> Vec<String> {
    vec![
        "rust".into(),
        "python".into(),
        "typescript".into(),
        "javascript".into(),
        "go".into(),
    ]
}

fn default_max_file_bytes() -> usize {
    1_048_576 // 1 MiB
}

fn default_max_files_per_tick() -> usize {
    300
}

fn default_exclude_globs() -> Vec<String> {
    vec![
        "**/node_modules/**".into(),
        "**/vendor/**".into(),
        "**/.git/**".into(),
        "**/target/**".into(),
        "**/dist/**".into(),
    ]
}

/// Options controlling how source code is fetched and parsed.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodeOpts {
    /// Languages to ingest (matched by file extension). Defaults to all five
    /// supported languages.
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    /// Skip files larger than this many bytes. Default 1 MiB.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: usize,
    /// Maximum number of files to fetch + parse in a single tick. Default 300.
    #[serde(default = "default_max_files_per_tick")]
    pub max_files_per_tick: usize,
    /// Glob patterns for paths to exclude. Checked against the full
    /// repo-relative path (e.g. `vendor/foo/bar.go`). Default covers the most
    /// common large vendor/generated directories.
    #[serde(default = "default_exclude_globs")]
    pub exclude_globs: Vec<String>,
}

impl Default for CodeOpts {
    fn default() -> Self {
        Self {
            languages: default_languages(),
            max_file_bytes: default_max_file_bytes(),
            max_files_per_tick: default_max_files_per_tick(),
            exclude_globs: default_exclude_globs(),
        }
    }
}

/// Configuration for the GitHub metadata connector.
///
/// Stored as JSON in the `connectors.config_jsonb` column. The `token`
/// field should be a `SecretStore` reference such as `"$env:GITHUB_TOKEN"`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GithubConfig {
    /// SecretStore ref (e.g. `"$env:GITHUB_TOKEN"`) or raw PAT.
    pub token: String,
    /// Repositories to ingest, each as `"owner/name"`.
    pub repos: Vec<String>,
    /// Which modules to fetch (all enabled by default).
    #[serde(default)]
    pub modules: Modules,
    /// Maximum API pages to fetch per module per tick (default 10 → up to
    /// 1 000 items per module per repo per tick with GitHub's 100/page cap).
    #[serde(default = "default_max_pages")]
    pub max_pages_per_tick: usize,
    /// Options for the structural code-graph module.
    #[serde(default)]
    pub code: CodeOpts,
}

impl GithubConfig {
    /// Validate the config, returning a descriptive error string on failure.
    pub fn validate(&self) -> Result<(), String> {
        if self.token.is_empty() {
            return Err("token must not be empty".into());
        }
        if self.repos.is_empty() {
            return Err("repos must contain at least one entry".into());
        }
        for repo in &self.repos {
            let parts: Vec<&str> = repo.splitn(2, '/').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(format!(
                    "repo {repo:?} must be in \"owner/name\" format"
                ));
            }
        }
        if self.max_pages_per_tick == 0 {
            return Err("max_pages_per_tick must be >= 1".into());
        }
        if self.code.max_files_per_tick == 0 {
            return Err("code.max_files_per_tick must be >= 1".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> serde_json::Value {
        serde_json::json!({
            "token": "$env:GITHUB_TOKEN",
            "repos": ["owner/repo"]
        })
    }

    #[test]
    fn valid_config_parses() {
        let cfg: GithubConfig = serde_json::from_value(valid_json()).unwrap();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.max_pages_per_tick, 10);
        assert!(cfg.modules.repos);
    }

    #[test]
    fn empty_repos_is_invalid() {
        let v = serde_json::json!({ "token": "tok", "repos": [] });
        let cfg: GithubConfig = serde_json::from_value(v).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn bad_repo_format_is_invalid() {
        let v = serde_json::json!({ "token": "tok", "repos": ["noslash"] });
        let cfg: GithubConfig = serde_json::from_value(v).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn missing_token_is_invalid() {
        let v = serde_json::json!({ "token": "", "repos": ["a/b"] });
        let cfg: GithubConfig = serde_json::from_value(v).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn unknown_field_rejected() {
        let v = serde_json::json!({ "token": "t", "repos": ["a/b"], "unknown_field": 1 });
        assert!(serde_json::from_value::<GithubConfig>(v).is_err());
    }

    #[test]
    fn modules_all_default_true() {
        let cfg: GithubConfig =
            serde_json::from_value(valid_json()).unwrap();
        assert!(cfg.modules.repos);
        assert!(cfg.modules.branches);
        assert!(cfg.modules.pulls);
        assert!(cfg.modules.issues);
        assert!(cfg.modules.contributors);
        assert!(cfg.modules.codebase);
    }

    #[test]
    fn codebase_module_can_be_disabled() {
        let v = serde_json::json!({
            "token": "tok",
            "repos": ["a/b"],
            "modules": { "codebase": false }
        });
        let cfg: GithubConfig = serde_json::from_value(v).unwrap();
        assert!(!cfg.modules.codebase);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn code_opts_defaults() {
        let cfg: GithubConfig = serde_json::from_value(valid_json()).unwrap();
        assert_eq!(cfg.code.max_file_bytes, 1_048_576);
        assert_eq!(cfg.code.max_files_per_tick, 300);
        assert_eq!(cfg.code.languages.len(), 5);
        assert_eq!(cfg.code.exclude_globs.len(), 5);
    }

    #[test]
    fn code_opts_custom() {
        let v = serde_json::json!({
            "token": "t",
            "repos": ["a/b"],
            "code": {
                "languages": ["rust"],
                "max_file_bytes": 512,
                "max_files_per_tick": 10,
                "exclude_globs": ["**/vendor/**"]
            }
        });
        let cfg: GithubConfig = serde_json::from_value(v).unwrap();
        assert_eq!(cfg.code.languages, vec!["rust"]);
        assert_eq!(cfg.code.max_file_bytes, 512);
        assert_eq!(cfg.code.max_files_per_tick, 10);
        assert_eq!(cfg.code.exclude_globs, vec!["**/vendor/**"]);
    }
}
