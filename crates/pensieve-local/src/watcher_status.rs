//! Local-mode watcher registry: kyma-local is a single process with SQLite,
//! so cc-sync watcher state lives in memory and is served through the same
//! `/v1/data-sources/watchers` shape as the control plane's Postgres registry
//! (`kyma_datasources::watchers::WatcherRow`) — the web UI's watchers tab
//! works identically in both modes.

use axum::{extract::State, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use crate::watcher_settings::WatcherSettings;

/// One registered in-process watcher. Mirrors the control plane's
/// `WatcherRow` field-for-field; `stale` is computed read-side (see
/// [`LocalWatcher::to_json`]) so it is not stored here.
#[derive(Debug, Clone)]
pub struct LocalWatcher {
    /// Stable id, e.g. `"cc-sync"` (one watcher per kind per process).
    pub id: String,
    /// Watcher kind, e.g. `"cc_sync"`.
    pub kind: String,
    pub node_host: String,
    pub node_id: String,
    /// OS user the process runs as.
    pub identity: String,
    /// Watcher config (`{poll_secs, root}` for cc-sync). `poll_secs` feeds
    /// the read-side staleness computation.
    pub config: Value,
    /// RFC 3339; captured once when the watcher loop starts.
    pub started_at: String,
    /// RFC 3339; bumped every poll cycle, including error cycles.
    pub last_heartbeat_at: String,
    /// Rollup of the most recent successful scan
    /// (`{seen, processed, errors, duration_ms, at, detail?}`).
    pub last_scan: Option<Value>,
}

impl LocalWatcher {
    /// Serialize to the wire shape, computing `stale` against `now`: the
    /// heartbeat is older than 3x the poll interval (`config.poll_secs`,
    /// default 30s — same rule the control plane computes in SQL). Computed
    /// at read time so a wedged sync loop shows stale even though nothing
    /// is updating the store. An unparseable heartbeat counts as stale.
    fn to_json(&self, now: DateTime<Utc>) -> Value {
        let poll_secs = self
            .config
            .get("poll_secs")
            .and_then(Value::as_f64)
            .unwrap_or(30.0);
        let stale = DateTime::parse_from_rfc3339(&self.last_heartbeat_at)
            .map(|hb| {
                let age_secs = (now - hb.with_timezone(&Utc)).num_milliseconds() as f64 / 1000.0;
                age_secs > 3.0 * poll_secs
            })
            .unwrap_or(true);
        json!({
            "id": self.id,
            "kind": self.kind,
            "node_host": self.node_host,
            "node_id": self.node_id,
            "identity": self.identity,
            "config": self.config,
            "started_at": self.started_at,
            "last_heartbeat_at": self.last_heartbeat_at,
            "last_scan": self.last_scan,
            "stale": stale,
        })
    }
}

/// `(hostname, node_id, os user)` — same resolution as
/// `kyma_datasources::watchers::node_identity` (inlined: kyma-local must not
/// depend on the Postgres-backed datasources crate). `KYMA_NODE_ID` overrides
/// the node id (defaults to the hostname).
/// Keep in sync with `kyma_datasources::watchers::node_identity`.
pub fn node_identity() -> (String, String, String) {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    let node_id = std::env::var("KYMA_NODE_ID").unwrap_or_else(|_| host.clone());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    (host, node_id, user)
}

/// Shared in-process watcher store. Cheap to clone (Arc inside); the cc-sync
/// loop holds one clone and upserts each cycle, the router holds another and
/// serves reads. `cc_sync_enabled` is the runtime toggle — flipped by the
/// settings API and persisted to `~/.kyma/watcher-settings.json`.
#[derive(Clone)]
pub struct LocalWatcherStatus {
    watchers: Arc<RwLock<Vec<LocalWatcher>>>,
    pub cc_sync_enabled: Arc<AtomicBool>,
}

impl Default for LocalWatcherStatus {
    fn default() -> Self {
        Self {
            watchers: Arc::new(RwLock::new(vec![])),
            cc_sync_enabled: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl LocalWatcherStatus {
    /// Replace the watcher with the same `id`, or append if new.
    pub fn upsert(&self, w: LocalWatcher) {
        let mut items = self.watchers.write().unwrap_or_else(|e| e.into_inner());
        match items.iter_mut().find(|x| x.id == w.id) {
            Some(slot) => *slot = w,
            None => items.push(w),
        }
    }

    /// Remove the watcher with the given `id` from the visible list.
    pub fn remove(&self, id: &str) {
        let mut items = self.watchers.write().unwrap_or_else(|e| e.into_inner());
        items.retain(|w| w.id != id);
    }

    pub fn cc_sync_enabled(&self) -> bool {
        self.cc_sync_enabled.load(Ordering::Relaxed)
    }

    pub fn set_cc_sync_enabled(&self, v: bool) {
        self.cc_sync_enabled.store(v, Ordering::Relaxed);
    }

    /// Router serving `GET /v1/data-sources/watchers` → `{"items": [...]}` —
    /// the same path + shape as the control plane's Postgres-backed handler.
    /// Mounted unconditionally (even with `KYMA_CC_WATCH=0`) so the web tab
    /// renders an empty state instead of a 404.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/v1/data-sources/watchers", get(list_watchers))
            .with_state(self.clone())
    }

    /// Router for watcher settings — always mounted regardless of ds_admin.
    /// `GET /v1/data-sources/watchers/settings` — current settings.
    /// `PUT /v1/data-sources/watchers/settings` — update settings (persisted).
    pub fn settings_router(&self) -> Router {
        Router::new()
            .route(
                "/v1/data-sources/watchers/settings",
                get(get_settings).put(put_settings),
            )
            .with_state(self.clone())
    }

    pub fn items_json(&self, now: DateTime<Utc>) -> Value {
        let items: Vec<Value> = self
            .watchers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|w| w.to_json(now))
            .collect();
        json!({ "items": items })
    }
}

async fn list_watchers(State(s): State<LocalWatcherStatus>) -> Json<Value> {
    Json(s.items_json(Utc::now()))
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsResponse {
    cc_sync_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SettingsPatch {
    cc_sync_enabled: Option<bool>,
}

async fn get_settings(State(s): State<LocalWatcherStatus>) -> Json<SettingsResponse> {
    Json(SettingsResponse {
        cc_sync_enabled: s.cc_sync_enabled(),
    })
}

async fn put_settings(
    State(s): State<LocalWatcherStatus>,
    Json(patch): Json<SettingsPatch>,
) -> Json<SettingsResponse> {
    if let Some(enabled) = patch.cc_sync_enabled {
        s.set_cc_sync_enabled(enabled);
        let settings = WatcherSettings {
            cc_sync_enabled: enabled,
        };
        tokio::spawn(async move {
            let _ = settings.save().await;
        });
    }
    Json(SettingsResponse {
        cc_sync_enabled: s.cc_sync_enabled(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watcher(id: &str, heartbeat: DateTime<Utc>, poll_secs: u64) -> LocalWatcher {
        LocalWatcher {
            id: id.to_string(),
            kind: "cc_sync".into(),
            node_host: "testhost".into(),
            node_id: "testhost".into(),
            identity: "tester".into(),
            config: json!({ "poll_secs": poll_secs, "root": "~/.claude/projects" }),
            started_at: heartbeat.to_rfc3339(),
            last_heartbeat_at: heartbeat.to_rfc3339(),
            last_scan: None,
        }
    }

    #[test]
    fn upsert_replaces_by_id() {
        let status = LocalWatcherStatus::default();
        let now = Utc::now();
        status.upsert(watcher("cc-sync", now, 30));
        let mut updated = watcher("cc-sync", now, 30);
        updated.last_scan = Some(json!({ "seen": 5 }));
        status.upsert(updated);
        status.upsert(watcher("other", now, 30));

        let items = status.items_json(now);
        let items = items["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "replace by id, append new ids");
        assert_eq!(items[0]["id"], "cc-sync");
        assert_eq!(items[0]["last_scan"]["seen"], 5);
        assert_eq!(items[1]["id"], "other");
    }

    #[test]
    fn remove_by_id() {
        let status = LocalWatcherStatus::default();
        let now = Utc::now();
        status.upsert(watcher("cc-sync", now, 30));
        status.upsert(watcher("other", now, 30));
        status.remove("cc-sync");
        let items = status.items_json(now)["items"].as_array().unwrap().clone();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "other");
    }

    #[test]
    fn stale_computed_from_heartbeat_and_poll() {
        let status = LocalWatcherStatus::default();
        let now = Utc::now();
        // Fresh heartbeat: not stale.
        status.upsert(watcher("fresh", now, 30));
        // 91s-old heartbeat with poll_secs=30 → older than 3x poll → stale.
        status.upsert(watcher("old", now - chrono::Duration::seconds(91), 30));
        // Same age but a 60s poll → within 3x poll → not stale.
        status.upsert(watcher("slow-poll", now - chrono::Duration::seconds(91), 60));

        let items = status.items_json(now);
        let items = items["items"].as_array().unwrap();
        assert_eq!(items[0]["stale"], false);
        assert_eq!(items[1]["stale"], true);
        assert_eq!(items[2]["stale"], false);
    }

    #[test]
    fn stale_defaults_poll_to_30s_and_handles_bad_heartbeat() {
        let status = LocalWatcherStatus::default();
        let now = Utc::now();
        let mut no_poll = watcher("no-poll", now - chrono::Duration::seconds(91), 30);
        no_poll.config = json!({}); // no poll_secs → default 30 → 91s is stale
        status.upsert(no_poll);
        let mut bad = watcher("bad-hb", now, 30);
        bad.last_heartbeat_at = "not-a-timestamp".into();
        status.upsert(bad);

        let items = status.items_json(now);
        let items = items["items"].as_array().unwrap();
        assert_eq!(items[0]["stale"], true);
        assert_eq!(items[1]["stale"], true, "unparseable heartbeat counts as stale");
    }
}
