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
}

impl Default for Modules {
    fn default() -> Self {
        Self {
            repos: true,
            branches: true,
            pulls: true,
            issues: true,
            contributors: true,
        }
    }
}

fn bool_true() -> bool {
    true
}

fn default_max_pages() -> usize {
    10
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
    }
}
