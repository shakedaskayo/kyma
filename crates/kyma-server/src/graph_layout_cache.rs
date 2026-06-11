//! Cache of laid-out full graphs keyed by (database, graph, realm, algorithm).
//! Entries are fingerprinted by (`total_nodes`, `total_relationships`) — a cheap
//! version proxy. Pagination slices a Ready entry; cursors embed the
//! `layout_id` so pages stay consistent even if the graph mutates mid-paging.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kyma_graph::{GraphExportPage, GraphRelationship, PositionedNode};

pub const PAGE_SIZE_DEFAULT: usize = 10_000;
/// Above this node count, layout is computed in a background task and the
/// endpoint reports `layout_status: "computing"` until it lands.
pub const SYNC_COMPUTE_MAX_NODES: usize = 20_000;
const MAX_ENTRIES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub database: String,
    pub graph: String,
    pub realm: Option<String>,
    pub algorithm: kyma_graph::LayoutAlgorithm,
}

#[derive(Debug)]
pub struct LaidOutGraph {
    pub layout_id: String,
    pub fingerprint: (usize, usize),
    pub nodes: Vec<PositionedNode>,
    pub edges: Vec<GraphRelationship>,
}

#[derive(Debug, Clone)]
pub enum CacheState {
    Computing,
    Ready(Arc<LaidOutGraph>),
}

/// Inner state factored out to avoid the `type_complexity` lint on the Mutex tuple.
#[derive(Default)]
struct CacheInner {
    map: HashMap<CacheKey, CacheState>,
    /// LRU order: front = oldest, back = most recently used.
    lru: Vec<CacheKey>,
    by_id: HashMap<String, Arc<LaidOutGraph>>,
}

#[derive(Default)]
pub struct LayoutCache {
    inner: Mutex<CacheInner>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deterministic id for a (key, fingerprint) pair.
    ///
    /// `DefaultHasher` is intentionally used here: `layout_id`s are in-memory
    /// only and clients restart on HTTP 410, so cross-process hash stability
    /// is not required.
    pub fn layout_id(key: &CacheKey, fingerprint: (usize, usize)) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        fingerprint.hash(&mut h);
        format!("L{:016x}", h.finish())
    }

    /// Current state for `key` IF its fingerprint still matches; stale Ready
    /// entries are evicted and `None` is returned (caller recomputes).
    ///
    /// `Computing` entries are returned regardless of fingerprint — a stale
    /// in-flight compute results in at most one spurious poll cycle; the stale
    /// result is evicted on the next first-page request when the new fingerprint
    /// is presented.
    pub fn get_fresh(&self, key: &CacheKey, fingerprint: (usize, usize)) -> Option<CacheState> {
        let mut g = self.inner.lock().unwrap();
        let inner = &mut *g;
        match inner.map.get(key) {
            Some(CacheState::Computing) => Some(CacheState::Computing),
            Some(CacheState::Ready(l)) if l.fingerprint == fingerprint => {
                // touch LRU
                inner.lru.retain(|k| k != key);
                inner.lru.push(key.clone());
                Some(CacheState::Ready(l.clone()))
            }
            Some(CacheState::Ready(l)) => {
                inner.by_id.remove(&l.layout_id);
                inner.map.remove(key);
                inner.lru.retain(|k| k != key);
                None
            }
            None => None,
        }
    }

    /// Mark `key` as computing (idempotent). Returns false if it was already
    /// computing (so only one task computes).
    pub fn begin_compute(&self, key: &CacheKey) -> bool {
        let mut g = self.inner.lock().unwrap();
        let inner = &mut *g;
        if matches!(inner.map.get(key), Some(CacheState::Computing)) {
            return false;
        }
        inner.map.insert(key.clone(), CacheState::Computing);
        true
    }

    pub fn insert_ready(&self, key: CacheKey, laid: LaidOutGraph) -> Arc<LaidOutGraph> {
        let laid = Arc::new(laid);
        let mut g = self.inner.lock().unwrap();
        let inner = &mut *g;
        inner.by_id.insert(laid.layout_id.clone(), laid.clone());
        inner.map.insert(key.clone(), CacheState::Ready(laid.clone()));
        inner.lru.retain(|k| k != &key);
        inner.lru.push(key);
        while inner.lru.len() > MAX_ENTRIES {
            let evict = inner.lru.remove(0);
            if let Some(CacheState::Ready(l)) = inner.map.remove(&evict) {
                inner.by_id.remove(&l.layout_id);
            }
        }
        laid
    }

    /// Drop a Computing marker after a failed compute so the next request retries.
    pub fn abort_compute(&self, key: &CacheKey) {
        let mut g = self.inner.lock().unwrap();
        let inner = &mut *g;
        if matches!(inner.map.get(key), Some(CacheState::Computing)) {
            inner.map.remove(key);
        }
    }

    /// Resolve a paging cursor's `layout_id` to its graph (for pages 2+).
    pub fn by_layout_id(&self, layout_id: &str) -> Option<Arc<LaidOutGraph>> {
        self.inner.lock().unwrap().by_id.get(layout_id).cloned()
    }

    /// Returns an RAII guard that calls `abort_compute` on drop unless
    /// `disarm()` is called first. Use this in background compute tasks so
    /// a panic or early-return never leaves an orphaned `Computing` marker.
    pub fn compute_guard(self: &Arc<Self>, key: CacheKey) -> ComputeGuard {
        ComputeGuard { cache: Arc::clone(self), key: Some(key) }
    }
}

/// RAII guard for a `Computing` marker.  Drop aborts; call `disarm()` after a
/// successful `insert_ready` to prevent that.
pub struct ComputeGuard {
    cache: Arc<LayoutCache>,
    key: Option<CacheKey>,
}

impl ComputeGuard {
    /// Prevent the guard from calling `abort_compute` when it drops.
    pub fn disarm(&mut self) {
        self.key = None;
    }
}

impl Drop for ComputeGuard {
    fn drop(&mut self) {
        if let Some(ref key) = self.key {
            self.cache.abort_compute(key);
        }
    }
}

/// Cursor grammar: `"{layout_id}:n:{offset}"` (node pages) then
/// `"{layout_id}:e:{offset}"` (edge pages). Returns None on garbage.
pub fn parse_cursor(cursor: &str) -> Option<(String, char, usize)> {
    let mut parts = cursor.rsplitn(3, ':');
    let offset: usize = parts.next()?.parse().ok()?;
    let kind = parts.next()?.chars().next().filter(|c| *c == 'n' || *c == 'e')?;
    let id = parts.next()?.to_string();
    if id.is_empty() {
        return None;
    }
    Some((id, kind, offset))
}

/// Slice one page out of a laid-out graph. Node pages first, then edge pages.
pub fn slice_page(g: &LaidOutGraph, kind: char, offset: usize, page_size: usize) -> GraphExportPage {
    let mut page = GraphExportPage {
        layout_status: "ready".into(),
        layout_id: g.layout_id.clone(),
        total_nodes: g.nodes.len(),
        total_edges: g.edges.len(),
        nodes: Vec::new(),
        edges: Vec::new(),
        next_cursor: None,
    };
    if kind == 'n' {
        let start = offset.min(g.nodes.len());
        let end = offset.saturating_add(page_size).min(g.nodes.len()).max(start);
        page.nodes = g.nodes[start..end].to_vec();
        page.next_cursor = if end < g.nodes.len() {
            Some(format!("{}:n:{}", g.layout_id, end))
        } else if !g.edges.is_empty() {
            Some(format!("{}:e:0", g.layout_id))
        } else {
            None
        };
    } else {
        let start = offset.min(g.edges.len());
        let end = offset.saturating_add(page_size).min(g.edges.len()).max(start);
        page.edges = g.edges[start..end].to_vec();
        page.next_cursor =
            (end < g.edges.len()).then(|| format!("{}:e:{}", g.layout_id, end));
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use kyma_graph::{GraphNode, NodeMetadata};

    fn laid(n: usize, e: usize) -> LaidOutGraph {
        let nodes = (0..n)
            .map(|i| PositionedNode {
                node: GraphNode {
                    id: format!("n{i}"),
                    labels: vec!["Table".into()],
                    properties: Default::default(),
                    metadata: NodeMetadata {
                        created_at: String::new(),
                        updated_at: String::new(),
                        source_type: None,
                        source_id: None,
                        realm: "default".into(),
                    },
                },
                x: i as f64,
                y: 0.0,
            })
            .collect();
        let edges = (0..e)
            .map(|i| GraphRelationship {
                id: format!("e{i}"),
                source_id: format!("n{}", i % n.max(1)),
                target_id: format!("n{}", (i + 1) % n.max(1)),
                relationship_type: "REFERENCES".into(),
                properties: Default::default(),
            })
            .collect();
        LaidOutGraph { layout_id: "Labc".into(), fingerprint: (n, e), nodes, edges }
    }

    #[test]
    fn pages_walk_nodes_then_edges_to_completion() {
        let g = laid(25, 7);
        let p1 = slice_page(&g, 'n', 0, 10);
        assert_eq!(p1.nodes.len(), 10);
        assert_eq!(p1.next_cursor.as_deref(), Some("Labc:n:10"));
        let p3 = slice_page(&g, 'n', 20, 10);
        assert_eq!(p3.nodes.len(), 5);
        assert_eq!(p3.next_cursor.as_deref(), Some("Labc:e:0"));
        let p4 = slice_page(&g, 'e', 0, 10);
        assert_eq!(p4.edges.len(), 7);
        assert_eq!(p4.next_cursor, None);
        assert_eq!(p4.total_nodes, 25);
        assert_eq!(p4.total_edges, 7);
    }

    #[test]
    fn cursor_roundtrip_and_garbage() {
        assert_eq!(parse_cursor("Labc:n:10"), Some(("Labc".into(), 'n', 10)));
        assert_eq!(parse_cursor("Labc:e:0"), Some(("Labc".into(), 'e', 0)));
        assert_eq!(parse_cursor("nonsense"), None);
        assert_eq!(parse_cursor("x:q:5"), None);
        assert_eq!(parse_cursor(":n:5"), None);
    }

    #[test]
    fn stale_fingerprint_evicts() {
        let cache = LayoutCache::new();
        let key = CacheKey {
            database: "db".into(),
            graph: "g".into(),
            realm: None,
            algorithm: kyma_graph::LayoutAlgorithm::Force,
        };
        cache.insert_ready(key.clone(), laid(5, 2));
        assert!(matches!(cache.get_fresh(&key, (5, 2)), Some(CacheState::Ready(_))));
        assert!(cache.get_fresh(&key, (6, 2)).is_none(), "stale entry must evict");
        assert!(cache.by_layout_id("Labc").is_none(), "by_id index must evict too");
    }

    #[test]
    fn begin_compute_is_exclusive() {
        let cache = LayoutCache::new();
        let key = CacheKey {
            database: "db".into(),
            graph: "g".into(),
            realm: None,
            algorithm: kyma_graph::LayoutAlgorithm::Force,
        };
        assert!(cache.begin_compute(&key));
        assert!(!cache.begin_compute(&key));
        cache.abort_compute(&key);
        assert!(cache.begin_compute(&key));
    }

    /// Regression: huge offsets must not overflow or panic.
    #[test]
    fn slice_page_survives_huge_offset() {
        let g = laid(5, 2);

        // Node kind with huge offset — should return empty nodes and chain to edges
        let p = slice_page(&g, 'n', usize::MAX - 5, 10_000);
        assert!(p.nodes.is_empty(), "nodes must be empty for huge offset");
        // edges exist, so cursor should chain to edge page
        assert_eq!(
            p.next_cursor.as_deref(),
            Some("Labc:e:0"),
            "should chain to edge page since edges exist"
        );

        // Edge kind with huge offset — should return empty edges and no next cursor
        let p2 = slice_page(&g, 'e', usize::MAX - 5, 10_000);
        assert!(p2.edges.is_empty(), "edges must be empty for huge offset");
        assert_eq!(p2.next_cursor, None, "no next cursor past last edge page");
    }

    /// RAII guard aborts a Computing marker on drop; disarmed guard leaves
    /// the Ready entry intact.
    #[test]
    fn compute_guard_aborts_on_drop() {
        let cache = Arc::new(LayoutCache::new());
        let key = CacheKey {
            database: "db".into(),
            graph: "g".into(),
            realm: None,
            algorithm: kyma_graph::LayoutAlgorithm::Force,
        };

        // 1. begin_compute via the guard path, then drop without disarm.
        {
            assert!(cache.begin_compute(&key));
            let _guard = cache.compute_guard(key.clone());
            // guard dropped here — should call abort_compute
        }
        // After abort, begin_compute must succeed again.
        assert!(cache.begin_compute(&key), "Computing marker must have been removed by guard drop");
        cache.abort_compute(&key);

        // 2. Disarmed guard must NOT abort — insert_ready first, then disarm.
        assert!(cache.begin_compute(&key));
        let mut guard = cache.compute_guard(key.clone());
        cache.insert_ready(key.clone(), laid(3, 1));
        guard.disarm();
        drop(guard);
        // Entry should still be Ready.
        assert!(
            matches!(cache.get_fresh(&key, (3, 1)), Some(CacheState::Ready(_))),
            "disarmed guard must not evict the Ready entry"
        );
    }
}
