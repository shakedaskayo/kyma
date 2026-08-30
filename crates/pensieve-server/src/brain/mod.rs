//! Brain repos server surface: `/v1/brain` management API and the
//! `/git/<name>.git` smart-HTTP service.
//!
//! The deterministic core (vault planning, git wrapper, ingest planning)
//! lives in the pure `kyma-brain` crate; this module supplies the HTTP
//! handlers, memory-table IO, per-brain locking, and run recording. Mounted
//! by both app assemblies (`kyma-bin` hosted, `kyma-local` serve).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kyma_brain::gitbin::GitBin;
use kyma_brain::registry::BrainRegistry;

use crate::agent::state::AgentState;

pub mod fetch;
pub mod git_http;
pub mod pg_registry;
pub mod routes;
pub mod scheduler;

/// Shared state for the brain routers.
#[derive(Clone)]
pub struct BrainState {
    pub registry: Arc<dyn BrainRegistry>,
    /// `None` ⇒ no git binary on this host: management API answers with
    /// `git_available: false` and repo-touching endpoints return 503.
    pub git: Option<Arc<GitBin>>,
    /// Directory holding the bare repos (`<brain_dir>/<name>.git`).
    pub brain_dir: PathBuf,
    /// Per-brain serialization: exports and receive-pack ingests hold this
    /// so the exporter can never miss a landed-but-uningested push.
    pub locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Memory-table IO (fetch rows for export, MemoryWriter for ingest).
    pub agent: AgentState,
}

impl BrainState {
    pub fn new(
        registry: Arc<dyn BrainRegistry>,
        git: Option<Arc<GitBin>>,
        brain_dir: PathBuf,
        agent: AgentState,
    ) -> Self {
        Self { registry, git, brain_dir, locks: Arc::new(Mutex::new(HashMap::new())), agent }
    }

    /// The per-brain lock, created on first use.
    pub fn lock_for(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.locks.lock().expect("brain lock map poisoned");
        map.entry(name.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Repo directory for a brain (name is validated at create time; the
    /// join is safe because valid names contain no separators).
    pub fn repo_dir(&self, name: &str) -> PathBuf {
        self.brain_dir.join(format!("{name}.git"))
    }
}
