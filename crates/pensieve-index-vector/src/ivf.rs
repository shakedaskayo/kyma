//! IVF (inverted-file) coarse quantizer: mini-batch k-means over
//! L2-normalized rows.
//!
//! We partition the extent's rows into `nlist` cells, each represented by a
//! centroid. At query time the probe step (see [`crate::file`]) picks the
//! `nprobe` nearest centroids and only those partitions are scanned. On the
//! unit sphere (all rows are L2-normalized), squared Euclidean distance is a
//! monotone function of cosine distance — `cosine_distance = ½‖a−b‖²` — so
//! clustering by squared L2 is equivalent to clustering by cosine.
//!
//! The clustering is fully deterministic: initialization and mini-batch
//! sampling both draw from a fixed-seed PRNG (see [`crate::rng`]). The same
//! input always yields byte-identical centroids, which is what makes index
//! rebuilds converge.

use crate::rng::{SplitMix64, Xorshift128};

/// Fixed seed for the IVF clustering. Changing this changes every index — it
/// is part of the on-object contract, not a tunable.
const KMEANS_SEED: u64 = 0x1F1F_2E2E_3D3D_4C4C;

/// Number of mini-batch k-means iterations. ~10 is plenty for a coarse IVF
/// quantizer whose only job is to route probes; exact rerank (Part B) fixes
/// any residual ordering error.
const KMEANS_ITERS: usize = 10;

/// Mini-batch size as a multiple of `nlist`, capped at the row count. Larger
/// batches stabilize the centroid updates.
const BATCH_PER_CLUSTER: usize = 64;

/// Pick `nlist = clamp(round(sqrt(n_rows)), 16, 256)`.
pub fn choose_nlist(n_rows: usize) -> usize {
    if n_rows == 0 {
        return 1;
    }
    let s = (n_rows as f64).sqrt().round() as usize;
    s.clamp(16, 256).min(n_rows)
}

/// Squared Euclidean distance between two equal-length vectors.
#[inline]
pub fn sq_l2(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x - y;
        acc += d * d;
    }
    acc
}

/// Index of the nearest centroid to `v` (by squared L2). Ties resolve to the
/// lowest index, keeping assignment deterministic.
#[inline]
pub fn nearest_centroid(v: &[f32], centroids: &[Vec<f32>]) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, c) in centroids.iter().enumerate() {
        let d = sq_l2(v, c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// Result of clustering: the centroids and a per-row partition assignment
/// aligned with the input `rows` order.
#[derive(Debug, Clone)]
pub struct Clustering {
    pub centroids: Vec<Vec<f32>>,
    pub assignments: Vec<usize>,
}

/// Cluster `rows` (each of length `dim`, assumed L2-normalized) into `nlist`
/// partitions via deterministic mini-batch k-means.
///
/// Returns `nlist` centroids and a partition index for every input row.
pub fn cluster(rows: &[Vec<f32>], dim: usize, nlist: usize) -> Clustering {
    let n = rows.len();
    assert!(n > 0, "cluster() requires at least one row");
    let nlist = nlist.clamp(1, n);

    // --- Deterministic initialization: distinct rows as seeds. ---
    // We sample without replacement using a seeded PRNG so the seed set is
    // reproducible, then de-dup so identical rows don't collapse a centroid.
    let mut init_rng = SplitMix64::new(KMEANS_SEED);
    let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(nlist);
    let mut used = vec![false; n];
    let mut attempts = 0;
    while centroids.len() < nlist && attempts < n * 4 {
        let idx = (init_rng.next_u64() % n as u64) as usize;
        attempts += 1;
        if used[idx] {
            continue;
        }
        used[idx] = true;
        centroids.push(rows[idx].clone());
    }
    // If de-dup / sampling came up short (e.g. many duplicate rows), top up
    // deterministically by scanning forward.
    let mut scan = 0usize;
    while centroids.len() < nlist && scan < n {
        if !used[scan] {
            used[scan] = true;
            centroids.push(rows[scan].clone());
        }
        scan += 1;
    }
    // Degenerate fallback: fewer distinct rows than nlist — pad with copies.
    while centroids.len() < nlist {
        centroids.push(rows[centroids.len() % n].clone());
    }

    // --- Mini-batch k-means. ---
    // Per-centroid running counts give each centroid its own learning-rate
    // schedule (Sculley 2010, "Web-scale k-means clustering").
    let batch = (nlist * BATCH_PER_CLUSTER).min(n);
    let mut counts = vec![0u64; nlist];
    let mut sampler = Xorshift128::new(KMEANS_SEED ^ 0xA5A5_A5A5_A5A5_A5A5);

    for _ in 0..KMEANS_ITERS {
        // Sample a mini-batch of row indices (with replacement — fine for the
        // running-mean update and keeps it deterministic + cheap).
        for _ in 0..batch {
            let ri = sampler.next_below(n);
            let v = &rows[ri];
            let c = nearest_centroid(v, &centroids);
            counts[c] += 1;
            let eta = 1.0f32 / counts[c] as f32;
            let cen = &mut centroids[c];
            for j in 0..dim {
                cen[j] += eta * (v[j] - cen[j]);
            }
        }
    }

    // --- Final hard assignment of every row. ---
    let assignments: Vec<usize> = rows
        .iter()
        .map(|v| nearest_centroid(v, &centroids))
        .collect();

    Clustering {
        centroids,
        assignments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(choose_nlist(0), 1);
        assert_eq!(choose_nlist(4), 4); // sqrt(4)=2 -> clamp 16 -> min(16,4)=4
        assert_eq!(choose_nlist(100), 16); // sqrt=10 -> clamp to 16
        assert_eq!(choose_nlist(40_000), 200);
        assert_eq!(choose_nlist(1_000_000), 256); // capped
    }

    #[test]
    fn kmeans_is_deterministic() {
        let dim = 8;
        let mut rng = SplitMix64::new(99);
        let rows: Vec<Vec<f32>> = (0..500)
            .map(|_| norm((0..dim).map(|_| rng.next_gaussian() as f32).collect()))
            .collect();
        let a = cluster(&rows, dim, 16);
        let b = cluster(&rows, dim, 16);
        assert_eq!(a.centroids, b.centroids);
        assert_eq!(a.assignments, b.assignments);
    }

    #[test]
    fn kmeans_separates_two_blobs() {
        // Two well-separated blobs on the sphere should land in distinct cells.
        let dim = 16;
        let mut rng = SplitMix64::new(7);
        let mut rows = Vec::new();
        // Blob A near +e0, blob B near -e0.
        for _ in 0..200 {
            let mut v = vec![0.0f32; dim];
            v[0] = 5.0;
            for x in v.iter_mut() {
                *x += 0.1 * rng.next_gaussian() as f32;
            }
            rows.push(norm(v));
        }
        for _ in 0..200 {
            let mut v = vec![0.0f32; dim];
            v[0] = -5.0;
            for x in v.iter_mut() {
                *x += 0.1 * rng.next_gaussian() as f32;
            }
            rows.push(norm(v));
        }
        let c = cluster(&rows, dim, 16);
        // The first blob and second blob should not all share one partition.
        let a_parts: std::collections::HashSet<_> = c.assignments[..200].iter().collect();
        let b_parts: std::collections::HashSet<_> = c.assignments[200..].iter().collect();
        // Their dominant partitions must differ.
        assert!(
            a_parts.is_disjoint(&b_parts) || {
                // allow small overlap but require the modes to differ
                let mode = |s: &[usize]| {
                    let mut counts = std::collections::HashMap::new();
                    for &p in s {
                        *counts.entry(p).or_insert(0usize) += 1;
                    }
                    *counts.iter().max_by_key(|(_, n)| **n).unwrap().0
                };
                mode(&c.assignments[..200]) != mode(&c.assignments[200..])
            }
        );
    }
}
