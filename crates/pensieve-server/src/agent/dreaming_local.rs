//! Degraded local-mode state for dreaming runs.
//!
//! In `pensieve serve` (single-binary local mode) there is no Postgres and no
//! worker fabric: dreaming runs execute INLINE as a tokio task in the serve
//! process. This module holds the state those inline runs read and write —
//! the same Run JSON shape the HTTP layer serves from Postgres, plus the live
//! progress snapshot and the per-run conversation trace.
//!
//! Durability: finished runs are persisted into the embedded SQLite catalog's
//! `local_dreaming_runs` table (via [`pensieve_catalog_sqlite::SqliteCatalog`]),
//! and the in-memory ring is hydrated from it on startup. When the catalog is
//! not a `SqliteCatalog` (e.g. some test catalogs), the store degrades to
//! in-memory-only — the ring still serves the UI for the process lifetime, but
//! runs do not survive a restart.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use uuid::Uuid;

use pensieve_core::catalog::Catalog;

/// How many finished runs to keep in the in-memory ring (and hydrate on boot).
const RUN_RING: usize = 50;

/// One run held in memory: the Run JSON (the `memory_pipeline_runs` shape the
/// HTTP layer serves) and the agent_runs conversation trace.
#[derive(Clone)]
struct RunEntry {
    id: Uuid,
    agent_run_id: Option<Uuid>,
    /// The full Run JSON object served at `…/dreaming/runs/:id`.
    run: Value,
    /// The agent-run trace array served at `/v1/agent/runs/:agent_run_id`.
    trace: Value,
}

/// In-memory + SQLite-backed state for local dreaming runs.
pub struct LocalDreamingStore {
    /// Finished runs, newest first. Capped at [`RUN_RING`].
    ring: Mutex<VecDeque<RunEntry>>,
    /// The currently-running run, if any (its live Run JSON, refreshed as
    /// progress snapshots arrive). `None` when idle.
    running: Mutex<Option<RunEntry>>,
    /// One-in-flight guard: `true` while a run is executing. The manual-trigger
    /// handler swaps this to dedupe concurrent runs.
    in_flight: AtomicBool,
    /// Catalog handle for durable persistence. We downcast to `SqliteCatalog`
    /// where available; otherwise the store is in-memory-only.
    catalog: Arc<dyn Catalog>,
}

impl LocalDreamingStore {
    /// Build the store and hydrate the ring from the SQLite catalog (if the
    /// catalog is a `SqliteCatalog`).
    pub async fn new(catalog: Arc<dyn Catalog>) -> Arc<Self> {
        let store = Arc::new(Self {
            ring: Mutex::new(VecDeque::with_capacity(RUN_RING)),
            running: Mutex::new(None),
            in_flight: AtomicBool::new(false),
            catalog,
        });
        store.hydrate().await;
        store
    }

    fn sqlite(&self) -> Option<&pensieve_catalog_sqlite::SqliteCatalog> {
        self.catalog
            .as_ref_any()
            .downcast_ref::<pensieve_catalog_sqlite::SqliteCatalog>()
    }

    /// Load the most recent runs from SQLite into the ring (newest first).
    async fn hydrate(&self) {
        let Some(sqlite) = self.sqlite() else {
            tracing::debug!("local dreaming store: non-SQLite catalog — runs are in-memory only");
            return;
        };
        match sqlite.recent_dreaming_runs(RUN_RING as i64).await {
            Ok(rows) => {
                let mut ring = self.ring.lock().unwrap();
                for (run_s, trace_s) in rows {
                    let run: Value = serde_json::from_str(&run_s).unwrap_or(Value::Null);
                    let trace: Value = serde_json::from_str(&trace_s).unwrap_or_else(|_| json!([]));
                    let id = run
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .unwrap_or_else(Uuid::new_v4);
                    let agent_run_id = run
                        .get("agent_run_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());
                    ring.push_back(RunEntry {
                        id,
                        agent_run_id,
                        run,
                        trace,
                    });
                }
                tracing::info!(runs = ring.len(), "local dreaming store hydrated from catalog");
            }
            Err(e) => tracing::warn!(error = %e, "local dreaming store: hydrate failed"),
        }
    }

    /// Try to acquire the single in-flight slot. Returns `true` when acquired
    /// (caller must call [`release`](Self::release) when the run ends), `false`
    /// when a run is already in flight (the caller returns `deduped`).
    pub fn try_acquire(&self) -> bool {
        self.in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// Release the in-flight slot.
    pub fn release(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }

    /// Is a run currently in flight?
    pub fn is_running(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Record the start of a run: it becomes the `running` entry.
    pub fn start_run(&self, id: Uuid, agent_run_id: Uuid, run: Value) {
        *self.running.lock().unwrap() = Some(RunEntry {
            id,
            agent_run_id: Some(agent_run_id),
            run,
            trace: json!([]),
        });
    }

    /// Update the live progress snapshot of the running run. Merges the
    /// `progress` object into the running entry's Run JSON.
    pub fn set_progress(&self, id: Uuid, progress: Value) {
        let mut g = self.running.lock().unwrap();
        if let Some(entry) = g.as_mut() {
            if entry.id == id {
                if let Value::Object(ref mut m) = entry.run {
                    m.insert("progress".into(), progress);
                }
            }
        }
    }

    /// Finalize the run: merge the terminal fields (`finished`) onto the run
    /// that was started via [`start_run`] (preserving `mode`/`trigger`/
    /// `started_at`), move it from `running` into the ring, attach the trace,
    /// and persist it durably to SQLite. `finished` carries the keys that
    /// change at the end of a run (status, finished_at, stats, memories_written,
    /// error, progress).
    pub async fn finalize_run(&self, id: Uuid, finished: Value, trace: Value) {
        // Start from the running entry's run JSON (which has mode/trigger/
        // started_at), falling back to `finished` alone if the running slot was
        // already cleared (e.g. after a restart hydrate path).
        let base = {
            let mut g = self.running.lock().unwrap();
            let taken = if g.as_ref().map(|e| e.id) == Some(id) {
                g.take().map(|e| e.run)
            } else {
                None
            };
            taken.unwrap_or_else(|| json!({ "id": id.to_string() }))
        };
        let mut run = base;
        if let (Value::Object(dst), Value::Object(src)) = (&mut run, &finished) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        let agent_run_id = run
            .get("agent_run_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        let started_at = run
            .get("started_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let entry = RunEntry {
            id,
            agent_run_id,
            run: run.clone(),
            trace: trace.clone(),
        };
        // Push to the front of the ring, cap to RUN_RING.
        {
            let mut ring = self.ring.lock().unwrap();
            ring.retain(|e| e.id != id);
            ring.push_front(entry);
            while ring.len() > RUN_RING {
                ring.pop_back();
            }
        }
        // Durable persistence (best-effort).
        if let Some(sqlite) = self.sqlite() {
            let run_s = serde_json::to_string(&run).unwrap_or_else(|_| "{}".into());
            let trace_s = serde_json::to_string(&trace).unwrap_or_else(|_| "[]".into());
            if let Err(e) = sqlite
                .upsert_dreaming_run(id, agent_run_id, &started_at, &run_s, &trace_s)
                .await
            {
                tracing::warn!(error = %e, "local dreaming store: persist failed");
            }
        }
    }

    /// All runs newest-first: the running one (if any) followed by the ring.
    pub fn list_runs(&self, limit: usize, offset: usize) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        if let Some(entry) = self.running.lock().unwrap().as_ref() {
            out.push(entry.run.clone());
        }
        for entry in self.ring.lock().unwrap().iter() {
            out.push(entry.run.clone());
        }
        out.into_iter().skip(offset).take(limit).collect()
    }

    /// One run by id (running or finished). Returns the Run JSON.
    pub fn get_run(&self, id: Uuid) -> Option<Value> {
        if let Some(entry) = self.running.lock().unwrap().as_ref() {
            if entry.id == id {
                return Some(entry.run.clone());
            }
        }
        self.ring
            .lock().unwrap()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.run.clone())
    }

    /// The agent-run trace for an `agent_run_id` (the conversation drilldown).
    /// Returns `(run_json, trace_json)` so the HTTP layer can synthesize the
    /// `/v1/agent/runs/:id` envelope. Searches the running run then the ring.
    pub fn get_agent_run(&self, agent_run_id: Uuid) -> Option<(Value, Value)> {
        if let Some(entry) = self.running.lock().unwrap().as_ref() {
            if entry.agent_run_id == Some(agent_run_id) {
                return Some((entry.run.clone(), entry.trace.clone()));
            }
        }
        self.ring
            .lock().unwrap()
            .iter()
            .find(|e| e.agent_run_id == Some(agent_run_id))
            .map(|e| (e.run.clone(), e.trace.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensieve_catalog_sqlite::SqliteCatalog;

    async fn store() -> Arc<LocalDreamingStore> {
        let cat: Arc<dyn Catalog> = Arc::new(SqliteCatalog::connect_in_memory().await.unwrap());
        LocalDreamingStore::new(cat).await
    }

    fn run_json(id: Uuid, agent_run_id: Uuid, status: &str) -> Value {
        json!({
            "id": id.to_string(),
            "agent_run_id": agent_run_id.to_string(),
            "status": status,
            "started_at": chrono::Utc::now().to_rfc3339(),
        })
    }

    #[tokio::test]
    async fn in_flight_guard_dedupes() {
        let s = store().await;
        assert!(s.try_acquire(), "first acquire succeeds");
        assert!(!s.try_acquire(), "second acquire is deduped");
        assert!(s.is_running());
        s.release();
        assert!(!s.is_running());
        assert!(s.try_acquire(), "acquire after release succeeds");
    }

    #[tokio::test]
    async fn start_progress_finalize_roundtrip() {
        let s = store().await;
        let id = Uuid::new_v4();
        let arid = Uuid::new_v4();
        s.start_run(id, arid, run_json(id, arid, "running"));

        // Running run is listed and fetchable.
        assert_eq!(s.list_runs(10, 0).len(), 1);
        assert_eq!(
            s.get_run(id).unwrap().get("status").unwrap().as_str(),
            Some("running")
        );

        // Live progress merges onto the running entry.
        s.set_progress(id, json!({ "current_phase": "reviewing" }));
        let prog = s.get_run(id).unwrap();
        assert_eq!(
            prog.pointer("/progress/current_phase").unwrap().as_str(),
            Some("reviewing")
        );

        // Finalize moves it to the ring with the final status + trace.
        let trace = json!([{ "event": "tool_call", "data": { "tool": "save_memory" } }]);
        s.finalize_run(id, run_json(id, arid, "success"), trace.clone())
            .await;
        assert_eq!(
            s.get_run(id).unwrap().get("status").unwrap().as_str(),
            Some("success")
        );
        let (_run, got_trace) = s.get_agent_run(arid).unwrap();
        assert_eq!(got_trace, trace);
        assert_eq!(s.list_runs(10, 0).len(), 1, "no duplicate after finalize");
    }

    #[tokio::test]
    async fn finalized_runs_persist_and_hydrate() {
        // A run finalized on one store instance is visible after re-opening the
        // SAME catalog (durability via the SQLite local_dreaming_runs table).
        let cat: Arc<dyn Catalog> = Arc::new(SqliteCatalog::connect_in_memory().await.unwrap());
        let id = Uuid::new_v4();
        let arid = Uuid::new_v4();
        {
            let s = LocalDreamingStore::new(cat.clone()).await;
            s.finalize_run(id, run_json(id, arid, "success"), json!([]))
                .await;
        }
        let s2 = LocalDreamingStore::new(cat.clone()).await;
        assert_eq!(s2.list_runs(10, 0).len(), 1, "hydrated from catalog");
        assert_eq!(
            s2.get_run(id).unwrap().get("status").unwrap().as_str(),
            Some("success")
        );
    }

    #[tokio::test]
    async fn ring_caps_at_fifty() {
        let s = store().await;
        for _ in 0..(RUN_RING + 10) {
            let id = Uuid::new_v4();
            let arid = Uuid::new_v4();
            s.finalize_run(id, run_json(id, arid, "success"), json!([]))
                .await;
        }
        assert_eq!(s.list_runs(1000, 0).len(), RUN_RING);
    }
}
