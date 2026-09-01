//! Tunable data-retention settings + resolution.
//!
//! One row per tenant (JSONB in `retention_settings`). Resolves an effective
//! retention (in days) for:
//!   * object-store **artifacts** (CI job logs, contributed files, fs-watch
//!     snapshots) — by artifact class, then source, then global default.
//!   * columnar **tables** — by explicit per-table override, then source, then
//!     global default.
//!
//! `None` at every level means *retain forever*. Lives in `pensieve-core` (a plain
//! serde struct + pure resolver) so both the settings API (pensieve-server) and the
//! retention worker (pensieve-compaction) use it without a dependency cycle.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Per-tenant retention configuration. All day-counts are "delete data older
/// than N days"; absence anywhere = retain forever.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionSettings {
    /// Fallback when nothing more specific matches. `None` = retain forever.
    pub global_default_days: Option<u32>,
    /// Per data-source (data source type / origin), e.g. `"github"`, `"fswatch"`.
    pub per_source_days: HashMap<String, u32>,
    /// Per columnar table, keyed `"database.table"`. Applies to extents.
    pub per_table_days: HashMap<String, u32>,
    /// Per artifact class, e.g. `"log"`, `"file"`. Applies to object-store blobs.
    pub per_artifact_class_days: HashMap<String, u32>,
}

impl RetentionSettings {
    /// Effective retention for an artifact: class → source → global.
    /// `None` = retain forever.
    pub fn artifact_days(&self, source: &str, artifact_class: &str) -> Option<u32> {
        self.per_artifact_class_days
            .get(artifact_class)
            .copied()
            .or_else(|| self.per_source_days.get(source).copied())
            .or(self.global_default_days)
    }

    /// Effective retention for a columnar table: explicit per-table → source →
    /// global. `table_ref` is `"database.table"`. `None` = retain forever.
    pub fn table_days(&self, table_ref: &str, source: Option<&str>) -> Option<u32> {
        self.per_table_days
            .get(table_ref)
            .copied()
            .or_else(|| source.and_then(|s| self.per_source_days.get(s).copied()))
            .or(self.global_default_days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> RetentionSettings {
        RetentionSettings {
            global_default_days: Some(90),
            per_source_days: HashMap::from([("github".to_string(), 30)]),
            per_table_days: HashMap::from([("pensieve.events".to_string(), 7)]),
            per_artifact_class_days: HashMap::from([("log".to_string(), 14)]),
        }
    }

    #[test]
    fn artifact_precedence_class_over_source_over_global() {
        let s = s();
        // class wins
        assert_eq!(s.artifact_days("github", "log"), Some(14));
        // no class match → source
        assert_eq!(s.artifact_days("github", "file"), Some(30));
        // no class/source → global
        assert_eq!(s.artifact_days("fswatch", "file"), Some(90));
    }

    #[test]
    fn table_precedence_table_over_source_over_global() {
        let s = s();
        assert_eq!(s.table_days("pensieve.events", Some("github")), Some(7));
        assert_eq!(s.table_days("pensieve.other", Some("github")), Some(30));
        assert_eq!(s.table_days("pensieve.other", None), Some(90));
    }

    #[test]
    fn empty_settings_retain_forever() {
        let s = RetentionSettings::default();
        assert_eq!(s.artifact_days("github", "log"), None);
        assert_eq!(s.table_days("pensieve.events", Some("github")), None);
    }

    #[test]
    fn global_none_with_specific_some() {
        let s = RetentionSettings {
            global_default_days: None,
            per_artifact_class_days: HashMap::from([("log".to_string(), 5)]),
            ..Default::default()
        };
        assert_eq!(s.artifact_days("github", "log"), Some(5));
        // falls through to None (forever) when nothing matches
        assert_eq!(s.artifact_days("github", "file"), None);
    }
}
