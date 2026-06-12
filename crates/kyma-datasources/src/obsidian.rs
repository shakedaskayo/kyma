//! Obsidian vault data source — catalog/validation only. Continuous-drive
//! (`drive_model = "continuous"`): the local engine's vault watcher performs
//! the actual sync (`kyma-local/src/vault_sync.rs`); the periodic scheduler
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
        let mut name = CatalogField::text("vault_name", "Vault name", "my-vault");
        name.required = false;
        name.help = Some("Realm the notes land in — defaults to the vault folder name.".into());
        CatalogEntry {
            type_id: "obsidian".into(),
            label: "Obsidian".into(),
            category: "knowledge".into(),
            description: "Notes from a local Obsidian vault, synced continuously by a file watcher — wikilinks become graph edges, deleted notes archive.".into(),
            brand: "obsidian".into(),
            auth_mode: "none".into(),
            status: "available".into(),
            drive_model: "continuous".into(),
            // Full-scan fallback interval; fs events trigger syncs in between.
            default_schedule_ms: 60_000,
            fields: vec![
                CatalogField::text("vault_path", "Vault path", "~/Documents/MyVault"),
                name,
            ],
            resource: None,
            default_target_table: None,
            config_defaults: None,
            graph_name: None,
            accepted_credential_kinds: Vec::new(),
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let path = cfg
            .get("vault_path")
            .and_then(Value::as_str)
            .map_or("", str::trim);
        if path.is_empty() {
            return Err(ConfigError("vault_path is required".into()));
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
        assert!(c.fields.iter().any(|f| f.key == "vault_path" && f.required));
    }

    #[test]
    fn validate_requires_vault_path() {
        assert!(ObsidianDataSource.validate_config(&json!({})).is_err());
        assert!(ObsidianDataSource
            .validate_config(&json!({ "vault_path": "  " }))
            .is_err());
        assert!(ObsidianDataSource
            .validate_config(&json!({ "vault_path": "~/Vault" }))
            .is_ok());
    }
}
