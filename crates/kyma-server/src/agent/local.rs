//! Null store implementations for **local single-binary mode** (`kyma-local`).
//!
//! The agent surface persists engine preference, enabled skills, and tenant
//! credentials in Postgres. Local mode has no Postgres, so these no-op stores
//! satisfy the `AgentState` trait-object fields: no engine is configured (Ask
//! AI surfaces a clean "configure an engine" error), no skills are injected,
//! and there are no stored credentials. Memory + data + graph all run over the
//! catalog + engine and are unaffected.

use async_trait::async_trait;
use uuid::Uuid;

use crate::agent::engine::{EngineConfig, EnginePreferenceStore};
use crate::agent::skills::EnabledSkillsStore;
use kyma_core::credentials::{Credential, CredentialStore};
use kyma_core::tenant::TenantId;

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

/// Credential store with no stored credentials.
pub struct NullCredentialStore;

#[async_trait]
impl CredentialStore for NullCredentialStore {
    async fn get(&self, _tenant: TenantId, _id: Uuid) -> anyhow::Result<Credential> {
        anyhow::bail!("no credential store in local mode")
    }
}
