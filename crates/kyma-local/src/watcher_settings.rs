//! Persistent user-tunable settings for local-mode watchers (cc-sync, etc.).
//!
//! Settings are stored as JSON at `~/.kyma/watcher-settings.json`. Missing
//! file or parse error falls back to `Default` (everything enabled).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherSettings {
    /// Whether the Claude Code memory sync loop is active.
    pub cc_sync_enabled: bool,
}

impl Default for WatcherSettings {
    fn default() -> Self {
        Self {
            cc_sync_enabled: true,
        }
    }
}

impl WatcherSettings {
    fn path() -> std::path::PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join(".kyma")
            .join("watcher-settings.json")
    }

    pub async fn load() -> Self {
        let path = Self::path();
        match tokio::fs::read_to_string(&path).await {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, serde_json::to_string_pretty(self)?).await?;
        Ok(())
    }
}
