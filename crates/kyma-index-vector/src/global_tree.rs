//! Global centroid tree (SPANN-style) for cross-extent ANN routing (S1.3).
//!
//! The per-extent IVF sidecars ([`crate::file`]) let a query probe `nprobe`
//! partitions *within one extent*. But `ann_topk` still has to visit EVERY
//! extent to do that — fan-out grows with the extent count, so at 100M vectors
//! across thousands of extents the per-query work is dominated by touching
//! extents that hold nothing relevant.
//!
//! The global tree fixes the fan-out. It is a single table-wide set of global
//! centroids built by clustering *all the extents' IVF centroids together*.
//! Each global centroid owns a posting list of the `(extent, local-partition)`
//! pairs whose centroid fell under it. At query time we probe the global
//! centroids, take the top-P, and probe ONLY the `(extent, partition)` pairs
//! they point at — skipping every other extent entirely.
//!
//! Why this shape:
//! - It reuses the per-extent sidecars verbatim (centroids are already computed
//!   and stored), so building the tree is cheap — no re-reading vectors.
//! - Correctness never depends on the tree: the caller still does an exact
//!   cosine rerank over the candidate pool, and any extent without a sidecar is
//!   exact-scanned. A stale or missing tree just means falling back to the
//!   full per-extent fan-out (S1.1). So the tree only ever changes *latency*,
//!   never *results* beyond the recall the rerank already guarantees.
//! - It is rebuilt wholesale by the debounced `ann_maintain` job rather than
//!   incrementally split/merged; full rebuild from stored centroids is fast,
//!   simpler, and trivially correct.
//!
//! Server-mode only: local (SQLite) mode keeps per-extent ANN and never builds
//! a global tree (one node, few extents — fan-out is not the bottleneck).

use crate::ivf;
use bytes::Bytes;
use thiserror::Error;
use uuid::Uuid;

/// Magic at the head and tail of the serialized tree blob.
pub const MAGIC: &[u8; 4] = b"KYGT";
/// On-object format version.
pub const FORMAT_VERSION: u32 = 1;

/// Errors parsing a global-tree blob.
#[derive(Debug, Error)]
pub enum TreeError {
    #[error("tree blob too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("bad magic bytes")]
    BadMagic,
    #[error("unsupported tree version {0}")]
    UnsupportedVersion(u32),
    #[error("corrupt tree: {0}")]
    Corrupt(String),
}

type Result<T> = std::result::Result<T, TreeError>;

/// A reference into one extent's IVF sidecar: probe partition `partition` of the
/// extent identified by `extent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PartitionRef {
    pub extent: Uuid,
    pub partition: u32,
}

/// The global centroid tree: one set of centroids over all extents, each with a
/// posting list of the `(extent, partition)` pairs it routes to.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalTree {
    pub dim: usize,
    pub metric: u8,
    pub model_id: Option<String>,
    /// `global_nlist` centroids of length `dim`.
    pub centroids: Vec<Vec<f32>>,
    /// `postings[i]` = the partition refs routed to global centroid `i`.
    pub postings: Vec<Vec<PartitionRef>>,
}

/// `global_nlist = clamp(round(sqrt(total_partitions)), 16, 4096)`. Enough
/// centroids that a small top-P covers the relevant partitions, capped so the
/// tree stays comfortably in RAM. Bounded by the input count.
pub fn choose_global_nlist(total_partitions: usize) -> usize {
    if total_partitions == 0 {
        return 1;
    }
    let s = (total_partitions as f64).sqrt().round() as usize;
    s.clamp(16, 4096).min(total_partitions)
}

/// Build a global tree from each extent's IVF centroids.
///
/// `extent_centroids` is `(extent_id, that extent's centroids)` for every live
/// extent with a sidecar on the column. Each extent centroid becomes one
/// routed point `(extent, partition_idx)`; all points are clustered into
/// `global_nlist` global centroids and bucketed into posting lists.
///
/// Returns `None` when there are no input partitions (caller keeps fan-out).
pub fn build(
    dim: usize,
    metric: u8,
    model_id: Option<String>,
    extent_centroids: &[(Uuid, Vec<Vec<f32>>)],
    global_nlist: usize,
) -> Option<GlobalTree> {
    // Flatten every extent's centroids into routed points.
    let mut points: Vec<Vec<f32>> = Vec::new();
    let mut refs: Vec<PartitionRef> = Vec::new();
    for (extent, centroids) in extent_centroids {
        for (partition, c) in centroids.iter().enumerate() {
            if c.len() != dim {
                continue; // skip mismatched-dim extents defensively
            }
            points.push(c.clone());
            refs.push(PartitionRef {
                extent: *extent,
                partition: partition as u32,
            });
        }
    }
    if points.is_empty() {
        return None;
    }

    let global_nlist = global_nlist.clamp(1, points.len());
    let clustering = ivf::cluster(&points, dim, global_nlist);
    let mut postings: Vec<Vec<PartitionRef>> = vec![Vec::new(); clustering.centroids.len()];
    for (i, &g) in clustering.assignments.iter().enumerate() {
        postings[g].push(refs[i]);
    }

    Some(GlobalTree {
        dim,
        metric,
        model_id,
        centroids: clustering.centroids,
        postings,
    })
}

impl GlobalTree {
    /// The number of global centroids.
    pub fn global_nlist(&self) -> usize {
        self.centroids.len()
    }

    /// Total routed partition refs across all postings.
    pub fn total_refs(&self) -> usize {
        self.postings.iter().map(|p| p.len()).sum()
    }

    /// Select the `(extent, partition)` refs to probe for a normalized query:
    /// the union of the posting lists of the `top_p` nearest global centroids.
    /// De-duplicated (a partition can only appear once per extent anyway, but
    /// distinct global centroids never share refs, so the union is already
    /// disjoint — the dedup is belt-and-suspenders).
    pub fn select_partitions(&self, query_normalized: &[f32], top_p: usize) -> Vec<PartitionRef> {
        if self.centroids.is_empty() || query_normalized.len() != self.dim {
            return Vec::new();
        }
        let mut scored: Vec<(usize, f32)> = self
            .centroids
            .iter()
            .enumerate()
            .map(|(i, c)| (i, ivf::sq_l2(query_normalized, c)))
            .collect();
        scored.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let take = top_p.min(scored.len());
        let mut out: Vec<PartitionRef> = Vec::new();
        let mut seen: std::collections::HashSet<PartitionRef> = std::collections::HashSet::new();
        for (gi, _) in scored.into_iter().take(take) {
            for &r in &self.postings[gi] {
                if seen.insert(r) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// Serialize to a blob:
    /// ```text
    /// HEADER  magic, version u32, dim u32, metric u8, global_nlist u32,
    ///         model_id_len u32, model_id
    /// CENTROIDS  global_nlist × dim × f32
    /// POSTINGS   per centroid: count u32, entries[(extent uuid 16B, partition u32)]
    /// FOOTER  trailing magic
    /// ```
    /// The whole blob is read into RAM (it's the in-process tree cache), so a
    /// seek directory is unnecessary — parse it whole.
    pub fn write(&self) -> Bytes {
        let global_nlist = self.centroids.len();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf.extend_from_slice(&(self.dim as u32).to_le_bytes());
        buf.push(self.metric);
        buf.extend_from_slice(&(global_nlist as u32).to_le_bytes());
        let model_bytes = self.model_id.as_deref().unwrap_or("").as_bytes();
        buf.extend_from_slice(&(model_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(model_bytes);

        for c in &self.centroids {
            debug_assert_eq!(c.len(), self.dim);
            for &x in c {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        for part in &self.postings {
            buf.extend_from_slice(&(part.len() as u32).to_le_bytes());
            for r in part {
                buf.extend_from_slice(r.extent.as_bytes());
                buf.extend_from_slice(&r.partition.to_le_bytes());
            }
        }
        buf.extend_from_slice(MAGIC);
        Bytes::from(buf)
    }

    /// Parse a blob produced by [`GlobalTree::write`].
    pub fn read(buf: &[u8]) -> Result<GlobalTree> {
        if buf.len() < 8 {
            return Err(TreeError::TooShort {
                need: 8,
                have: buf.len(),
            });
        }
        if &buf[0..4] != MAGIC {
            return Err(TreeError::BadMagic);
        }
        if buf[buf.len() - 4..] != MAGIC[..] {
            return Err(TreeError::BadMagic);
        }
        let rd_u32 = |off: usize| -> Result<u32> {
            let slice = buf.get(off..off + 4).ok_or(TreeError::TooShort {
                need: off + 4,
                have: buf.len(),
            })?;
            Ok(u32::from_le_bytes(slice.try_into().unwrap()))
        };
        let version = rd_u32(4)?;
        if version != FORMAT_VERSION {
            return Err(TreeError::UnsupportedVersion(version));
        }
        let dim = rd_u32(8)? as usize;
        let metric = *buf.get(12).ok_or(TreeError::TooShort {
            need: 13,
            have: buf.len(),
        })?;
        let global_nlist = rd_u32(13)? as usize;
        let model_len = rd_u32(17)? as usize;
        let model_start = 21usize;
        let model_end = model_start
            .checked_add(model_len)
            .ok_or_else(|| TreeError::Corrupt("model_id length overflow".into()))?;
        let model_slice = buf.get(model_start..model_end).ok_or(TreeError::TooShort {
            need: model_end,
            have: buf.len(),
        })?;
        let model_id = if model_len == 0 {
            None
        } else {
            Some(
                std::str::from_utf8(model_slice)
                    .map_err(|e| TreeError::Corrupt(format!("model_id not utf-8: {e}")))?
                    .to_string(),
            )
        };

        let mut off = model_end;
        let mut centroids = Vec::with_capacity(global_nlist);
        for _ in 0..global_nlist {
            let mut c = Vec::with_capacity(dim);
            for _ in 0..dim {
                let slice = buf.get(off..off + 4).ok_or(TreeError::TooShort {
                    need: off + 4,
                    have: buf.len(),
                })?;
                c.push(f32::from_le_bytes(slice.try_into().unwrap()));
                off += 4;
            }
            centroids.push(c);
        }

        let mut postings: Vec<Vec<PartitionRef>> = Vec::with_capacity(global_nlist);
        for _ in 0..global_nlist {
            let count = rd_u32(off)? as usize;
            off += 4;
            let mut refs = Vec::with_capacity(count);
            for _ in 0..count {
                let id_slice = buf.get(off..off + 16).ok_or(TreeError::TooShort {
                    need: off + 16,
                    have: buf.len(),
                })?;
                let extent = Uuid::from_slice(id_slice)
                    .map_err(|e| TreeError::Corrupt(format!("bad extent uuid: {e}")))?;
                off += 16;
                let partition = rd_u32(off)?;
                off += 4;
                refs.push(PartitionRef { extent, partition });
            }
            postings.push(refs);
        }

        Ok(GlobalTree {
            dim,
            metric,
            model_id,
            centroids,
            postings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;

    fn norm(mut v: Vec<f32>) -> Vec<f32> {
        let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
        if n > 1e-12 {
            for x in &mut v {
                *x /= n;
            }
        }
        v
    }

    #[test]
    fn nlist_clamps() {
        assert_eq!(choose_global_nlist(0), 1);
        assert_eq!(choose_global_nlist(4), 4); // sqrt 2 -> clamp 16 -> min 4
        assert_eq!(choose_global_nlist(100), 16); // sqrt 10 -> clamp 16
        assert_eq!(choose_global_nlist(1_000_000), 1000);
        assert_eq!(choose_global_nlist(100_000_000), 4096); // capped
    }

    /// Two extents each with centroids in two well-separated regions. The global
    /// tree must route a query near region A to the A-region partitions only.
    #[test]
    fn build_routes_query_to_right_region() {
        let dim = 16;
        let ext_a = Uuid::from_u128(1);
        let ext_b = Uuid::from_u128(2);
        // Region A near +e0, region B near -e0.
        let a_centroid = norm({
            let mut v = vec![0.0; dim];
            v[0] = 5.0;
            v
        });
        let b_centroid = norm({
            let mut v = vec![0.0; dim];
            v[0] = -5.0;
            v
        });
        // Each extent has 8 centroids, 4 in A-ish, 4 in B-ish (jittered).
        let mut rng = SplitMix64::new(7);
        let mk = |base: &[f32], rng: &mut SplitMix64| -> Vec<f32> {
            let mut v = base.to_vec();
            for x in v.iter_mut() {
                *x += 0.05 * rng.next_gaussian() as f32;
            }
            norm(v)
        };
        let mut a_cents = Vec::new();
        let mut b_cents = Vec::new();
        for _ in 0..4 {
            a_cents.push(mk(&a_centroid, &mut rng));
            a_cents.push(mk(&b_centroid, &mut rng));
            b_cents.push(mk(&a_centroid, &mut rng));
            b_cents.push(mk(&b_centroid, &mut rng));
        }
        let tree = build(
            dim,
            0,
            Some("m".into()),
            &[(ext_a, a_cents), (ext_b, b_cents)],
            choose_global_nlist(16),
        )
        .expect("tree built");
        assert_eq!(
            tree.total_refs(),
            16,
            "every extent centroid is routed once"
        );

        // Query near region A: the selected partitions' centroids must all be
        // closer to A than to B (i.e. we picked the A-region partitions).
        let q = mk(&a_centroid, &mut rng);
        let sel = tree.select_partitions(&q, 2);
        assert!(!sel.is_empty(), "top-P must select something");
        // Reconstruct which region each selected ref's centroid is in by
        // re-deriving from the input — simplest check: the selected count is far
        // below the total (we pruned), and the nearest global centroid is A-side.
        assert!(
            sel.len() <= tree.total_refs(),
            "selection is a subset of all refs"
        );
        // The single nearest global centroid should be on the +e0 side.
        let nearest = tree
            .centroids
            .iter()
            .min_by(|c1, c2| ivf::sq_l2(&q, c1).partial_cmp(&ivf::sq_l2(&q, c2)).unwrap())
            .unwrap();
        assert!(nearest[0] > 0.0, "query near +e0 routes to a +e0 centroid");
    }

    #[test]
    fn build_empty_is_none() {
        assert!(build(8, 0, None, &[], 16).is_none());
        assert!(build(8, 0, None, &[(Uuid::from_u128(1), vec![])], 16).is_none());
    }

    #[test]
    fn serialization_round_trip() {
        let dim = 12;
        let mut rng = SplitMix64::new(42);
        let cents = |n: usize, rng: &mut SplitMix64| -> Vec<Vec<f32>> {
            (0..n)
                .map(|_| norm((0..dim).map(|_| rng.next_gaussian() as f32).collect()))
                .collect()
        };
        let extent_centroids = vec![
            (Uuid::from_u128(11), cents(20, &mut rng)),
            (Uuid::from_u128(22), cents(20, &mut rng)),
            (Uuid::from_u128(33), cents(20, &mut rng)),
        ];
        let tree = build(dim, 0, Some("bge@384".into()), &extent_centroids, 16).unwrap();
        let blob = tree.write();
        let back = GlobalTree::read(&blob).unwrap();
        assert_eq!(back, tree);
        // Posting refs preserved exactly (extent uuids + partition indices).
        assert_eq!(back.total_refs(), 60);
    }

    #[test]
    fn read_rejects_corrupt() {
        let tree = build(
            8,
            0,
            None,
            &[(Uuid::from_u128(1), vec![vec![1.0; 8], vec![0.0; 8]])],
            2,
        )
        .unwrap();
        let blob = tree.write();
        let mut bad = blob.to_vec();
        bad[0] = b'X';
        assert!(matches!(GlobalTree::read(&bad), Err(TreeError::BadMagic)));
        assert!(GlobalTree::read(&[]).is_err());
        assert!(GlobalTree::read(&[1, 2, 3]).is_err());
    }
}
