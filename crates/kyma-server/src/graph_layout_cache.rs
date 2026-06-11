//! Cache of laid-out full graphs keyed by (database, graph, realm, algorithm).
//! Entries are fingerprinted by (total_nodes, total_relationships) — a cheap
//! version proxy. Pagination slices a Ready entry; cursors embed the
//! layout_id so pages stay consistent even if the graph mutates mid-paging.

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

#[derive(Default)]
pub struct LayoutCache {
    /// Primary store + LRU order (front = oldest).
    inner: Mutex<(HashMap<CacheKey, CacheState>, Vec<CacheKey>, HashMap<String, Arc<LaidOutGraph>>)>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deterministic id for a (key, fingerprint) pair.
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
    pub fn get_fresh(&self, key: &CacheKey, fingerprint: (usize, usize)) -> Option<CacheState> {
        let mut g = self.inner.lock().unwrap();
        let (map, lru, by_id) = &mut *g;
        match map.get(key) {
            Some(CacheState::Computing) => Some(CacheState::Computing),
            Some(CacheState::Ready(l)) if l.fingerprint == fingerprint => {
                // touch LRU
                lru.retain(|k| k != key);
                lru.push(key.clone());
                Some(CacheState::Ready(l.clone()))
            }
            Some(CacheState::Ready(l)) => {
                by_id.remove(&l.layout_id);
                map.remove(key);
                lru.retain(|k| k != key);
                None
            }
            None => None,
        }
    }

    /// Mark `key` as computing (idempotent). Returns false if it was already
    /// computing (so only one task computes).
    pub fn begin_compute(&self, key: &CacheKey) -> bool {
        let mut g = self.inner.lock().unwrap();
        let (map, _, _) = &mut *g;
        if matches!(map.get(key), Some(CacheState::Computing)) {
            return false;
        }
        map.insert(key.clone(), CacheState::Computing);
        true
    }

    pub fn insert_ready(&self, key: CacheKey, laid: LaidOutGraph) -> Arc<LaidOutGraph> {
        let laid = Arc::new(laid);
        let mut g = self.inner.lock().unwrap();
        let (map, lru, by_id) = &mut *g;
        by_id.insert(laid.layout_id.clone(), laid.clone());
        map.insert(key.clone(), CacheState::Ready(laid.clone()));
        lru.retain(|k| k != &key);
        lru.push(key);
        while lru.len() > MAX_ENTRIES {
            let evict = lru.remove(0);
            if let Some(CacheState::Ready(l)) = map.remove(&evict) {
                by_id.remove(&l.layout_id);
            }
        }
        laid
    }

    /// Drop a Computing marker after a failed compute so the next request retries.
    pub fn abort_compute(&self, key: &CacheKey) {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.0.get(key), Some(CacheState::Computing)) {
            g.0.remove(key);
        }
    }

    /// Resolve a paging cursor's layout_id to its graph (for pages 2+).
    pub fn by_layout_id(&self, layout_id: &str) -> Option<Arc<LaidOutGraph>> {
        self.inner.lock().unwrap().2.get(layout_id).cloned()
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
    match kind {
        'n' => {
            let end = (offset + page_size).min(g.nodes.len());
            page.nodes = g.nodes[offset.min(g.nodes.len())..end].to_vec();
            page.next_cursor = if end < g.nodes.len() {
                Some(format!("{}:n:{}", g.layout_id, end))
            } else if !g.edges.is_empty() {
                Some(format!("{}:e:0", g.layout_id))
            } else {
                None
            };
        }
        _ => {
            let end = (offset + page_size).min(g.edges.len());
            page.edges = g.edges[offset.min(g.edges.len())..end].to_vec();
            page.next_cursor =
                (end < g.edges.len()).then(|| format!("{}:e:{}", g.layout_id, end));
        }
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
}
