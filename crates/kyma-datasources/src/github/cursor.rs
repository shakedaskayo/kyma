//! Incremental-fetch cursor for the GitHub data source.
//!
//! The cursor is serialized as JSON and stored in `connector_cursors`. On
//! each tick we read it back, use the stored timestamps for `since`-filtered
//! pulls/issues queries, and write an updated cursor after ingestion.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-repository incremental state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoCursor {
    /// RFC-3339 timestamp: fetch pulls updated at or after this time.
    /// `None` means "first run" → fetch all.
    pub pulls_since: Option<DateTime<Utc>>,
    /// RFC-3339 timestamp: fetch issues updated at or after this time.
    /// `None` means "first run" → fetch all.
    pub issues_since: Option<DateTime<Utc>>,
    /// The default-branch HEAD SHA that was last fully ingested for the code
    /// graph.  When the current HEAD SHA matches this value, the code-graph
    /// walk is skipped.  `None` means the code graph has never been ingested
    /// for this repo.
    pub tree_sha_ingested: Option<String>,
    /// Number of files successfully ingested in the last code-graph tick.
    /// Used for resume when a tick hits `max_files_per_tick`: the next tick
    /// can skip already-processed paths (implementation detail, carried as
    /// informational state).
    pub files_done: Option<usize>,
    /// RFC-3339 timestamp: the `created_at` of the most-recent workflow run
    /// ingested. The next tick skips runs created at or before this time.
    /// `None` means Actions have never been ingested for this repo.
    #[serde(default)]
    pub workflows_since: Option<DateTime<Utc>>,
}

/// Top-level cursor — keyed by `"owner/name"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub repos: HashMap<String, RepoCursor>,
    /// Content signature of the full-refetch metadata (Repository/User/Branch
    /// nodes + OWNS/HAS_BRANCH edges) emitted on the last successful tick.
    /// These modules are re-fetched in full every poll (unlike the
    /// cursor-incremental pulls/issues), so without this gate a continuous
    /// data source would re-append identical rows on every tick. When the new
    /// signature matches, those rows are dropped before ingest. `None` = never
    /// ingested.
    #[serde(default)]
    pub refetch_signature: Option<String>,
}

impl Cursor {
    /// Deserialize from the `Option<Value>` the framework passes in.
    pub fn from_value(v: Option<&serde_json::Value>) -> Self {
        match v {
            None => Self::default(),
            Some(val) => serde_json::from_value(val.clone()).unwrap_or_default(),
        }
    }

    /// Serialize to a JSON value for storage.
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Get or create the cursor for a repo.
    pub fn for_repo(&self, repo: &str) -> RepoCursor {
        self.repos.get(repo).cloned().unwrap_or_default()
    }

    /// Update the metadata cursor for a repo after a successful fetch.
    pub fn update_repo(
        &mut self,
        repo: &str,
        pulls_since: Option<DateTime<Utc>>,
        issues_since: Option<DateTime<Utc>>,
    ) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        if let Some(ts) = pulls_since {
            entry.pulls_since = Some(ts);
        }
        if let Some(ts) = issues_since {
            entry.issues_since = Some(ts);
        }
    }

    /// Advance the Actions watermark for a repo to the newest run `created_at`
    /// seen this tick. Monotonic — never moves backwards.
    pub fn update_workflows_cursor(&mut self, repo: &str, newest_created: DateTime<Utc>) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        match entry.workflows_since {
            Some(prev) if prev >= newest_created => {}
            _ => entry.workflows_since = Some(newest_created),
        }
    }

    /// Update the code-graph cursor for a repo after a successful tree walk.
    ///
    /// `sha` is the default-branch HEAD commit SHA that was just ingested.
    /// `files_done` is the number of files that were processed this tick.
    pub fn update_code_cursor(&mut self, repo: &str, sha: String, files_done: usize) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        entry.tree_sha_ingested = Some(sha);
        entry.files_done = Some(files_done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty() {
        let c = Cursor::default();
        let v = c.to_value();
        let c2 = Cursor::from_value(Some(&v));
        assert!(c2.repos.is_empty());
    }

    #[test]
    fn round_trip_with_data() {
        let mut c = Cursor::default();
        let ts = Utc::now();
        c.update_repo("owner/repo", Some(ts), Some(ts));
        let v = c.to_value();
        let c2 = Cursor::from_value(Some(&v));
        let rc = c2.for_repo("owner/repo");
        assert!(rc.pulls_since.is_some());
        assert!(rc.issues_since.is_some());
    }

    #[test]
    fn missing_cursor_returns_default() {
        let c = Cursor::from_value(None);
        assert!(c.repos.is_empty());
    }

    #[test]
    fn code_cursor_round_trip() {
        let mut c = Cursor::default();
        c.update_code_cursor("owner/repo", "abc123sha".to_string(), 42);
        let v = c.to_value();
        let c2 = Cursor::from_value(Some(&v));
        let rc = c2.for_repo("owner/repo");
        assert_eq!(rc.tree_sha_ingested.as_deref(), Some("abc123sha"));
        assert_eq!(rc.files_done, Some(42));
    }

    #[test]
    fn tree_sha_default_is_none() {
        let c = Cursor::default();
        let rc = c.for_repo("any/repo");
        assert!(rc.tree_sha_ingested.is_none());
        assert!(rc.files_done.is_none());
        assert!(rc.workflows_since.is_none());
    }

    #[test]
    fn workflows_cursor_round_trip_and_monotonic() {
        let mut c = Cursor::default();
        let t1 = Utc::now();
        c.update_workflows_cursor("owner/repo", t1);
        // An older timestamp must not move the watermark backwards.
        c.update_workflows_cursor("owner/repo", t1 - chrono::Duration::hours(1));
        let v = c.to_value();
        let c2 = Cursor::from_value(Some(&v));
        assert_eq!(c2.for_repo("owner/repo").workflows_since, Some(t1));
    }
}
