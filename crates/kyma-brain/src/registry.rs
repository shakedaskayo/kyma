//! Brain registry: the configuration of each published brain plus its
//! mutable runtime state, behind a storage-agnostic trait. `kyma-server`
//! backs it with Postgres (`brain_repos` table); `kyma-local` with
//! `${KYMA_HOME}/brains.json`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::BrainError;

/// Which realms a brain includes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "realms", rename_all = "snake_case")]
pub enum RealmSelector {
    /// Every realm present in `memory_nodes`.
    All,
    /// An explicit list of realm names.
    Realms(Vec<String>),
}

impl RealmSelector {
    /// Layout mode is fixed at creation from the selector *shape*: a single
    /// literal realm renders flat; a list or `All` renders realm folders
    /// (even if the list currently matches one realm) so the tree never
    /// reshuffles as realms appear.
    pub fn layout(&self) -> VaultLayout {
        match self {
            Self::Realms(r) if r.len() == 1 => VaultLayout::Flat,
            _ => VaultLayout::ByRealm,
        }
    }
}

/// Directory layout of the vault, derived once from [`RealmSelector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultLayout {
    /// `notes/<type>/…` — single-realm brains.
    Flat,
    /// `notes/<realm>/<type>/…` — multi-realm brains.
    ByRealm,
}

/// Which memories a brain includes beyond the realm selector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainFilters {
    /// `None` = all types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_types: Option<Vec<String>>,
    /// Default `["active"]`; `background`/`archived` are opt-in.
    #[serde(default = "default_statuses")]
    pub statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_importance: Option<f64>,
    /// `<private>…</private>` spans are stripped before render.
    #[serde(default = "default_true")]
    pub redact_private_spans: bool,
    /// Bi-temporal: memories with `invalid_at` set are excluded by default.
    #[serde(default)]
    pub include_invalidated: bool,
}

fn default_statuses() -> Vec<String> {
    vec!["active".to_string()]
}

fn default_true() -> bool {
    true
}

impl Default for BrainFilters {
    fn default() -> Self {
        Self {
            memory_types: None,
            statuses: default_statuses(),
            min_importance: None,
            redact_private_spans: true,
            include_invalidated: false,
        }
    }
}

/// Agentic wiki-gardener settings for a brain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GardenerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gardener_interval")]
    pub interval_secs: u64,
}

fn default_gardener_interval() -> u64 {
    86_400
}

impl Default for GardenerConfig {
    fn default() -> Self {
        Self { enabled: false, interval_secs: default_gardener_interval() }
    }
}

/// Immutable-ish configuration of a brain (name never changes; the rest is
/// editable through `PUT /v1/brain/:name`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainConfig {
    pub name: String,
    pub realms: RealmSelector,
    /// Fixed at creation from the selector shape (see [`RealmSelector::layout`]).
    pub layout: VaultLayout,
    #[serde(default)]
    pub include: BrainFilters,
    /// Minimum role required to clone: "read" | "write" | "admin". Push
    /// always requires write.
    #[serde(default = "default_visibility")]
    pub visibility_role: String,
    /// Seconds between scheduled exports; `0` = manual only.
    #[serde(default = "default_export_interval")]
    pub export_interval_secs: u64,
    #[serde(default)]
    pub gardener: GardenerConfig,
    pub created_at: String,
    pub updated_at: String,
}

fn default_visibility() -> String {
    "read".to_string()
}

fn default_export_interval() -> u64 {
    900
}

impl BrainConfig {
    /// Construct with layout derived from the selector.
    pub fn new(name: &str, realms: RealmSelector, now_rfc3339: &str) -> Result<Self, BrainError> {
        crate::validate_name(name)?;
        if let RealmSelector::Realms(list) = &realms {
            if list.is_empty() {
                return Err(BrainError::Other("realm list must not be empty".into()));
            }
            for r in list {
                if !r.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
                    return Err(BrainError::Other(format!("invalid realm name: {r}")));
                }
            }
        }
        let layout = realms.layout();
        Ok(Self {
            name: name.to_string(),
            realms,
            layout,
            include: BrainFilters::default(),
            visibility_role: default_visibility(),
            export_interval_secs: default_export_interval(),
            gardener: GardenerConfig::default(),
            created_at: now_rfc3339.to_string(),
            updated_at: now_rfc3339.to_string(),
        })
    }
}

/// One export / push-ingest / gardener run, kept in a capped ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainRunRecord {
    /// "export" | "push_ingest" | "gardener".
    pub kind: String,
    pub started_at: String,
    pub finished_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default)]
    pub files_written: u64,
    #[serde(default)]
    pub files_deleted: u64,
    #[serde(default)]
    pub notes_ingested: u64,
    #[serde(default)]
    pub noop: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Max runs retained per brain.
pub const RUN_RING: usize = 50;

/// Mutable runtime state of a brain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrainRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_export_at: Option<String>,
    /// Informational only; ref truth is always `git rev-parse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_gardener_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Note count at last export (list-view stat).
    #[serde(default)]
    pub note_count: u64,
    /// Newest first, capped at [`RUN_RING`].
    #[serde(default)]
    pub runs: Vec<BrainRunRecord>,
}

impl BrainRuntime {
    /// Push a run record, keeping the ring capped.
    pub fn record_run(&mut self, run: BrainRunRecord) {
        self.runs.insert(0, run);
        self.runs.truncate(RUN_RING);
    }
}

/// A registry row: config + runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainRecord {
    pub config: BrainConfig,
    #[serde(default)]
    pub runtime: BrainRuntime,
}

/// Storage-agnostic registry of brains.
#[async_trait]
pub trait BrainRegistry: Send + Sync + 'static {
    async fn list(&self) -> Result<Vec<BrainRecord>, BrainError>;
    async fn get(&self, name: &str) -> Result<Option<BrainRecord>, BrainError>;
    async fn upsert_config(&self, cfg: &BrainConfig) -> Result<(), BrainError>;
    async fn delete(&self, name: &str) -> Result<(), BrainError>;
    async fn update_runtime(&self, name: &str, rt: &BrainRuntime) -> Result<(), BrainError>;
}
