//! Process-global cache of built graph topologies (S3.2).
//!
//! A deep `subgraph` traversal builds a whole-graph [`CsrGraph`] from one edge
//! scan and keeps the edge rows (with properties) needed to assemble the
//! payload. Rebuilding that on every deep query rescans the edge table and
//! re-runs the build; this cache reuses it across queries on the same graph.
//!
//! Invalidation is by a cheap `(node_count, edge_count)` **fingerprint**: two
//! `COUNT(*)` probes are far cheaper than rescanning + rebuilding, and any
//! append-only mutation (the data-source ingest model) moves the counts, so a
//! changed graph is never served stale. The cache is bounded (LRU over a small
//! entry cap) because each entry retains its edge set in memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kyma_graph_topo::CsrGraph;

use crate::types::GraphRelationship;

/// Max cached graphs. Small because each entry retains up to
/// `KYMA_GRAPH_TOPO_MAX_EDGES` edge rows; operators trade memory for hit rate
/// via that cap.
const MAX_ENTRIES: usize = 4;

/// Identifies a graph by its backing tables.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TopoKey {
    pub database: String,
    pub node_table: String,
    pub edge_table: String,
}

/// A built topology plus the edge rows needed to assemble subgraph payloads.
pub(crate) struct TopoSnapshot {
    /// `(node_count, edge_count)` at build time — the invalidation fingerprint.
    pub fingerprint: (usize, usize),
    pub csr: CsrGraph,
    pub edges: Vec<GraphRelationship>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<TopoKey, Arc<TopoSnapshot>>,
    /// LRU order: front = least-recently-used.
    lru: Vec<TopoKey>,
}

/// A byte-bounded, fingerprint-validated topology cache.
#[derive(Default)]
pub(crate) struct TopoCache {
    inner: Mutex<Inner>,
}

impl TopoCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-global instance used by the live query path.
    pub fn global() -> &'static TopoCache {
        static C: OnceLock<TopoCache> = OnceLock::new();
        C.get_or_init(TopoCache::new)
    }

    /// Return the cached snapshot for `key` **only if** its fingerprint matches
    /// `fingerprint` (else the graph changed → miss). Promotes to MRU on a hit.
    pub fn get(&self, key: &TopoKey, fingerprint: (usize, usize)) -> Option<Arc<TopoSnapshot>> {
        let mut g = self.inner.lock().ok()?;
        let snap = g.map.get(key)?.clone();
        if snap.fingerprint != fingerprint {
            // Stale: drop it so the caller rebuilds and re-inserts.
            g.map.remove(key);
            g.lru.retain(|k| k != key);
            return None;
        }
        g.lru.retain(|k| k != key);
        g.lru.push(key.clone());
        Some(snap)
    }

    /// Insert (or replace) the snapshot for `key`, evicting the LRU entry when
    /// over the cap.
    pub fn put(&self, key: TopoKey, snap: Arc<TopoSnapshot>) {
        let Ok(mut g) = self.inner.lock() else {
            return;
        };
        g.map.insert(key.clone(), snap);
        g.lru.retain(|k| k != &key);
        g.lru.push(key);
        while g.lru.len() > MAX_ENTRIES {
            let victim = g.lru.remove(0);
            g.map.remove(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(fp: (usize, usize)) -> Arc<TopoSnapshot> {
        Arc::new(TopoSnapshot {
            fingerprint: fp,
            csr: CsrGraph::build(["a", "b"], [("a", "b", "E")]),
            edges: Vec::new(),
        })
    }
    fn key(t: &str) -> TopoKey {
        TopoKey {
            database: "db".into(),
            node_table: format!("{t}_n"),
            edge_table: format!("{t}_e"),
        }
    }

    #[test]
    fn hit_only_on_matching_fingerprint() {
        let c = TopoCache::new();
        let k = key("g");
        c.put(k.clone(), snap((10, 20)));
        assert!(c.get(&k, (10, 20)).is_some(), "matching fingerprint hits");
        // A changed graph (different counts) misses and evicts the stale entry.
        assert!(c.get(&k, (10, 21)).is_none(), "changed fingerprint misses");
        assert!(
            c.get(&k, (10, 20)).is_none(),
            "stale entry was dropped on miss"
        );
    }

    #[test]
    fn lru_evicts_oldest_over_cap() {
        let c = TopoCache::new();
        for i in 0..MAX_ENTRIES {
            c.put(key(&format!("g{i}")), snap((i, i)));
        }
        // Touch g0 so it is MRU, then overflow by one.
        assert!(c.get(&key("g0"), (0, 0)).is_some());
        c.put(key("gX"), snap((99, 99)));
        // g1 (now LRU) evicted; g0 (touched) retained.
        assert!(c.get(&key("g0"), (0, 0)).is_some(), "MRU survived");
        assert!(c.get(&key("g1"), (1, 1)).is_none(), "LRU evicted");
        assert!(c.get(&key("gX"), (99, 99)).is_some(), "new entry present");
    }
}
