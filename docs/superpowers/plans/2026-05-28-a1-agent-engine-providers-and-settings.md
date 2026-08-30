# A1 — Agent Engine Providers + Credential Discovery + Settings UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded Ollama-only agent backend with a swappable engine registry (Anthropic / OpenAI / Ollama), a credential resolver that picks up `ANTHROPIC_API_KEY`/`~/.claude/.credentials.json`/catalog credentials, and a Settings → Agent Engine UI to configure all of it.

**Architecture:** Introduce an `EngineRegistry` trait inside `crates/pensieve-server/src/agent/engine/` with one impl per provider. `runner.rs` calls `EngineRegistry::resolve(...)` per run to construct the right adk-rust `Llm`. A new `engine_config` table (single global row for v1) persists the active provider/model/credential_id. A `CredentialResolver` walks env vars → `~/.claude/.credentials.json` → catalog `CredentialStore`. Web Settings page lists available providers (via `GET /v1/agent/engines`), shows the active config, lets the user save/test a new one.

**Tech Stack:** Rust (axum, sqlx, adk-rust 0.6 with `anthropic` + `openai` features), TypeScript/React (react-query, shadcn/ui), Postgres migration.

---

## File Structure

**Rust — `crates/pensieve-server/src/agent/`:**

- Create `engine/mod.rs` — `EngineKind`, `EngineConfig`, `ResolvedEngine`, `EngineRegistry` trait, `build_engine` dispatch.
- Create `engine/anthropic.rs` — `AnthropicEngine` impl, default model list.
- Create `engine/openai.rs` — `OpenAIEngine` impl, default model list.
- Create `engine/ollama.rs` — `OllamaEngine` impl (moves the current logic out of `runner.rs`).
- Create `engine/resolver.rs` — `CredentialResolver` (env → claude creds file → catalog).
- Create `engine/claude_creds.rs` — read `~/.claude/.credentials.json`, return the active subscription's API key.
- Create `engine/store.rs` — `EnginePreferenceStore` trait + Postgres impl (one row in new `engine_config` table).
- Modify `runner.rs:67-103` — `build_agent` accepts a resolved engine instead of constructing Ollama inline.
- Modify `mod.rs:1-27` — re-export new types.
- Modify `state.rs:13-23` — add `Arc<dyn EnginePreferenceStore>` + `Arc<dyn CredentialStore>` (already present?) to `AgentState`.

**Rust — `crates/pensieve-server/src/agent/routes.rs`:**

- Add `GET /v1/agent/engines` — list available engine kinds + their default models + whether creds are detected.
- Add `GET /v1/agent/engine` — current `EngineConfig`.
- Add `PUT /v1/agent/engine` — save a new `EngineConfig`.
- Add `POST /v1/agent/engine/test` — dry-run a one-shot completion to verify creds + connectivity.

**Rust — migration:**

- Create `crates/pensieve-catalog/migrations/012_agent_engine_config.sql` — `engine_config` table (singleton row for v1).

**Rust — bin wiring:**

- Modify `crates/pensieve-bin/src/main.rs` — wire the new engine routes into the agent sub-router; construct `AgentState` with the new stores.

**Web — `web/src/sdk/`:**

- Create `web/src/sdk/agent-engine.ts` — typed client for the four new endpoints.

**Web — `web/src/features/settings/`:**

- Create `web/src/features/settings/EngineSettings.tsx` — the Settings → Agent Engine card.
- Modify `web/src/routes/_app.settings.tsx` — add the EngineSettings card to the page.

---

## Pre-flight

- [ ] **Step 0a: Confirm working directory**

Run: `pwd`
Expected: `/Users/shakedaskayo/shaked/projects/pensieve/.claude/worktrees/feature+graph-layer`

- [ ] **Step 0b: Confirm clean working tree before starting**

Run: `git status --short`
Expected: a list of in-flight changes from earlier sessions is acceptable, but no `M`-staged surprises in `crates/pensieve-server/src/agent/` or `crates/pensieve-catalog/`. If you see unrelated modifications there, stash them before starting.

- [ ] **Step 0c: Verify adk-rust features needed are *not* enabled yet**

Run: `grep -A10 "adk-rust = " Cargo.toml`
Expected: workspace dep has `ollama` but NOT `anthropic` or `openai`. (We'll add them in Task 2.)

- [ ] **Step 0d: Verify catalog `CredentialStore` exists**

Run: `grep -n "pub trait CredentialStore" crates/pensieve-core/src/credentials.rs`
Expected: line 152 (or thereabouts) — the trait we'll wire into the resolver.

---

## Task 1: Add the `engine_config` migration

**Files:**
- Create: `crates/pensieve-catalog/migrations/012_agent_engine_config.sql`

- [ ] **Step 1: Write the migration**

```sql
-- 012_agent_engine_config.sql
-- Singleton row for the active agent engine. v1 is single-tenant globally; a
-- future per-user / per-tenant variant adds (tenant_id, user_id) columns and
-- relaxes the singleton constraint.
CREATE TABLE engine_config (
    id              SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    kind            TEXT NOT NULL,
    model           TEXT NOT NULL,
    credential_id   UUID REFERENCES credentials(id) ON DELETE SET NULL,
    host            TEXT,
    extras          JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the existing implicit default (Ollama at localhost) so a fresh deploy
-- with no API keys still gets the legacy behaviour out of the box.
INSERT INTO engine_config (id, kind, model, host)
VALUES (1, 'ollama', 'gemma4:latest', 'http://localhost:11434')
ON CONFLICT (id) DO NOTHING;
```

- [ ] **Step 2: Apply the migration locally**

Run: `sqlx migrate run --source crates/pensieve-catalog/migrations`
Expected: `Applied 012/migrate agent engine config (...)`

- [ ] **Step 3: Verify the row exists**

Run: `psql "$DATABASE_URL" -c "SELECT id, kind, model FROM engine_config"`
Expected: one row, `id=1 kind=ollama model=gemma4:latest`

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-catalog/migrations/012_agent_engine_config.sql
git commit -m "feat(agent): migration 012 — engine_config singleton table"
```

---

## Task 2: Enable adk-rust anthropic + openai features

**Files:**
- Modify: `Cargo.toml` workspace dep

- [ ] **Step 1: Edit the workspace dep**

In `Cargo.toml` find the `adk-rust = {...}` line and replace its `features` array with:

```toml
adk-rust = { version = "0.6", default-features = false, features = [
    "agents",
    "models",
    "ollama",
    "anthropic",
    "openai",
    "tools",
    "sessions",
    "runner",
] }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p pensieve-server`
Expected: clean build (may take ~60s on first run due to new deps).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(agent): enable adk-rust anthropic + openai features"
```

---

## Task 3: Engine kinds + config type

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/mod.rs`
- Test: `crates/pensieve-server/src/agent/engine/mod.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Create `crates/pensieve-server/src/agent/engine/mod.rs` with:

```rust
//! Agent engine registry — picks an LLM provider based on persisted config.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Anthropic,
    Openai,
    Ollama,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub kind: EngineKind,
    pub model: String,
    pub credential_id: Option<Uuid>,
    pub host: Option<String>,
    #[serde(default)]
    pub extras: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_kind_roundtrips_via_json() {
        for kind in [EngineKind::Anthropic, EngineKind::Openai, EngineKind::Ollama] {
            let s = serde_json::to_string(&kind).unwrap();
            let back: EngineKind = serde_json::from_str(&s).unwrap();
            assert_eq!(kind, back, "roundtrip for {kind:?}");
        }
    }

    #[test]
    fn engine_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EngineKind::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(serde_json::to_string(&EngineKind::Openai).unwrap(), "\"openai\"");
    }
}
```

- [ ] **Step 2: Wire the new module into the agent crate**

Add to `crates/pensieve-server/src/agent/mod.rs` after the existing `pub mod` lines:

```rust
pub mod engine;
```

- [ ] **Step 3: Run the test to confirm it passes**

Run: `cargo test -p pensieve-server --lib agent::engine::tests`
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/mod.rs crates/pensieve-server/src/agent/mod.rs
git commit -m "feat(agent): EngineKind + EngineConfig types"
```

---

## Task 4: `EnginePreferenceStore` trait + Postgres impl

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/store.rs`
- Modify: `crates/pensieve-server/src/agent/engine/mod.rs` (add `pub mod store;`)
- Test: inline `#[cfg(test)]` in `store.rs`

- [ ] **Step 1: Write the trait + impl**

Create `crates/pensieve-server/src/agent/engine/store.rs`:

```rust
//! Persistence for the active EngineConfig — singleton row in `engine_config`.

use super::{EngineConfig, EngineKind};
use async_trait::async_trait;
use sqlx::PgPool;
use std::str::FromStr;
use uuid::Uuid;

#[async_trait]
pub trait EnginePreferenceStore: Send + Sync {
    async fn get(&self) -> anyhow::Result<EngineConfig>;
    async fn put(&self, cfg: &EngineConfig) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct PgEnginePreferenceStore {
    pool: PgPool,
}

impl PgEnginePreferenceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EnginePreferenceStore for PgEnginePreferenceStore {
    async fn get(&self) -> anyhow::Result<EngineConfig> {
        let row: (String, String, Option<Uuid>, Option<String>, serde_json::Value) = sqlx::query_as(
            "SELECT kind, model, credential_id, host, extras FROM engine_config WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        let kind = parse_kind(&row.0)?;
        Ok(EngineConfig {
            kind,
            model: row.1,
            credential_id: row.2,
            host: row.3,
            extras: row.4,
        })
    }

    async fn put(&self, cfg: &EngineConfig) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO engine_config (id, kind, model, credential_id, host, extras, updated_at)
             VALUES (1, $1, $2, $3, $4, $5, NOW())
             ON CONFLICT (id) DO UPDATE SET
                kind = EXCLUDED.kind,
                model = EXCLUDED.model,
                credential_id = EXCLUDED.credential_id,
                host = EXCLUDED.host,
                extras = EXCLUDED.extras,
                updated_at = NOW()",
        )
        .bind(cfg.kind.as_str())
        .bind(&cfg.model)
        .bind(cfg.credential_id)
        .bind(&cfg.host)
        .bind(&cfg.extras)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn parse_kind(s: &str) -> anyhow::Result<EngineKind> {
    Ok(match s {
        "anthropic" => EngineKind::Anthropic,
        "openai" => EngineKind::Openai,
        "ollama" => EngineKind::Ollama,
        other => anyhow::bail!("unknown engine kind: {other}"),
    })
}

impl FromStr for EngineKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_kind(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    async fn pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok()?;
        PgPoolOptions::new().connect(&url).await.ok()
    }

    #[tokio::test]
    async fn roundtrip_through_postgres() {
        let Some(pool) = pool().await else {
            eprintln!("skip: DATABASE_URL not set");
            return;
        };
        let store = PgEnginePreferenceStore::new(pool);
        let cfg = EngineConfig {
            kind: EngineKind::Anthropic,
            model: "claude-sonnet-4-6".into(),
            credential_id: None,
            host: None,
            extras: serde_json::json!({"max_tokens": 4096}),
        };
        store.put(&cfg).await.unwrap();
        let back = store.get().await.unwrap();
        assert_eq!(back.kind, EngineKind::Anthropic);
        assert_eq!(back.model, "claude-sonnet-4-6");
        assert_eq!(back.extras["max_tokens"], 4096);
        // Restore the default so other tests don't see weird state.
        store
            .put(&EngineConfig {
                kind: EngineKind::Ollama,
                model: "gemma4:latest".into(),
                credential_id: None,
                host: Some("http://localhost:11434".into()),
                extras: serde_json::json!({}),
            })
            .await
            .unwrap();
    }
}
```

- [ ] **Step 2: Wire the new submodule**

Append to `crates/pensieve-server/src/agent/engine/mod.rs`:

```rust
pub mod store;
pub use store::{EnginePreferenceStore, PgEnginePreferenceStore};
```

- [ ] **Step 3: Run the test**

Run: `DATABASE_URL=postgres://pensieve:pensieve@localhost:5432/pensieve cargo test -p pensieve-server --lib agent::engine::store::tests -- --nocapture`
Expected: 1 test passes. If `DATABASE_URL` isn't set the test prints `skip:` and exits 0 — that's fine for laptops without Postgres.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/store.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): EnginePreferenceStore trait + Postgres impl"
```

---

## Task 5: Claude Code credentials reader

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/claude_creds.rs`
- Modify: `crates/pensieve-server/src/agent/engine/mod.rs` (`pub mod claude_creds;`)
- Test: inline

- [ ] **Step 1: Write the reader**

Create `crates/pensieve-server/src/agent/engine/claude_creds.rs`:

```rust
//! Read the Claude Code creds file at `~/.claude/.credentials.json` and
//! return an Anthropic API key if one is present.
//!
//! The file shape can vary between Claude Code versions, so we use a
//! permissive serde derive and a small set of well-known keys. If the file
//! doesn't exist, or doesn't contain a recognisable key, return `None` —
//! callers fall back to env vars and the catalog credential store.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
struct ClaudeCreds {
    #[serde(default, rename = "apiKey")]
    api_key: Option<String>,
    #[serde(default, rename = "ANTHROPIC_API_KEY")]
    anthropic_api_key: Option<String>,
    #[serde(default)]
    subscriptions: Vec<ClaudeSubscription>,
}

#[derive(Debug, Deserialize)]
struct ClaudeSubscription {
    #[serde(default)]
    active: bool,
    #[serde(default, rename = "apiKey")]
    api_key: Option<String>,
}

fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".claude");
    p.push(".credentials.json");
    Some(p)
}

/// Try to discover an Anthropic API key from the Claude Code config dir.
/// Returns `None` quietly if the file doesn't exist or doesn't yield a key —
/// not finding a key is an expected case, not an error.
pub fn discover_anthropic_key() -> Option<String> {
    let path = default_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: ClaudeCreds = serde_json::from_str(&raw).ok()?;
    // Prefer an explicit top-level key…
    if let Some(k) = parsed.api_key.or(parsed.anthropic_api_key) {
        if !k.is_empty() {
            return Some(k);
        }
    }
    // …then the active subscription's key.
    parsed
        .subscriptions
        .into_iter()
        .find(|s| s.active)
        .and_then(|s| s.api_key)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_none() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/nonexistent/pensieve-test-home");
        assert_eq!(discover_anthropic_key(), None);
        if let Some(v) = prev {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn parses_top_level_api_key() {
        let raw = r#"{"apiKey":"sk-ant-test"}"#;
        let parsed: ClaudeCreds = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.api_key.as_deref(), Some("sk-ant-test"));
    }

    #[test]
    fn parses_active_subscription_key() {
        let raw = r#"{"subscriptions":[{"active":false,"apiKey":"old"},{"active":true,"apiKey":"new"}]}"#;
        let parsed: ClaudeCreds = serde_json::from_str(raw).unwrap();
        let active = parsed.subscriptions.into_iter().find(|s| s.active).unwrap();
        assert_eq!(active.api_key.as_deref(), Some("new"));
    }
}
```

- [ ] **Step 2: Wire the submodule**

Append to `crates/pensieve-server/src/agent/engine/mod.rs`:

```rust
pub mod claude_creds;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p pensieve-server --lib agent::engine::claude_creds::tests`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/claude_creds.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): ClaudeCode credential discovery (~/.claude/.credentials.json)"
```

---

## Task 6: `CredentialResolver` — env → claude → catalog

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/resolver.rs`
- Modify: `crates/pensieve-server/src/agent/engine/mod.rs` (`pub mod resolver;`)

- [ ] **Step 1: Write the resolver**

Create `crates/pensieve-server/src/agent/engine/resolver.rs`:

```rust
//! Resolve the secret material for an EngineConfig.
//!
//! Lookup order — first match wins:
//!   1. The engine's `credential_id` if set (explicit override).
//!   2. Provider-specific env var (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, …).
//!   3. `~/.claude/.credentials.json` (Anthropic only).
//!   4. Ollama: no key — host URL is the config.

use super::{claude_creds, EngineConfig, EngineKind};
use pensieve_core::credentials::{CredentialStore, CredentialValue};
use pensieve_core::tenant::TenantId;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ResolvedKey {
    /// API key string (Anthropic, OpenAI, …).
    ApiKey(String),
    /// No key needed (Ollama, others).
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
            let cred = self.creds.get(self.tenant.clone(), id).await?;
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
        }
    }
}
```

- [ ] **Step 2: Wire the submodule**

Append to `mod.rs`:

```rust
pub mod resolver;
pub use resolver::{CredentialResolver, ResolvedKey};
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/resolver.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): CredentialResolver — env / claude-code / catalog"
```

---

## Task 7: `EngineRegistry` + Ollama impl (refactor existing logic out of runner.rs)

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/ollama.rs`

- [ ] **Step 1: Write the registry trait + Ollama impl**

Create `crates/pensieve-server/src/agent/engine/ollama.rs`:

```rust
//! Ollama engine — existing default. No API key needed; host is the config.

use adk_rust::model::ollama::{OllamaConfig, OllamaModel};
use adk_rust::Llm;
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "gemma4:latest";
pub const DEFAULT_HOST: &str = "http://localhost:11434";

pub fn build(cfg: &EngineConfig, _key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let host = cfg
        .host
        .clone()
        .unwrap_or_else(|| DEFAULT_HOST.to_string());
    let llm_cfg = OllamaConfig {
        host,
        model: cfg.model.clone(),
        temperature: Some(0.0),
        num_ctx: None,
        top_p: None,
        top_k: None,
    };
    let llm = OllamaModel::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("ollama init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<&'static str> {
    &["gemma4:latest", "llama4:latest", "qwen3:latest", "mistral:latest"]
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .collect()
}
```

- [ ] **Step 2: Wire the submodule**

Append to `mod.rs`:

```rust
pub mod ollama;
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/ollama.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): Ollama engine extracted behind the new registry shape"
```

---

## Task 8: Anthropic engine impl

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/anthropic.rs`

- [ ] **Step 1: Write the impl**

Create `crates/pensieve-server/src/agent/engine/anthropic.rs`:

```rust
//! Anthropic engine — adk-rust's AnthropicClient.

use adk_rust::Llm;
use adk_rust::model::anthropic::{AnthropicClient, AnthropicConfig};
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

pub fn build(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let api_key = match key {
        ResolvedKey::ApiKey(k) => k,
        ResolvedKey::None => anyhow::bail!("Anthropic engine requires an API key"),
    };
    let mut llm_cfg = AnthropicConfig::new(api_key, cfg.model.clone());
    if let Some(host) = &cfg.host {
        llm_cfg = llm_cfg.with_base_url(host);
    }
    if let Some(mt) = cfg.extras.get("max_tokens").and_then(|v| v.as_u64()) {
        llm_cfg = llm_cfg.with_max_tokens(mt as u32);
    }
    let llm = AnthropicClient::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("anthropic init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<&'static str> {
    vec![
        "claude-opus-4-7",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
    ]
}
```

- [ ] **Step 2: Wire the submodule**

Append to `mod.rs`:

```rust
pub mod anthropic;
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean. (If the adk-rust API differs slightly, adjust the import — the upstream Anthropic surface is `AnthropicClient::new(AnthropicConfig)`.)

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/anthropic.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): Anthropic engine via adk-rust AnthropicClient"
```

---

## Task 9: OpenAI engine impl

**Files:**
- Create: `crates/pensieve-server/src/agent/engine/openai.rs`

- [ ] **Step 1: Write the impl**

```rust
//! OpenAI engine — adk-rust's OpenAIResponsesClient.

use adk_rust::Llm;
use adk_rust::model::openai::{OpenAIResponsesClient, OpenAIResponsesConfig};
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "gpt-5";

pub fn build(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let api_key = match key {
        ResolvedKey::ApiKey(k) => k,
        ResolvedKey::None => anyhow::bail!("OpenAI engine requires an API key"),
    };
    let llm_cfg = OpenAIResponsesConfig {
        api_key,
        model: cfg.model.clone(),
        base_url: cfg.host.clone(),
        ..Default::default()
    };
    let llm = OpenAIResponsesClient::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("openai init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<&'static str> {
    vec!["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini"]
}
```

- [ ] **Step 2: Wire the submodule**

Append to `mod.rs`:

```rust
pub mod openai;
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean. If `OpenAIResponsesConfig` struct fields differ in adk-rust 0.6, the field set has `api_key: String`, `model: String`, `base_url: Option<String>` plus other defaulted knobs — keep the field assignment minimal and let `..Default::default()` fill the rest.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/openai.rs crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): OpenAI engine via adk-rust OpenAIResponsesClient"
```

---

## Task 10: Engine dispatch — `build_engine`

**Files:**
- Modify: `crates/pensieve-server/src/agent/engine/mod.rs`

- [ ] **Step 1: Append the dispatch function**

In `mod.rs`, below the existing types and module declarations:

```rust
use adk_rust::Llm;
use std::sync::Arc;

/// Construct an `Llm` for the given engine config + resolved credential.
pub fn build_engine(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    match cfg.kind {
        EngineKind::Anthropic => anthropic::build(cfg, key),
        EngineKind::Openai => openai::build(cfg, key),
        EngineKind::Ollama => ollama::build(cfg, key),
    }
}

/// Available providers and their default model menus. Returned by
/// `GET /v1/agent/engines` so the UI can render the picker.
#[derive(Debug, serde::Serialize)]
pub struct EngineSummary {
    pub kind: EngineKind,
    pub label: &'static str,
    pub models: Vec<&'static str>,
    pub needs_key: bool,
}

pub fn engine_catalogue() -> Vec<EngineSummary> {
    vec![
        EngineSummary {
            kind: EngineKind::Anthropic,
            label: "Anthropic (Claude)",
            models: anthropic::default_models(),
            needs_key: true,
        },
        EngineSummary {
            kind: EngineKind::Openai,
            label: "OpenAI",
            models: openai::default_models(),
            needs_key: true,
        },
        EngineSummary {
            kind: EngineKind::Ollama,
            label: "Ollama (local)",
            models: ollama::default_models(),
            needs_key: false,
        },
    ]
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/pensieve-server/src/agent/engine/mod.rs
git commit -m "feat(agent): build_engine dispatch + engine_catalogue summary"
```

---

## Task 11: Wire the new engine into the runner

**Files:**
- Modify: `crates/pensieve-server/src/agent/state.rs`
- Modify: `crates/pensieve-server/src/agent/runner.rs`

- [ ] **Step 1: Extend `AgentState`**

Replace `crates/pensieve-server/src/agent/state.rs` with:

```rust
//! Shared handler state for the `/v1/agent/*` surface.

use crate::agent::engine::EnginePreferenceStore;
use pensieve_core::catalog::Catalog;
use pensieve_core::credentials::CredentialStore;
use pensieve_core::segment_format::SegmentFormat;
use pensieve_core::tenant::TenantId;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentState {
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
    pub pool: PgPool,
    pub engines: Arc<dyn EnginePreferenceStore>,
    pub credentials: Arc<dyn CredentialStore>,
    pub tenant: TenantId,
}
```

- [ ] **Step 2: Rewrite `build_agent` to consult the engine store**

In `crates/pensieve-server/src/agent/runner.rs`, replace `build_agent` (around line 67-103) with:

```rust
use crate::agent::engine::{build_engine, CredentialResolver};

pub async fn build_agent(state: &AgentState) -> anyhow::Result<Arc<dyn Agent>> {
    let cfg = state.engines.get().await?;
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant.clone());
    let key = resolver.resolve(&cfg).await?;
    let llm = build_engine(&cfg, key)?;

    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
    };

    let agent = LlmAgentBuilder::new("pensieve-assistant")
        .description(
            "Pensieve inline data assistant — answers English questions about the user's data.",
        )
        .instruction(SYSTEM_PROMPT)
        .model(llm)
        .tool(tool_list_databases(shared.clone()))
        .tool(tool_explore_schema(shared.clone()))
        .tool(tool_describe_table(shared.clone()))
        .tool(tool_run_kql(shared.clone()))
        .tool(tool_run_sql(shared.clone()))
        .tool(tool_sample_rows(shared.clone()))
        .tool(tool_find_references_to(shared.clone()))
        .tool(tool_graph_traverse(shared))
        .build()
        .map_err(|e| anyhow::anyhow!("agent build failed: {e:?}"))?;

    Ok(Arc::new(agent))
}
```

- [ ] **Step 3: Update `make_runner` to await the new async `build_agent`**

In `runner.rs`, change the call inside `make_runner` from `let agent = build_agent(state)?;` to `let agent = build_agent(state).await?;`.

- [ ] **Step 4: Update `model_id` to read from the store**

Replace the existing `model_id` function in `runner.rs`:

```rust
/// Effective model id string persisted into `agent_runs.model_id`.
pub async fn model_id(state: &AgentState) -> String {
    match state.engines.get().await {
        Ok(cfg) => format!("{}/{}", cfg.kind.as_str(), cfg.model),
        Err(_) => format!("ollama/{}", DEFAULT_MODEL),
    }
}
```

- [ ] **Step 5: Update call sites of `model_id`**

Run: `grep -rn "model_id()" crates/pensieve-server/src/agent/`
Each caller of `model_id()` needs to become `model_id(&state).await`. Update each one in turn (likely only `routes.rs`).

- [ ] **Step 6: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean (with one warning expected about the now-unused `DEFAULT_OLLAMA_HOST`/`DEFAULT_MODEL` constants — leave them; they're documented defaults).

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-server/src/agent/runner.rs crates/pensieve-server/src/agent/state.rs
git commit -m "feat(agent): runner consults EnginePreferenceStore + CredentialResolver"
```

---

## Task 12: HTTP routes — `GET /v1/agent/engines` and `GET/PUT /v1/agent/engine`

**Files:**
- Modify: `crates/pensieve-server/src/agent/routes.rs`

- [ ] **Step 1: Add the new handlers**

In `routes.rs`, near the existing route definitions, add:

```rust
use crate::agent::engine::{engine_catalogue, EngineConfig};

async fn list_engines(
    State(state): State<AgentState>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let catalogue = engine_catalogue();
    let active = state
        .engines
        .get()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(serde_json::json!({
        "available": catalogue,
        "active": active,
    })))
}

async fn get_engine(
    State(state): State<AgentState>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .get()
        .await
        .map(axum::Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn put_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .put(&cfg)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(cfg))
}
```

- [ ] **Step 2: Add the routes to the router**

Find the existing `pub fn router() -> Router<AgentState>` (or the function that returns the agent sub-router) and add:

```rust
.route("/engines", axum::routing::get(list_engines))
.route("/engine", axum::routing::get(get_engine).put(put_engine))
```

- [ ] **Step 3: Verify the new routes compile and respond**

Run: `cargo check -p pensieve-server`
Then, with `pensieve-bin` running and `DATABASE_URL` set:

```
curl -sS http://localhost:8080/v1/agent/engines -H "Authorization: Bearer $TOKEN" | jq '.available[].kind'
```

Expected: `"anthropic"`, `"openai"`, `"ollama"` in some order.

- [ ] **Step 4: Commit**

```bash
git add crates/pensieve-server/src/agent/routes.rs
git commit -m "feat(agent): GET /v1/agent/engines + GET/PUT /v1/agent/engine"
```

---

## Task 13: `POST /v1/agent/engine/test` — connectivity smoke test

**Files:**
- Modify: `crates/pensieve-server/src/agent/routes.rs`

- [ ] **Step 1: Add the test endpoint**

In `routes.rs`:

```rust
async fn test_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    use crate::agent::engine::{build_engine, CredentialResolver};
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant.clone());
    let key = resolver
        .resolve(&cfg)
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, format!("credential: {e}")))?;
    let llm = build_engine(&cfg, key)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, format!("init: {e}")))?;
    // Send a 1-token health probe — any successful streaming response
    // confirms creds + endpoint + model name.
    let probe = llm
        .generate_content(adk_rust::request::GenerateContentRequest {
            contents: vec![adk_rust::request::Content::user("ping")],
            ..Default::default()
        })
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, format!("probe: {e:?}")))?;
    Ok(axum::Json(serde_json::json!({
        "ok": true,
        "kind": cfg.kind,
        "model": cfg.model,
        "tokens": probe.usage.map(|u| u.total_tokens).unwrap_or(0),
    })))
}
```

Add the route inside `router()`:

```rust
.route("/engine/test", axum::routing::post(test_engine))
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p pensieve-server`
Expected: clean. If the `generate_content` signature in adk-rust 0.6 differs, use `llm.health_check()` if it exists, or `llm.list_models()` — whichever is the smallest no-op the trait offers. Worst case, `cargo doc --open -p adk-rust` and find the minimal probe method.

- [ ] **Step 3: Commit**

```bash
git add crates/pensieve-server/src/agent/routes.rs
git commit -m "feat(agent): POST /v1/agent/engine/test — connectivity probe"
```

---

## Task 14: Wire the new stores into `pensieve-bin`

**Files:**
- Modify: `crates/pensieve-bin/src/main.rs`

- [ ] **Step 1: Find where `AgentState` is constructed**

Run: `grep -n "AgentState" crates/pensieve-bin/src/main.rs`

- [ ] **Step 2: Add the new fields**

Replace the existing `AgentState { catalog, format, pool }` construction (likely line ~150–200) with:

```rust
use pensieve_server::agent::engine::PgEnginePreferenceStore;
use pensieve_catalog::PgCredentialStore;

let engines = std::sync::Arc::new(PgEnginePreferenceStore::new(pg_pool.clone()));
let creds_store: std::sync::Arc<dyn pensieve_core::credentials::CredentialStore> =
    std::sync::Arc::new(PgCredentialStore::new(pg_pool.clone(), encryption_key.clone()));
let agent_state = AgentState {
    catalog: catalog.clone(),
    format: format.clone(),
    pool: pg_pool.clone(),
    engines,
    credentials: creds_store,
    tenant: pensieve_core::tenant::DEFAULT_TENANT,
};
```

(The exact `PgCredentialStore::new` signature lives in `crates/pensieve-catalog/src/credentials.rs` — match it.)

- [ ] **Step 3: Verify the bin compiles + boots**

Run: `cargo run -p pensieve-bin -- --help`
Expected: usage prints without panic.

- [ ] **Step 4: Smoke-test the new endpoints**

```
cargo run -p pensieve-bin &
sleep 4
curl -sS http://localhost:8080/v1/agent/engines | jq
kill %1
```

Expected: JSON listing 3 engines + the active Ollama config.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-bin/src/main.rs
git commit -m "feat(agent): wire EnginePreferenceStore + CredentialStore into pensieve-bin"
```

---

## Task 15: Web SDK client

**Files:**
- Create: `web/src/sdk/agent-engine.ts`

- [ ] **Step 1: Write the client**

```ts
//! Typed client for `/v1/agent/engines` + `/v1/agent/engine`.

export type EngineKind = "anthropic" | "openai" | "ollama";

export interface EngineConfig {
  kind: EngineKind;
  model: string;
  credential_id?: string | null;
  host?: string | null;
  extras?: Record<string, unknown>;
}

export interface EngineSummary {
  kind: EngineKind;
  label: string;
  models: string[];
  needs_key: boolean;
}

export interface EngineList {
  available: EngineSummary[];
  active: EngineConfig;
}

type Args = { endpoint: string; token: string };

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}
function headers(token: string): Record<string, string> {
  return {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
}

export async function listEngines(a: Args): Promise<EngineList> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engines`, {
    headers: headers(a.token),
  });
  if (!res.ok) throw new Error(`engines: ${res.status}`);
  return res.json();
}

export async function putEngine(a: Args, cfg: EngineConfig): Promise<EngineConfig> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engine`, {
    method: "PUT",
    headers: headers(a.token),
    body: JSON.stringify(cfg),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`save engine: ${res.status} ${t}`);
  }
  return res.json();
}

export async function testEngine(
  a: Args,
  cfg: EngineConfig,
): Promise<{ ok: boolean; kind: string; model: string }> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engine/test`, {
    method: "POST",
    headers: headers(a.token),
    body: JSON.stringify(cfg),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`test engine: ${res.status} ${t}`);
  }
  return res.json();
}
```

- [ ] **Step 2: Verify typecheck**

Run: `cd web && pnpm typecheck`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add web/src/sdk/agent-engine.ts
git commit -m "feat(web): SDK client for /v1/agent/engines + /v1/agent/engine"
```

---

## Task 16: Settings UI — `EngineSettings` card

**Files:**
- Create: `web/src/features/settings/EngineSettings.tsx`
- Modify: `web/src/routes/_app.settings.tsx`

- [ ] **Step 1: Write the card**

```tsx
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Check, Loader2, X } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  listEngines,
  putEngine,
  testEngine,
  type EngineConfig,
  type EngineKind,
} from "@/sdk/agent-engine";
import { listCredentials } from "@/sdk/credentials";

/**
 * Settings → Agent Engine card.
 *
 * Reads /v1/agent/engines, lets the user pick provider + model + credential,
 * tests connectivity, and saves. The card surfaces auto-detected credentials
 * (env, Claude Code) by labelling the credential dropdown with a "detected
 * from host" hint when the backend resolves a key without one configured.
 */
export function EngineSettings() {
  const { endpoint, token } = useSession();
  const qc = useQueryClient();
  const engines = useQuery({
    queryKey: ["agent-engines", endpoint],
    queryFn: () => listEngines({ endpoint, token }),
    enabled: Boolean(endpoint),
  });
  const creds = useQuery({
    queryKey: ["credentials", endpoint],
    queryFn: () => listCredentials({ endpoint, token }),
    enabled: Boolean(endpoint),
  });

  const [kind, setKind] = useState<EngineKind>("ollama");
  const [model, setModel] = useState("");
  const [credentialId, setCredentialId] = useState<string | null>(null);
  const [host, setHost] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null);

  // Seed local state from the server's active config when it loads.
  useEffect(() => {
    if (engines.data) {
      const a = engines.data.active;
      setKind(a.kind);
      setModel(a.model);
      setCredentialId(a.credential_id ?? null);
      setHost(a.host ?? null);
    }
  }, [engines.data]);

  const activeProvider = engines.data?.available.find((e) => e.kind === kind);
  const apiCreds = (creds.data ?? []).filter((c) => c.kind === "api_key" || c.kind === "pat");

  const save = useMutation({
    mutationFn: (cfg: EngineConfig) => putEngine({ endpoint, token }, cfg),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["agent-engines", endpoint] }),
  });
  const probe = useMutation({
    mutationFn: (cfg: EngineConfig) => testEngine({ endpoint, token }, cfg),
    onSuccess: (r) => setTestResult({ ok: r.ok, msg: `${r.kind}/${r.model} reachable` }),
    onError: (e: Error) => setTestResult({ ok: false, msg: e.message }),
  });

  const cfg: EngineConfig = {
    kind,
    model,
    credential_id: credentialId,
    host,
  };

  return (
    <div className="rounded-lg border bg-card p-4">
      <h2 className="mb-1 text-sm font-semibold">Agent engine</h2>
      <p className="mb-4 text-xs text-muted-foreground">
        Pick the model Ask Pensieve uses. Anthropic / OpenAI keys are auto-detected
        from <code>ANTHROPIC_API_KEY</code>, <code>OPENAI_API_KEY</code>, or
        <code>~/.claude/.credentials.json</code> when no explicit credential is
        attached.
      </p>

      <div className="grid grid-cols-2 gap-3">
        <label className="block">
          <span className="mb-1 block text-[11px] font-medium text-muted-foreground">
            Provider
          </span>
          <select
            value={kind}
            onChange={(e) => {
              const k = e.target.value as EngineKind;
              setKind(k);
              const p = engines.data?.available.find((a) => a.kind === k);
              if (p && !p.models.includes(model)) setModel(p.models[0] ?? "");
            }}
            className="w-full rounded-md border bg-background px-2 py-1.5 text-xs"
          >
            {engines.data?.available.map((p) => (
              <option key={p.kind} value={p.kind}>
                {p.label}
              </option>
            ))}
          </select>
        </label>

        <label className="block">
          <span className="mb-1 block text-[11px] font-medium text-muted-foreground">
            Model
          </span>
          <select
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="w-full rounded-md border bg-background px-2 py-1.5 text-xs"
          >
            {activeProvider?.models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </label>

        {activeProvider?.needs_key && (
          <label className="block col-span-2">
            <span className="mb-1 block text-[11px] font-medium text-muted-foreground">
              Credential (optional — auto-detect if blank)
            </span>
            <select
              value={credentialId ?? ""}
              onChange={(e) => setCredentialId(e.target.value || null)}
              className="w-full rounded-md border bg-background px-2 py-1.5 text-xs"
            >
              <option value="">— auto-detect from host —</option>
              {apiCreds.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label} ({c.preview})
                </option>
              ))}
            </select>
          </label>
        )}

        {kind === "ollama" && (
          <label className="block col-span-2">
            <span className="mb-1 block text-[11px] font-medium text-muted-foreground">
              Ollama host
            </span>
            <input
              type="text"
              value={host ?? ""}
              onChange={(e) => setHost(e.target.value || null)}
              placeholder="http://localhost:11434"
              className="w-full rounded-md border bg-background px-2 py-1.5 text-xs"
            />
          </label>
        )}
      </div>

      <div className="mt-4 flex items-center gap-2">
        <button
          type="button"
          onClick={() => probe.mutate(cfg)}
          disabled={probe.isPending}
          className="rounded-md border px-3 py-1 text-xs hover:bg-accent"
        >
          {probe.isPending ? (
            <span className="inline-flex items-center gap-1">
              <Loader2 className="h-3 w-3 animate-spin" /> Testing…
            </span>
          ) : (
            "Test connection"
          )}
        </button>
        <button
          type="button"
          onClick={() => save.mutate(cfg)}
          disabled={save.isPending}
          className="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground hover:opacity-90"
        >
          {save.isPending ? "Saving…" : "Save"}
        </button>
        {testResult && (
          <span
            className={`inline-flex items-center gap-1 text-xs ${
              testResult.ok ? "text-emerald-600" : "text-destructive"
            }`}
          >
            {testResult.ok ? <Check className="h-3 w-3" /> : <X className="h-3 w-3" />}
            {testResult.msg}
          </span>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Mount the card on the Settings route**

In `web/src/routes/_app.settings.tsx`, find the existing settings layout and add `<EngineSettings />` next to the other cards (top of the list is fine):

```tsx
import { EngineSettings } from "@/features/settings/EngineSettings";
// …
<EngineSettings />
```

- [ ] **Step 3: Verify typecheck + dev server boots**

Run: `cd web && pnpm typecheck` (clean), then `pnpm dev` and visit `/settings`. The Agent Engine card renders, the provider dropdown has 3 options, switching it changes the model menu, and the "Test connection" button calls the backend.

- [ ] **Step 4: Commit**

```bash
git add web/src/features/settings/EngineSettings.tsx web/src/routes/_app.settings.tsx
git commit -m "feat(web): Settings → Agent Engine card (picker + test + save)"
```

---

## Task 17: End-to-end smoke test

**Files:** none — manual verification.

- [ ] **Step 1: Confirm the legacy default still works**

With `pensieve-bin` running fresh (Ollama at localhost), open `/agent` and ask a question. Expect: a normal answer streamed back. (This proves the refactor didn't break the existing path.)

- [ ] **Step 2: Confirm Anthropic flow works via auto-detect**

```
export ANTHROPIC_API_KEY=sk-ant-...
# restart pensieve-bin
```

In `/settings`, change provider to `Anthropic (Claude)`, pick `claude-sonnet-4-6`, click "Test connection" — expect green check with `anthropic/claude-sonnet-4-6 reachable`. Click "Save". Go to `/agent`, ask a question — answer should stream from Claude.

- [ ] **Step 3: Confirm Claude Code creds fallback**

```
unset ANTHROPIC_API_KEY
# ensure ~/.claude/.credentials.json contains an Anthropic key
# restart pensieve-bin
```

Test connection should still pass. Ask Pensieve should still work. (If you don't have Claude Code installed, skip this — A1's contract is "auto-detect when present", not "Claude Code must be present".)

- [ ] **Step 4: Confirm explicit credential override**

Create a new credential via `/credentials` UI with kind=API key, label "Backup Anthropic", value `sk-ant-other`. In `/settings`, attach it to the Anthropic engine, save. Verify the model_id in `agent_runs` (or the SSE `run_started` event) reflects the active config.

- [ ] **Step 5: Final commit (release notes)**

```bash
# No code change — bump CHANGELOG or release notes if your repo tracks them.
git commit --allow-empty -m "release(agent): A1 engine providers + Settings UI"
```

---

## Done criteria

- A user with `ANTHROPIC_API_KEY` (or Claude Code) opens Pensieve → Settings → Engine, picks Claude Sonnet, clicks Save, and Ask Pensieve answers their next question with Claude.
- The same flow works for OpenAI with `OPENAI_API_KEY`.
- The legacy Ollama default still works on a fresh deploy with no API keys.
- `cargo check -p pensieve-server` and `cd web && pnpm typecheck` are both clean.
- Reverting just `crates/pensieve-catalog/migrations/012_agent_engine_config.sql` is enough to roll back (the runner falls back to constants if the store read fails — handled in Task 11 Step 4).
