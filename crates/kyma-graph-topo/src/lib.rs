//! Immutable CSR graph topology + multi-source BFS traversal (S3.1).
//!
//! The substrate for graph queries beyond the current 2-hop ceiling. A
//! [`CsrGraph`] holds a directed multigraph in compressed-sparse-row form — a
//! dense node-id dictionary plus flat `offsets`/`targets` arrays — with a CSC
//! twin (in-edges) so backward and undirected traversal are equally cheap.
//! Traversal is bare integer BFS over those arrays: no per-hop SQL self-join,
//! no recursion-depth wall, cycle-safe by construction.
//!
//! # Semantics (must match the reference oracle)
//!
//! These mirror `kyma_graph_testkit::oracle` exactly — that petgraph engine is
//! the normative spec, and [`CsrGraph`] is differentially validated against it:
//! - **Directed multigraph**: parallel edges are all kept and traversable.
//! - **Dangling edges dropped at build**: an edge whose `src` or `dst` id is
//!   absent from the node set never participates (mirrors the engine inner-join
//!   on both endpoints).
//! - **`etype` filter is equality**; `None` traverses every edge. A filter
//!   naming a type not present in the graph matches nothing.
//! - **Results are sets** of `(source_id, node_id, distance)` tuples — order
//!   never matters. Each reached node is reported once per source, at its
//!   shortest hop count (node-distinct BFS). The source itself appears as
//!   `(s, s, 0)` only when `s` is a known node.
//!
//! # On-disk form
//!
//! [`CsrGraph::to_bytes`] / [`CsrGraph::from_bytes`] is a self-describing,
//! checksummed single-file layout (magic `KYCS`) so a built topology can live
//! as an object-store sidecar keyed by graph snapshot (the activation path is a
//! later increment). The layout is little-endian and version-tagged; a bad
//! magic, unknown version, or checksum mismatch is a hard error rather than a
//! silent misread.

#![forbid(unsafe_code)]
// The CSR is u32-indexed by design: node and edge ordinals, offsets, and the
// dictionary positions are all `u32` (a documented ~4.3B-element ceiling that
// keeps the arrays half the size of `usize` and the on-disk form compact).
// Casting these `usize` lengths/positions to `u32` is therefore intentional and
// in-range, not a truncation hazard.
#![allow(clippy::cast_possible_truncation)]
// PPR arithmetic divides residual mass by node degree / seed count as `f64`.
// Graph sizes are far below `f64`'s 2^52 exact-integer range, so these
// `usize as f64` casts lose no precision in practice.
#![allow(clippy::cast_precision_loss)]

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

/// Traversal direction. Mirrors `kyma_graph_testkit::oracle::Direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow edges `src → dst`.
    Fwd,
    /// Follow edges `dst → src` (reverse traversal).
    Back,
    /// Treat every edge as bidirectional (shortest hop count in the underlying
    /// undirected multigraph, after the `etype` filter).
    Both,
}

/// A resolved edge-type traversal predicate (the dictionary lookup is done
/// once per query, not per edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EtypeFilter {
    /// No filter — every edge traverses.
    Any,
    /// The filter named a type absent from this graph — nothing traverses.
    Unknown,
    /// Traverse only edges of this dictionary id.
    Only(u32),
}

impl EtypeFilter {
    #[inline]
    fn matches(self, etype_id: u32) -> bool {
        match self {
            EtypeFilter::Any => true,
            EtypeFilter::Unknown => false,
            EtypeFilter::Only(id) => etype_id == id,
        }
    }
}

/// Failure decoding a serialized [`CsrGraph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopoError {
    /// Wrong/absent magic, or a length field overran the buffer.
    Corrupt(String),
    /// Serialized by an incompatible future format version.
    UnsupportedVersion(u16),
    /// Stored checksum did not match the recomputed one.
    ChecksumMismatch,
}

impl fmt::Display for TopoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopoError::Corrupt(m) => write!(f, "corrupt CSR topology: {m}"),
            TopoError::UnsupportedVersion(v) => write!(f, "unsupported CSR topology version {v}"),
            TopoError::ChecksumMismatch => write!(f, "CSR topology checksum mismatch"),
        }
    }
}

impl std::error::Error for TopoError {}

const MAGIC: &[u8; 4] = b"KYCS";
const VERSION: u16 = 1;

/// An immutable directed multigraph in CSR (+ CSC) form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrGraph {
    /// Dense index → node id.
    ids: Vec<String>,
    /// Node id → dense index.
    id_to_idx: HashMap<String, u32>,
    /// Dense index → edge-type string.
    etypes: Vec<String>,

    /// Out-CSR: `out_targets[out_offsets[v]..out_offsets[v+1]]` are v's out-neighbors.
    out_offsets: Vec<u32>,
    out_targets: Vec<u32>,
    /// Edge-type id parallel to `out_targets`.
    out_etype: Vec<u32>,

    /// In-CSR (CSC twin): sources of edges pointing at v.
    in_offsets: Vec<u32>,
    in_sources: Vec<u32>,
    /// Edge-type id parallel to `in_sources`.
    in_etype: Vec<u32>,
}

impl CsrGraph {
    /// Build from a node-id set and a `(src, dst, etype)` edge stream.
    ///
    /// Node ids are de-duplicated (first occurrence wins); edges with an
    /// endpoint absent from the node set are dropped. Parallel edges are kept.
    pub fn build<'a, N, E>(nodes: N, edges: E) -> Self
    where
        N: IntoIterator<Item = &'a str>,
        E: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    {
        let mut ids: Vec<String> = Vec::new();
        let mut id_to_idx: HashMap<String, u32> = HashMap::new();
        for n in nodes {
            if let Entry::Vacant(e) = id_to_idx.entry(n.to_string()) {
                e.insert(ids.len() as u32);
                ids.push(n.to_string());
            }
        }

        let mut etypes: Vec<String> = Vec::new();
        let mut etype_to_id: HashMap<String, u32> = HashMap::new();
        // Collect kept edges as (src_idx, dst_idx, etype_id), dropping dangling.
        let mut kept: Vec<(u32, u32, u32)> = Vec::new();
        for (s, d, t) in edges {
            let (Some(&si), Some(&di)) = (id_to_idx.get(s), id_to_idx.get(d)) else {
                continue;
            };
            let tid = match etype_to_id.entry(t.to_string()) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    let id = etypes.len() as u32;
                    etypes.push(t.to_string());
                    *e.insert(id)
                }
            };
            kept.push((si, di, tid));
        }

        let n = ids.len();
        let (out_offsets, out_targets, out_etype) =
            build_csr(n, kept.iter().map(|&(s, d, t)| (s, d, t)));
        // CSC twin: index by destination, store the source.
        let (in_offsets, in_sources, in_etype) =
            build_csr(n, kept.iter().map(|&(s, d, t)| (d, s, t)));

        Self {
            ids,
            id_to_idx,
            etypes,
            out_offsets,
            out_targets,
            out_etype,
            in_offsets,
            in_sources,
            in_etype,
        }
    }

    /// Number of (distinct) nodes.
    pub fn num_nodes(&self) -> usize {
        self.ids.len()
    }

    /// Number of retained (non-dangling) directed edges.
    pub fn num_edges(&self) -> usize {
        self.out_targets.len()
    }

    /// Whether `id` is a known node.
    pub fn contains(&self, id: &str) -> bool {
        self.id_to_idx.contains_key(id)
    }

    /// Resolve an `etype` filter to a traversal predicate over edge-type ids.
    fn resolve_etype(&self, etype: Option<&str>) -> EtypeFilter {
        match etype {
            None => EtypeFilter::Any,
            Some(t) => match self.etypes.iter().position(|e| e == t) {
                Some(p) => EtypeFilter::Only(p as u32),
                None => EtypeFilter::Unknown,
            },
        }
    }

    /// Push the dense-index neighbors of `v` under `dir`, honoring the resolved
    /// `etype` predicate, into `out`.
    fn step(&self, v: u32, dir: Direction, etype: EtypeFilter, out: &mut Vec<u32>) {
        let vi = v as usize;
        if matches!(dir, Direction::Fwd | Direction::Both) {
            let (start, end) = (
                self.out_offsets[vi] as usize,
                self.out_offsets[vi + 1] as usize,
            );
            for i in start..end {
                if etype.matches(self.out_etype[i]) {
                    out.push(self.out_targets[i]);
                }
            }
        }
        if matches!(dir, Direction::Back | Direction::Both) {
            let (start, end) = (
                self.in_offsets[vi] as usize,
                self.in_offsets[vi + 1] as usize,
            );
            for i in start..end {
                if etype.matches(self.in_etype[i]) {
                    out.push(self.in_sources[i]);
                }
            }
        }
    }

    /// k-hop expansion from each source: the set of `(source, node, distance)`
    /// reachable in at most `k` hops, each node reported once at its shortest
    /// distance. `(s, s, 0)` is included for every known source.
    pub fn k_hop(
        &self,
        sources: &[&str],
        k: usize,
        dir: Direction,
        etype: Option<&str>,
    ) -> BTreeSet<(String, String, usize)> {
        let pred = self.resolve_etype(etype);
        let mut out = BTreeSet::new();
        let mut depth_of: HashMap<u32, usize> = HashMap::new();
        let mut nbrs: Vec<u32> = Vec::new();
        for &s in sources {
            let Some(&start) = self.id_to_idx.get(s) else {
                continue; // unknown source contributes nothing
            };
            depth_of.clear();
            depth_of.insert(start, 0);
            out.insert((s.to_string(), s.to_string(), 0));
            let mut q = VecDeque::from([start]);
            while let Some(cur) = q.pop_front() {
                let d = depth_of[&cur];
                if d == k {
                    continue;
                }
                nbrs.clear();
                self.step(cur, dir, pred, &mut nbrs);
                for &nb in &nbrs {
                    if let Entry::Vacant(slot) = depth_of.entry(nb) {
                        slot.insert(d + 1);
                        out.insert((s.to_string(), self.ids[nb as usize].clone(), d + 1));
                        q.push_back(nb);
                    }
                }
            }
        }
        out
    }

    /// Var-length `[min..max]` expansion: [`Self::k_hop`] at `max`, keeping only
    /// nodes at distance `>= min`.
    pub fn var_length(
        &self,
        sources: &[&str],
        min: usize,
        max: usize,
        dir: Direction,
        etype: Option<&str>,
    ) -> BTreeSet<(String, String, usize)> {
        self.k_hop(sources, max, dir, etype)
            .into_iter()
            .filter(|(_, _, d)| *d >= min)
            .collect()
    }

    /// Unweighted shortest-path hop count `src → dst` under `dir`/`etype`, or
    /// `None` if unreachable (or either id is unknown). `src == dst` is `0`.
    pub fn shortest_path_len(
        &self,
        src: &str,
        dst: &str,
        dir: Direction,
        etype: Option<&str>,
    ) -> Option<usize> {
        let (&start, &goal) = (self.id_to_idx.get(src)?, self.id_to_idx.get(dst)?);
        if start == goal {
            return Some(0);
        }
        let pred = self.resolve_etype(etype);
        let mut depth_of: HashMap<u32, usize> = HashMap::new();
        depth_of.insert(start, 0);
        let mut q = VecDeque::from([start]);
        let mut nbrs: Vec<u32> = Vec::new();
        while let Some(cur) = q.pop_front() {
            let d = depth_of[&cur];
            nbrs.clear();
            self.step(cur, dir, pred, &mut nbrs);
            for &nb in &nbrs {
                if nb == goal {
                    return Some(d + 1);
                }
                if let Entry::Vacant(slot) = depth_of.entry(nb) {
                    slot.insert(d + 1);
                    q.push_back(nb);
                }
            }
        }
        None
    }

    /// Connected components by BFS labeling under `dir` (use `Both` for
    /// weakly-connected components — the usual notion of "is this graph in one
    /// piece"). Returns `(node_id, component_id)` with dense component ids
    /// assigned in node-index discovery order; a node with no edges is its own
    /// singleton component. Cheap connectivity for graph stats, the explorer,
    /// and as the partition for per-component PPR / community runs.
    pub fn connected_components(&self, dir: Direction) -> Vec<(String, u32)> {
        let n = self.ids.len();
        let mut comp = vec![u32::MAX; n];
        let mut next = 0u32;
        let mut nbrs: Vec<u32> = Vec::new();
        let mut queue: VecDeque<u32> = VecDeque::new();
        for start in 0..n {
            if comp[start] != u32::MAX {
                continue;
            }
            comp[start] = next;
            queue.clear();
            queue.push_back(start as u32);
            while let Some(u) = queue.pop_front() {
                nbrs.clear();
                self.step(u, dir, EtypeFilter::Any, &mut nbrs);
                for &v in &nbrs {
                    if comp[v as usize] == u32::MAX {
                        comp[v as usize] = next;
                        queue.push_back(v);
                    }
                }
            }
            next += 1;
        }
        comp.iter()
            .enumerate()
            .map(|(i, &c)| (self.ids[i].clone(), c))
            .collect()
    }

    /// Approximate personalized `PageRank` from `seeds` via the
    /// Andersen–Chung–Lang forward-push algorithm (S3.4): the basis for graph
    /// proximity in memory retrieval, replacing the fixed hop loop.
    ///
    /// Returns `(node_id, mass)` for every node with nonzero PPR mass, highest
    /// first. `alpha` is the teleport/restart probability (≈0.15); `epsilon`
    /// bounds the approximation — smaller pushes the result toward the exact PPR
    /// vector (residual-degree threshold `r[u] < epsilon·deg(u)` stops a node).
    /// Edge multiplicity is a weight (parallel edges raise transition mass);
    /// `dir` selects the walk's adjacency (`Both` = undirected proximity). A
    /// sink (no out-neighbors under `dir`) absorbs its residual as terminal
    /// mass. Unknown seed ids are ignored; all-unknown ⇒ empty.
    pub fn personalized_pagerank(
        &self,
        seeds: &[&str],
        alpha: f64,
        epsilon: f64,
        dir: Direction,
    ) -> Vec<(String, f64)> {
        let n = self.ids.len();
        let seed_idx: Vec<u32> = seeds
            .iter()
            .filter_map(|s| self.id_to_idx.get(*s).copied())
            .collect();
        if seed_idx.is_empty() || n == 0 {
            return Vec::new();
        }
        let alpha = alpha.clamp(f64::MIN_POSITIVE, 1.0);
        let eps = epsilon.max(f64::MIN_POSITIVE);

        let mut p = vec![0.0f64; n];
        let mut r = vec![0.0f64; n];
        let share = 1.0 / seed_idx.len() as f64;
        for &si in &seed_idx {
            r[si as usize] += share;
        }
        // Process nodes whose residual may exceed the degree-scaled threshold.
        // `queued` prevents duplicate enqueues; a node re-enters when a push
        // grows its residual.
        let mut queued = vec![false; n];
        let mut queue: VecDeque<u32> = VecDeque::new();
        for &si in &seed_idx {
            if !queued[si as usize] {
                queued[si as usize] = true;
                queue.push_back(si);
            }
        }
        let mut nbrs: Vec<u32> = Vec::new();
        while let Some(u) = queue.pop_front() {
            queued[u as usize] = false;
            nbrs.clear();
            self.step(u, dir, EtypeFilter::Any, &mut nbrs);
            let deg = nbrs.len();
            let ru = r[u as usize];
            if deg == 0 {
                // Sink: the walk terminates here, so all residual becomes mass.
                p[u as usize] += ru;
                r[u as usize] = 0.0;
                continue;
            }
            if ru < eps * deg as f64 {
                continue; // below threshold — leave the residual
            }
            p[u as usize] += alpha * ru;
            let push = (1.0 - alpha) * ru / deg as f64;
            r[u as usize] = 0.0;
            for &v in &nbrs {
                r[v as usize] += push;
                if !queued[v as usize] {
                    queued[v as usize] = true;
                    queue.push_back(v);
                }
            }
        }

        let mut out: Vec<(String, f64)> = p
            .iter()
            .enumerate()
            .filter(|(_, &m)| m > 0.0)
            .map(|(i, &m)| (self.ids[i].clone(), m))
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Weighted degree (edge multiplicity counts) of every node under `dir`,
    /// plus `m = Σ deg / 2` (the undirected total edge weight). Shared by
    /// community detection and modularity.
    fn degrees(&self, dir: Direction) -> (Vec<f64>, f64) {
        let n = self.ids.len();
        let mut deg = vec![0.0f64; n];
        let mut nbrs: Vec<u32> = Vec::new();
        let mut total = 0.0;
        for (u, d) in deg.iter_mut().enumerate() {
            nbrs.clear();
            self.step(u as u32, dir, EtypeFilter::Any, &mut nbrs);
            *d = nbrs.len() as f64;
            total += *d;
        }
        (deg, total / 2.0)
    }

    /// Newman–Girvan modularity `Q` of a `community`-id labeling under `dir`:
    /// `Σ_C [ in_C/(2m) − (tot_C/2m)² ]`. Higher = stronger community structure
    /// (range roughly `[-0.5, 1)`). The independent yardstick for community
    /// detection.
    pub fn modularity(&self, community: &[u32], dir: Direction) -> f64 {
        let n = self.ids.len();
        if community.len() != n {
            return 0.0;
        }
        let (deg, m) = self.degrees(dir);
        if m == 0.0 {
            return 0.0;
        }
        let mut in_c: HashMap<u32, f64> = HashMap::new();
        let mut tot_c: HashMap<u32, f64> = HashMap::new();
        let mut nbrs: Vec<u32> = Vec::new();
        for u in 0..n {
            let cu = community[u];
            *tot_c.entry(cu).or_insert(0.0) += deg[u];
            nbrs.clear();
            self.step(u as u32, dir, EtypeFilter::Any, &mut nbrs);
            for &v in &nbrs {
                if community[v as usize] == cu {
                    *in_c.entry(cu).or_insert(0.0) += 1.0;
                }
            }
        }
        let two_m = 2.0 * m;
        tot_c
            .iter()
            .map(|(c, &tot)| {
                let inside = in_c.get(c).copied().unwrap_or(0.0);
                inside / two_m - (tot / two_m).powi(2)
            })
            .sum()
    }

    /// Detect communities by **single-level Louvain local moving** (S3.4):
    /// greedily move each node to the neighboring community that most increases
    /// modularity, swept in index order (ties → smallest community id) until a
    /// pass makes no move or `max_passes`. Modularity-based, so — unlike label
    /// propagation — a single bridge does not collapse two dense clusters; the
    /// hierarchical-Leiden aggregation/refinement layers on top later.
    /// Deterministic. Returns `(node_id, community_id)` with dense ids.
    pub fn detect_communities(&self, dir: Direction, max_passes: usize) -> Vec<(String, u32)> {
        let n = self.ids.len();
        let (deg, m) = self.degrees(dir);
        let mut comm: Vec<u32> = (0..n as u32).collect();
        if m == 0.0 {
            // No edges → every node is its own community.
            return self.dense_communities(&comm);
        }
        let two_m = 2.0 * m;
        let mut sigma_tot = deg.clone(); // Σ degrees per community (each node alone)
        let mut nbrs: Vec<u32> = Vec::new();
        let mut k_in: HashMap<u32, f64> = HashMap::new();

        for _ in 0..max_passes {
            let mut moved = false;
            for u in 0..n {
                if deg[u] == 0.0 {
                    continue;
                }
                nbrs.clear();
                self.step(u as u32, dir, EtypeFilter::Any, &mut nbrs);
                // Edge multiplicity from u into each neighboring community.
                k_in.clear();
                for &v in &nbrs {
                    *k_in.entry(comm[v as usize]).or_insert(0.0) += 1.0;
                }
                let cu = comm[u];
                // Remove u from its community before scoring candidates.
                sigma_tot[cu as usize] -= deg[u];
                // Gain of placing u in C ∝ k_in(C) − deg[u]·Σ_tot(C)/(2m).
                // Staying isolated scores 0; pick the best neighbor community.
                // Candidates are scored in ascending community-id order with a
                // strict `>`, so the smallest id wins ties — deterministic
                // regardless of the hash-map's iteration order.
                let mut cands: Vec<u32> = k_in.keys().copied().collect();
                cands.sort_unstable();
                let mut best_c = cu;
                let mut best_gain = 0.0f64;
                for c in cands {
                    let edges_to_c = k_in[&c];
                    let gain = edges_to_c - deg[u] * sigma_tot[c as usize] / two_m;
                    if gain > best_gain {
                        best_gain = gain;
                        best_c = c;
                    }
                }
                sigma_tot[best_c as usize] += deg[u];
                comm[u] = best_c;
                if best_c != cu {
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        self.dense_communities(&comm)
    }

    /// Remap arbitrary community labels to dense `0..k` ids (first-seen order)
    /// and pair each with its node id.
    fn dense_communities(&self, comm: &[u32]) -> Vec<(String, u32)> {
        let mut remap: HashMap<u32, u32> = HashMap::new();
        comm.iter()
            .enumerate()
            .map(|(i, &c)| {
                let next = remap.len() as u32;
                let cid = *remap.entry(c).or_insert(next);
                (self.ids[i].clone(), cid)
            })
            .collect()
    }

    /// Build the weighted adjacency (edge multiplicity as weight) the
    /// hierarchical Louvain operates on — `adj[u][v] = w(u→v)` under `dir`.
    fn weighted_adjacency(&self, dir: Direction) -> Vec<HashMap<usize, f64>> {
        let n = self.ids.len();
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
        let mut nbrs: Vec<u32> = Vec::new();
        for (u, row) in adj.iter_mut().enumerate() {
            nbrs.clear();
            self.step(u as u32, dir, EtypeFilter::Any, &mut nbrs);
            for &v in &nbrs {
                *row.entry(v as usize).or_insert(0.0) += 1.0;
            }
        }
        adj
    }

    /// **Hierarchical** (multi-level) Louvain community detection (S3.4): run
    /// local moving, aggregate each community into a super-node, and recurse on
    /// the aggregated weighted graph until no level merges further. Refines the
    /// single-level [`Self::detect_communities`] — modularity is monotonically
    /// non-decreasing across levels — and yields the coarser communities that
    /// drive LazyGraphRAG-style summaries. Uses an internal weighted adjacency
    /// (built from the CSR), so the topology struct + serialization are
    /// unchanged. Deterministic. Returns `(node_id, community_id)`, dense ids.
    pub fn detect_communities_hierarchical(
        &self,
        dir: Direction,
        max_passes_per_level: usize,
        max_levels: usize,
    ) -> Vec<(String, u32)> {
        let n = self.ids.len();
        let mut adj = self.weighted_adjacency(dir);
        // original node → its node index at the current (aggregated) level.
        let mut node_comm: Vec<usize> = (0..n).collect();
        for _ in 0..max_levels.max(1) {
            let raw = louvain_local_moving(&adj, max_passes_per_level);
            let (dense, k) = densify(&raw);
            for nc in &mut node_comm {
                *nc = dense[*nc];
            }
            if k == adj.len() {
                break; // a level that merges nothing → converged
            }
            adj = aggregate(&adj, &dense, k);
        }
        let labels: Vec<u32> = node_comm.iter().map(|&c| c as u32).collect();
        self.dense_communities(&labels)
    }

    /// Serialize to the checksummed `KYCS` single-file layout.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        put_u32(&mut payload, self.ids.len() as u32);
        put_u32(&mut payload, self.etypes.len() as u32);
        put_u32(&mut payload, self.out_targets.len() as u32);
        put_u32(&mut payload, self.in_sources.len() as u32);
        for s in &self.ids {
            put_str(&mut payload, s);
        }
        for s in &self.etypes {
            put_str(&mut payload, s);
        }
        put_u32_slice(&mut payload, &self.out_offsets);
        put_u32_slice(&mut payload, &self.out_targets);
        put_u32_slice(&mut payload, &self.out_etype);
        put_u32_slice(&mut payload, &self.in_offsets);
        put_u32_slice(&mut payload, &self.in_sources);
        put_u32_slice(&mut payload, &self.in_etype);

        let mut buf = Vec::with_capacity(payload.len() + 16);
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
        buf.extend_from_slice(&fnv1a(&payload).to_le_bytes());
        buf.extend_from_slice(&payload);
        buf
    }

    /// Decode bytes written by [`Self::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TopoError> {
        if bytes.len() < 16 {
            return Err(TopoError::Corrupt("header truncated".into()));
        }
        if &bytes[0..4] != MAGIC {
            return Err(TopoError::Corrupt("bad magic".into()));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VERSION {
            return Err(TopoError::UnsupportedVersion(version));
        }
        let stored_ck = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| TopoError::Corrupt("checksum field".into()))?,
        );
        let payload = &bytes[16..];
        if fnv1a(payload) != stored_ck {
            return Err(TopoError::ChecksumMismatch);
        }

        let mut c = Cursor::new(payload);
        let n_nodes = c.u32()? as usize;
        let n_etypes = c.u32()? as usize;
        let n_out = c.u32()? as usize;
        let n_in = c.u32()? as usize;

        let mut ids = Vec::with_capacity(n_nodes);
        let mut id_to_idx = HashMap::with_capacity(n_nodes);
        for _ in 0..n_nodes {
            let s = c.string()?;
            id_to_idx.insert(s.clone(), ids.len() as u32);
            ids.push(s);
        }
        let mut etypes = Vec::with_capacity(n_etypes);
        for _ in 0..n_etypes {
            etypes.push(c.string()?);
        }
        let out_offsets = c.u32_vec(n_nodes + 1)?;
        let out_targets = c.u32_vec(n_out)?;
        let out_etype = c.u32_vec(n_out)?;
        let in_offsets = c.u32_vec(n_nodes + 1)?;
        let in_sources = c.u32_vec(n_in)?;
        let in_etype = c.u32_vec(n_in)?;

        Ok(Self {
            ids,
            id_to_idx,
            etypes,
            out_offsets,
            out_targets,
            out_etype,
            in_offsets,
            in_sources,
            in_etype,
        })
    }
}

/// One level of Louvain local moving over a weighted adjacency
/// (`adj[u][v] = w(u→v)`): greedily place each node in the neighboring community
/// that most raises modularity, swept in index order to a fixed point (or
/// `max_passes`). Self-loops (`v == u`) are always internal and excluded from
/// the move decision. Deterministic: candidates scored in ascending
/// community-id order with strict `>`. Returns a (non-dense) community per node.
fn louvain_local_moving(adj: &[HashMap<usize, f64>], max_passes: usize) -> Vec<usize> {
    let n = adj.len();
    let deg: Vec<f64> = adj.iter().map(|a| a.values().sum()).collect();
    let m = deg.iter().sum::<f64>() / 2.0;
    let mut comm: Vec<usize> = (0..n).collect();
    if m == 0.0 {
        return comm;
    }
    let two_m = 2.0 * m;
    let mut sigma_tot = deg.clone();
    let mut k_in: HashMap<usize, f64> = HashMap::new();
    for _ in 0..max_passes.max(1) {
        let mut moved = false;
        for u in 0..n {
            if deg[u] == 0.0 {
                continue;
            }
            k_in.clear();
            for (&v, &w) in &adj[u] {
                if v == u {
                    continue;
                }
                *k_in.entry(comm[v]).or_insert(0.0) += w;
            }
            let cu = comm[u];
            sigma_tot[cu] -= deg[u];
            let mut cands: Vec<usize> = k_in.keys().copied().collect();
            cands.sort_unstable();
            let mut best_c = cu;
            let mut best_gain = 0.0f64;
            for c in cands {
                let gain = k_in[&c] - deg[u] * sigma_tot[c] / two_m;
                if gain > best_gain {
                    best_gain = gain;
                    best_c = c;
                }
            }
            sigma_tot[best_c] += deg[u];
            comm[u] = best_c;
            if best_c != cu {
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    comm
}

/// Remap arbitrary labels to dense `0..k` ids (first-seen order); returns the
/// dense labeling and `k`.
fn densify(raw: &[usize]) -> (Vec<usize>, usize) {
    let mut remap: HashMap<usize, usize> = HashMap::new();
    let dense: Vec<usize> = raw
        .iter()
        .map(|&c| {
            let next = remap.len();
            *remap.entry(c).or_insert(next)
        })
        .collect();
    (dense, remap.len())
}

/// Collapse `adj` by the dense `comm` labeling into a `k`-node super-graph:
/// `sup[ci][cj] = Σ adj[u][v]` over `u∈ci, v∈cj` (intra-community weight becomes
/// a super-node self-loop). Degrees and total weight are preserved.
fn aggregate(adj: &[HashMap<usize, f64>], comm: &[usize], k: usize) -> Vec<HashMap<usize, f64>> {
    let mut sup: Vec<HashMap<usize, f64>> = vec![HashMap::new(); k];
    for (u, row) in adj.iter().enumerate() {
        let cu = comm[u];
        for (&v, &w) in row {
            *sup[cu].entry(comm[v]).or_insert(0.0) += w;
        }
    }
    sup
}

/// Build CSR arrays `(offsets, targets, etype)` for `n` nodes from
/// `(from, to, etype_id)` triples, counting-sort style (stable, O(n + e)).
fn build_csr(
    n: usize,
    triples: impl Iterator<Item = (u32, u32, u32)> + Clone,
) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let mut offsets = vec![0u32; n + 1];
    for (from, _, _) in triples.clone() {
        offsets[from as usize + 1] += 1;
    }
    for i in 0..n {
        offsets[i + 1] += offsets[i];
    }
    let total = *offsets.last().unwrap_or(&0) as usize;
    let mut targets = vec![0u32; total];
    let mut etype = vec![0u32; total];
    let mut cursor = offsets.clone();
    for (from, to, tid) in triples {
        let slot = cursor[from as usize] as usize;
        targets[slot] = to;
        etype[slot] = tid;
        cursor[from as usize] += 1;
    }
    (offsets, targets, etype)
}

/// FNV-1a 64-bit — a fast, dependency-free integrity checksum (corruption
/// detection, not cryptographic).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
}

fn put_u32_slice(buf: &mut Vec<u8>, v: &[u32]) {
    for &x in v {
        put_u32(buf, x);
    }
}

/// Bounds-checked little-endian reader over the payload.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], TopoError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| TopoError::Corrupt("length overflow".into()))?;
        if end > self.b.len() {
            return Err(TopoError::Corrupt("read past end".into()));
        }
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, TopoError> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn string(&mut self) -> Result<String, TopoError> {
        let len = self.u32()? as usize;
        let s = self.take(len)?;
        String::from_utf8(s.to_vec()).map_err(|_| TopoError::Corrupt("invalid utf8 id".into()))
    }

    fn u32_vec(&mut self, n: usize) -> Result<Vec<u32>, TopoError> {
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u32()?);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_graph() -> CsrGraph {
        // a -KNOWS-> b -KNOWS-> c -CALLS-> d
        CsrGraph::build(
            ["a", "b", "c", "d"],
            [
                ("a", "b", "KNOWS"),
                ("b", "c", "KNOWS"),
                ("c", "d", "CALLS"),
            ],
        )
    }

    #[test]
    fn k_hop_forward_reaches_at_increasing_distance() {
        let g = line_graph();
        let r = g.k_hop(&["a"], 3, Direction::Fwd, None);
        assert!(r.contains(&("a".into(), "a".into(), 0)));
        assert!(r.contains(&("a".into(), "b".into(), 1)));
        assert!(r.contains(&("a".into(), "c".into(), 2)));
        assert!(r.contains(&("a".into(), "d".into(), 3)));
        // capped at k=2 → d excluded
        let r2 = g.k_hop(&["a"], 2, Direction::Fwd, None);
        assert!(!r2.iter().any(|(_, n, _)| n == "d"));
    }

    #[test]
    fn etype_filter_restricts_edges() {
        let g = line_graph();
        // KNOWS-only: a reaches b,c but not d (the c->d edge is CALLS).
        let r = g.k_hop(&["a"], 9, Direction::Fwd, Some("KNOWS"));
        assert!(r.contains(&("a".into(), "c".into(), 2)));
        assert!(!r.iter().any(|(_, n, _)| n == "d"));
        // Unknown type matches nothing → only the source.
        let r2 = g.k_hop(&["a"], 9, Direction::Fwd, Some("NOPE"));
        assert_eq!(r2, BTreeSet::from([("a".into(), "a".into(), 0)]));
    }

    #[test]
    fn backward_and_both_directions() {
        let g = line_graph();
        // Back from d reaches c,b,a.
        let r = g.k_hop(&["d"], 9, Direction::Back, None);
        assert!(r.contains(&("d".into(), "a".into(), 3)));
        // Both is undirected: from c reach everything within 2.
        let r2 = g.k_hop(&["c"], 2, Direction::Both, None);
        for n in ["a", "b", "c", "d"] {
            assert!(r2.iter().any(|(_, x, _)| x == n), "missing {n}");
        }
    }

    #[test]
    fn shortest_path_and_var_length() {
        let g = line_graph();
        assert_eq!(g.shortest_path_len("a", "d", Direction::Fwd, None), Some(3));
        assert_eq!(g.shortest_path_len("a", "a", Direction::Fwd, None), Some(0));
        assert_eq!(g.shortest_path_len("d", "a", Direction::Fwd, None), None);
        assert_eq!(
            g.shortest_path_len("d", "a", Direction::Back, None),
            Some(3)
        );
        // var-length [2..3]: only c (2) and d (3) from a.
        let vl = g.var_length(&["a"], 2, 3, Direction::Fwd, None);
        let nodes: BTreeSet<_> = vl.iter().map(|(_, n, _)| n.clone()).collect();
        assert_eq!(nodes, BTreeSet::from(["c".to_string(), "d".to_string()]));
    }

    #[test]
    fn dangling_edges_dropped_unknown_sources_empty() {
        // edge to "zzz" (no node row) is dropped.
        let g = CsrGraph::build(["a", "b"], [("a", "b", "E"), ("a", "zzz", "E")]);
        assert_eq!(g.num_edges(), 1);
        // unknown source id → contributes nothing (not even (s,s,0)).
        let r = g.k_hop(&["ghost"], 3, Direction::Fwd, None);
        assert!(r.is_empty());
    }

    #[test]
    fn parallel_edges_kept() {
        let g = CsrGraph::build(["a", "b"], [("a", "b", "X"), ("a", "b", "Y")]);
        assert_eq!(g.num_edges(), 2);
        // Distinct still reaches b once at distance 1.
        let r = g.k_hop(&["a"], 1, Direction::Fwd, None);
        assert!(r.contains(&("a".into(), "b".into(), 1)));
    }

    #[test]
    fn serialize_round_trips() {
        let g = line_graph();
        let bytes = g.to_bytes();
        let back = CsrGraph::from_bytes(&bytes).expect("decodes");
        assert_eq!(g, back);
        // Traversal is identical post-decode.
        assert_eq!(
            g.k_hop(&["a"], 3, Direction::Fwd, None),
            back.k_hop(&["a"], 3, Direction::Fwd, None)
        );
    }

    #[test]
    fn corruption_is_detected() {
        let g = line_graph();
        let mut bytes = g.to_bytes();
        assert_eq!(
            CsrGraph::from_bytes(b"xx")
                .unwrap_err()
                .to_string()
                .is_empty(),
            false
        );
        // flip a payload byte → checksum mismatch
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        assert_eq!(
            CsrGraph::from_bytes(&bytes),
            Err(TopoError::ChecksumMismatch)
        );
        // bad magic
        let mut b2 = g.to_bytes();
        b2[0] = b'Z';
        assert!(matches!(
            CsrGraph::from_bytes(&b2),
            Err(TopoError::Corrupt(_))
        ));
    }
}
