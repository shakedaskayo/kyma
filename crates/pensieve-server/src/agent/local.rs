//! Store implementations for **local single-binary mode** (`pensieve-local`).
//!
//! The agent surface persists engine preference, enabled skills, and tenant
//! credentials in Postgres. Local mode has no Postgres: engine preference and
//! enabled skills persist to JSON files under `~/.pensieve` (so Settings → Agent
//! engine genuinely works and survives restarts), while credentials stay a
//! control-plane feature (engine auth auto-detects env vars /
//! `~/.claude/.credentials.json` instead). Memory + data + graph all run over
//! the catalog + engine and are unaffected.

use async_trait::async_trait;
use std::path::PathBuf;
use uuid::Uuid;

use crate::agent::engine::{EngineConfig, EnginePreferenceStore};
use crate::agent::skills::EnabledSkillsStore;
use pensieve_core::credentials::{Credential, CredentialStore};
use pensieve_core::tenant::TenantId;

/// Engine preference store that reports "no engine configured".
pub struct NullEnginePreferenceStore;

#[async_trait]
impl EnginePreferenceStore for NullEnginePreferenceStore {
    async fn get(&self) -> anyhow::Result<EngineConfig> {
        anyhow::bail!("no agent engine configured (local mode — set one to enable Ask AI)")
    }
    async fn put(&self, _cfg: &EngineConfig) -> anyhow::Result<()> {
        // Local mode doesn't persist engine preference. Accept silently so the
        // settings UI doesn't error.
        Ok(())
    }
}

/// Engine preference persisted to a JSON file — local mode's stand-in for the
/// Postgres preference row. Lets Settings → Agent engine actually save.
pub struct FileEnginePreferenceStore {
    path: PathBuf,
}

impl FileEnginePreferenceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl EnginePreferenceStore for FileEnginePreferenceStore {
    async fn get(&self) -> anyhow::Result<EngineConfig> {
        let raw = tokio::fs::read_to_string(&self.path).await.map_err(|_| {
            anyhow::anyhow!(
                "no agent engine configured — set one in Settings → Agent engine to enable Ask AI"
            )
        })?;
        Ok(serde_json::from_str(&raw)?)
    }
    async fn put(&self, cfg: &EngineConfig) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&self.path, serde_json::to_string_pretty(cfg)?).await?;
        Ok(())
    }
}

/// Enabled-skills store that reports no enabled skills.
pub struct NullEnabledSkillsStore;

#[async_trait]
impl EnabledSkillsStore for NullEnabledSkillsStore {
    async fn get(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn put(&self, _skills: &[String]) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Enabled skills persisted to a JSON file (same role as
/// [`FileEnginePreferenceStore`]) so skill toggles survive restarts locally.
pub struct FileEnabledSkillsStore {
    path: PathBuf,
}

impl FileEnabledSkillsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl EnabledSkillsStore for FileEnabledSkillsStore {
    async fn get(&self) -> anyhow::Result<Vec<String>> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(_) => Ok(Vec::new()),
        }
    }
    async fn put(&self, skills: &[String]) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&self.path, serde_json::to_string_pretty(&skills)?).await?;
        Ok(())
    }
}

/// Credential store with no stored credentials.
pub struct NullCredentialStore;

#[async_trait]
impl CredentialStore for NullCredentialStore {
    async fn get(&self, _tenant: TenantId, _id: Uuid) -> anyhow::Result<Credential> {
        anyhow::bail!("no credential store in local mode")
    }
}
