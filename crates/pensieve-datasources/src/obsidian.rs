//! Obsidian vault data source — catalog/validation only. Continuous-drive
//! (`drive_model = "continuous"`): the local engine's vault watcher performs
//! the actual sync (`pensieve-local/src/vault_sync.rs`); the periodic scheduler
//! never ticks these rows, and `run_once` refuses defensively.

use async_trait::async_trait;
use serde_json::Value;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun};

pub struct ObsidianDataSource;

#[async_trait]
impl DataSource for ObsidianDataSource {
    fn type_id(&self) -> &'static str {
        "obsidian"
    }

    fn catalog(&self) -> CatalogEntry {
        let mut path = CatalogField::text("vault_path", "Vault path", "~/Documents/MyVault");
        path.required = false;
        path.help = Some("Local folder to sync. Leave blank to import a git-hosted vault instead.".into());
        let mut git_url = CatalogField::text(
            "git_url",
            "Git URL (optional)",
            "https://github.com/org/team-vault.git",
        );
        git_url.required = false;
        git_url.help =
            Some("Import an existing Obsidian vault from a git repo — pensieve clones/pulls it and ingests the notes.".into());
        let mut git_token = CatalogField::secret(
            "git_token",
            "Git token (private repos)",
            "ghp_… / glpat-…",
            "Personal access token for a private git-hosted vault. Public repos need none.",
        );
        git_token.required = false;
        let mut git_branch = CatalogField::text("git_branch", "Branch (optional)", "main");
        git_branch.required = false;
        let mut name = CatalogField::text("vault_name", "Vault name", "my-vault");
        name.required = false;
        name.help = Some("Realm the notes land in — defaults to the vault/repo name.".into());
        CatalogEntry {
            type_id: "obsidian".into(),
            label: "Obsidian".into(),
            category: "knowledge".into(),
            description: "Notes from an Obsidian vault — a local folder or a git-hosted repo — synced continuously; wikilinks become graph edges, deleted notes archive.".into(),
            brand: "obsidian".into(),
            auth_mode: "none".into(),
            status: "available".into(),
            drive_model: "continuous".into(),
            // Full-scan fallback interval; fs events trigger syncs in between.
            default_schedule_ms: 60_000,
            fields: vec![path, git_url, git_token, git_branch, name],
            resource: None,
            default_target_table: None,
            config_defaults: None,
            graph_name: None,
            accepted_credential_kinds: Vec::new(),
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let field = |k: &str| cfg.get(k).and_then(Value::as_str).map_or("", str::trim);
        let path = field("vault_path");
        let git_url = field("git_url");
        if path.is_empty() && git_url.is_empty() {
            return Err(ConfigError("provide a vault_path or a git_url".into()));
        }
        if !git_url.is_empty()
            && !(git_url.starts_with("https://")
                || git_url.starts_with("http://")
                || git_url.starts_with("git@")
                || git_url.starts_with("ssh://")
                || git_url.starts_with("file://"))
        {
            return Err(ConfigError("git_url must be an http(s), ssh, or file git URL".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        _ctx: &DataSourceCtx,
        _cfg: &Value,
        _cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        Err(DataSourceError::Permanent(
            "obsidian sources are synced by the local vault watcher, not the scheduler".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_is_continuous_and_available() {
        let c = ObsidianDataSource.catalog();
        assert_eq!(c.type_id, "obsidian");
        assert_eq!(c.drive_model, "continuous");
        assert_eq!(c.status, "available");
        assert_eq!(c.brand, "obsidian");
        // Both a local path and a git URL are offered; neither is hard-required
        // on its own (validate enforces the either/or).
        assert!(c.fields.iter().any(|f| f.key == "vault_path"));
        assert!(c.fields.iter().any(|f| f.key == "git_url"));
    }

    #[test]
    fn validate_requires_path_or_git_url() {
        assert!(ObsidianDataSource.validate_config(&json!({})).is_err());
        assert!(ObsidianDataSource
            .validate_config(&json!({ "vault_path": "  " }))
            .is_err());
        assert!(ObsidianDataSource
            .validate_config(&json!({ "vault_path": "~/Vault" }))
            .is_ok());
        // Git-hosted import: URL alone is enough.
        assert!(ObsidianDataSource
            .validate_config(&json!({ "git_url": "https://github.com/org/vault.git" }))
            .is_ok());
        // A bad URL scheme is rejected.
        assert!(ObsidianDataSource
            .validate_config(&json!({ "git_url": "s3://not-git" }))
            .is_err());
    }
}
