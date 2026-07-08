//! Data shapes crossing the crate boundary: the memory rows the caller
//! fetches from the memory tables, the planned vault files the exporter
//! commits, and the manifest that records which paths the exporter owns.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One latest-version memory node row, as fetched by the caller from
/// `memory_nodes` (embedding and other internal columns already excluded).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteRow {
    /// Bare uuid string (without the `memory:` node-id prefix).
    pub id: String,
    pub realm: String,
    /// fact | decision | preference | learning | summary | procedure | entity.
    pub memory_type: String,
    pub title: String,
    pub content: String,
    /// Comma-joined as stored; split by the renderer.
    pub tags: String,
    pub importance: f64,
    /// active | background | archived.
    pub status: String,
    /// RFC3339 strings, copied verbatim into frontmatter.
    pub created_at: String,
    pub updated_at: String,
    pub valid_at: Option<String>,
    pub invalid_at: Option<String>,
    pub topic_key: Option<String>,
}

/// One memory edge among the included nodes (node ids carry the `memory:`
/// prefix as stored in `memory_edges`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRow {
    pub src: String,
    pub dst: String,
    pub edge_type: String,
}

/// A file the exporter will commit: repo-relative path (forward slashes)
/// plus exact bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// One exporter-owned path recorded in `.kyma/manifest.json`. `memory_id`
/// is set for note files (the push-ingest key for deletions) and `None` for
/// generated files like `index.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
}

/// The exporter-owned manifest. Every path the exporter wrote last run is
/// listed here; anything in the tree but absent from the manifest is a
/// user/agent file the next export must preserve.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub const CURRENT_VERSION: u32 = 1;

    /// Deterministic JSON bytes (entries sorted by path, trailing newline).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut m = self.clone();
        m.entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = serde_json::to_vec_pretty(&m).unwrap_or_default();
        out.push(b'\n');
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// `memory_id → path` for filename stability across exports.
    pub fn paths_by_memory_id(&self) -> BTreeMap<String, String> {
        self.entries
            .iter()
            .filter_map(|e| e.memory_id.clone().map(|id| (id, e.path.clone())))
            .collect()
    }

    /// `path → memory_id` for push-ingest deletion handling.
    pub fn memory_ids_by_path(&self) -> BTreeMap<String, String> {
        self.entries
            .iter()
            .filter_map(|e| e.memory_id.clone().map(|id| (e.path.clone(), id)))
            .collect()
    }

    /// The set of exporter-owned paths.
    pub fn owned_paths(&self) -> std::collections::BTreeSet<String> {
        self.entries.iter().map(|e| e.path.clone()).collect()
    }
}
