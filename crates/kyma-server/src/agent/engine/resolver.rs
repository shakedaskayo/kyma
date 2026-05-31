//! Resolve the secret material for an EngineConfig.
//!
//! Lookup order — first match wins:
//!   1. The engine's `credential_id` if set (explicit override).
//!   2. Provider-specific env var (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, …).
//!   3. `~/.claude/.credentials.json` (Anthropic only).
//!   4. Ollama: no key — host URL is the config.

use super::{claude_creds, EngineConfig, EngineKind};
use kyma_core::credentials::{CredentialStore, CredentialValue};
use kyma_core::tenant::TenantId;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ResolvedKey {
    ApiKey(String),
    None,
}

pub struct CredentialResolver {
    creds: Arc<dyn CredentialStore>,
    tenant: TenantId,
}

impl CredentialResolver {
    pub fn new(creds: Arc<dyn CredentialStore>, tenant: TenantId) -> Self {
        Self { creds, tenant }
    }

    pub async fn resolve(&self, cfg: &EngineConfig) -> anyhow::Result<ResolvedKey> {
        // 1. Explicit credential reference always wins.
        if let Some(id) = cfg.credential_id {
            let cred = self.creds.get(self.tenant, id).await?;
            let key = match cred.value {
                CredentialValue::ApiKey { value, .. } => value,
                CredentialValue::Pat { token } => token,
                other => anyhow::bail!(
                    "credential {} is kind {} — expected api_key or pat",
                    id,
                    other.kind()
                ),
            };
            return Ok(ResolvedKey::ApiKey(key));
        }

        // 2/3/4. Per-kind fallback chain.
        match cfg.kind {
            EngineKind::Anthropic => {
                if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
                    if !v.is_empty() {
                        return Ok(ResolvedKey::ApiKey(v));
                    }
                }
                if let Some(v) = claude_creds::discover_anthropic_key() {
                    return Ok(ResolvedKey::ApiKey(v));
                }
                anyhow::bail!(
                    "no Anthropic key found: set ANTHROPIC_API_KEY, log in to Claude Code, or attach a credential in Settings → Engine"
                )
            }
            EngineKind::Openai => {
                if let Ok(v) = std::env::var("OPENAI_API_KEY") {
                    if !v.is_empty() {
                        return Ok(ResolvedKey::ApiKey(v));
                    }
                }
                anyhow::bail!(
                    "no OpenAI key found: set OPENAI_API_KEY or attach a credential in Settings → Engine"
                )
            }
            EngineKind::Ollama => Ok(ResolvedKey::None),
            // The CLI handles its own auth — Keychain OAuth, env vars,
            // whatever. We never see a key for this kind.
            EngineKind::ClaudeCli => Ok(ResolvedKey::None),
        }
    }
}
