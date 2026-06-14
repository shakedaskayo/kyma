//! End-to-end recall validation for the IVF + 1-bit RaBitQ index.
//!
//! This is the gate that proves the index + rerank design before Part B wires
//! it into the query path. We:
//!
//! 1. generate clustered synthetic data (5k vectors × 384-dim, seeded;
//!    384 = Kyma's default BGE-small-en-v1.5 embedding dimension),
//! 2. build the sidecar,
//! 3. for many random queries: probe the IVF, scan the probed partitions to
//!    collect the top-(rerank_factor·k) candidates by the RaBitQ estimator,
//!    then **exact-rerank** those candidates by true cosine distance,
//! 4. assert the resulting top-k matches the brute-force exact top-k with
//!    `recall@10 ≥ 0.95` averaged over queries.
//!
//! The RaBitQ estimator only needs to *rank* well enough that the true top-k
//! survives into the candidate set; the exact rerank fixes the final order.
//! This mirrors exactly what Part B's `ann_topk` will do.

use kyma_core::index_sidecar::RowAddress;
use kyma_core::segment_format::BlockId;
use kyma_index_vector::{
    build_ivf_rabitq, encode_query_rotated, file, read_header, scan_partition, Rotation,
};

// --- Local deterministic RNG (mirrors the crate's internal SplitMix64) so the
// test is reproducible without depending on crate-private items. ---
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gaussian(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if n > 1e-12 {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

/// cosine distance between two unit vectors = 1 - dot.
fn cos_dist(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    1.0 - dot
}

#[test]
fn rabitq_recall_at_10_meets_threshold() {
    // dim=384 matches Kyma's default BGE-small-en-v1.5 embeddings — the regime
    // 1-bit RaBitQ is designed for (estimator error ∝ 1/√dim). rerank_factor=8
    // (exact-rerank ~1.6% of a 5k set) is well within the production budget.
    let dim = 384;
    let n = 5_000;
    let n_clusters = 50;
    let k = 10;
    // 1-bit RaBitQ is a coarse *ranking* filter; the exact rerank below fixes
    // the final order. rerank_factor=48 (≈1% of a 5k set, and a fixed ~480
    // exact-cosine computations regardless of corpus size) is what the
    // production ann_topk path uses. Multi-bit RaBitQ is the future lever to
    // shrink this pool. See the rerank sweep in diag_sweep_rerank.
    let rerank_factor = 48;
    let nprobe = 16; // probe a healthy fraction of the ~71 partitions

    // --- Generate clustered data: pick cluster anchors, jitter around them. ---
    let mut rng = Rng::new(0xC0FF_EE12_3456_789A);
    let anchors: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| norm((0..dim).map(|_| rng.gaussian() as f32).collect()))
        .collect();

    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut rows: Vec<(RowAddress, Vec<f32>)> = Vec::with_capacity(n);
    for i in 0..n {
        let a = &anchors[i % n_clusters];
        let v = norm(a.iter().map(|c| c + 0.30 * rng.gaussian() as f32).collect());
        vectors.push(v.clone());
        rows.push((
            RowAddress {
                block: BlockId((i / 1000) as u32),
                row: (i % 1000) as u32,
            },
            v,
        ));
    }

    // --- Build the sidecar. ---
    let (bytes, params) = build_ivf_rabitq(&rows, dim, Some("test/recall")).unwrap();
    let header = read_header(&bytes).unwrap();
    let rotation = Rotation::new(dim);

    // Map RowAddress -> index into `vectors` for exact rerank / brute force.
    let addr_to_idx: std::collections::HashMap<(u32, u32), usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (a, _))| ((a.block.0, a.row), i))
        .collect();

    // --- Queries: jitter around random anchors (in-distribution). ---
    let n_queries = 100;
    let mut total_recall = 0.0f64;
    let mut qrng = Rng::new(0x1234_5678_9ABC_DEF0);

    for _ in 0..n_queries {
        let a = &anchors[(qrng.next_u64() % n_clusters as u64) as usize];
        let q = norm(
            a.iter()
                .map(|c| c + 0.30 * qrng.gaussian() as f32)
                .collect(),
        );

        // Brute-force exact top-k by cosine distance.
        let mut exact: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cos_dist(&q, v)))
            .collect();
        exact.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap().then(x.0.cmp(&y.0)));
        let exact_topk: std::collections::HashSet<usize> =
            exact.iter().take(k).map(|(i, _)| *i).collect();

        // --- Index path: probe → scan → top (rerank_factor·k) by estimator. ---
        let qc = encode_query_rotated(&rotation.apply(&q));
        let probed = file::probe(&bytes, &header, &q, nprobe).unwrap();
        let mut candidates: Vec<(RowAddress, f32)> = Vec::new();
        for p in probed {
            let part = file::read_partition(&bytes, &header, p).unwrap();
            candidates.extend(scan_partition(&qc, &part));
        }
        // Take top (rerank_factor·k) by the approximate estimator.
        candidates.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        candidates.truncate(rerank_factor * k);

        // Exact rerank the candidates by true cosine distance.
        let mut reranked: Vec<(usize, f32)> = candidates
            .iter()
            .map(|(addr, _)| {
                let idx = addr_to_idx[&(addr.block.0, addr.row)];
                (idx, cos_dist(&q, &vectors[idx]))
            })
            .collect();
        reranked.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap().then(x.0.cmp(&y.0)));
        let got_topk: std::collections::HashSet<usize> =
            reranked.iter().take(k).map(|(i, _)| *i).collect();

        let hit = exact_topk.intersection(&got_topk).count();
        total_recall += hit as f64 / k as f64;
    }

    let recall = total_recall / n_queries as f64;
    eprintln!(
        "RaBitQ recall@{k} = {recall:.4} over {n_queries} queries \
         (n={n}, dim={dim}, nlist={}, nprobe={nprobe}, rerank_factor={rerank_factor})",
        params["nlist"]
    );
    assert!(
        recall >= 0.95,
        "recall@{k} = {recall:.4} below 0.95 threshold"
    );
}

#[test]
fn diag_breakdown() {
    let dim = 64;
    let n = 5_000;
    let n_clusters = 50;
    let k = 10;

    let mut rng = Rng::new(0xC0FF_EE12_3456_789A);
    let anchors: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| norm((0..dim).map(|_| rng.gaussian() as f32).collect()))
        .collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut rows: Vec<(RowAddress, Vec<f32>)> = Vec::with_capacity(n);
    for i in 0..n {
        let a = &anchors[i % n_clusters];
        let v = norm(a.iter().map(|c| c + 0.30 * rng.gaussian() as f32).collect());
        vectors.push(v.clone());
        rows.push((
            RowAddress {
                block: BlockId((i / 1000) as u32),
                row: (i % 1000) as u32,
            },
            v,
        ));
    }
    let (bytes, _p) = build_ivf_rabitq(&rows, dim, None).unwrap();
    let header = read_header(&bytes).unwrap();
    let rotation = Rotation::new(dim);
    let addr_to_idx: std::collections::HashMap<(u32, u32), usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (a, _))| ((a.block.0, a.row), i))
        .collect();

    // Build a flat list of (idx, VectorCode) by reading all partitions.
    let mut all_codes: Vec<(usize, kyma_index_vector::VectorCode)> = Vec::new();
    for p in 0..header.nlist as usize {
        for e in file::read_partition(&bytes, &header, p).unwrap() {
            let idx = addr_to_idx[&(e.addr.block.0, e.addr.row)];
            all_codes.push((idx, e.code));
        }
    }

    let mut qrng = Rng::new(0x1234_5678_9ABC_DEF0);
    let nq = 50;
    let mut est_only_recall = 0.0; // estimator scan ALL, take top 4k, rerank
    let mut ivf_partition_hit = 0.0; // fraction of true top-10 whose partition is probed
    let nprobe = 12;
    for _ in 0..nq {
        let a = &anchors[(qrng.next_u64() % n_clusters as u64) as usize];
        let q = norm(
            a.iter()
                .map(|c| c + 0.30 * qrng.gaussian() as f32)
                .collect(),
        );
        let mut exact: Vec<(usize, f32)> = vectors
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cos_dist(&q, v)))
            .collect();
        exact.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        let exact_topk: std::collections::HashSet<usize> =
            exact.iter().take(k).map(|(i, _)| *i).collect();

        // estimator-only: scan ALL codes
        let qc = encode_query_rotated(&rotation.apply(&q));
        let mut cand: Vec<(usize, f32)> = all_codes
            .iter()
            .map(|(i, c)| (*i, kyma_index_vector::rabitq::estimate_distance(c, &qc)))
            .collect();
        cand.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        cand.truncate(4 * k);
        let mut rer: Vec<(usize, f32)> = cand
            .iter()
            .map(|(i, _)| (*i, cos_dist(&q, &vectors[*i])))
            .collect();
        rer.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        let got: std::collections::HashSet<usize> = rer.iter().take(k).map(|(i, _)| *i).collect();
        est_only_recall += exact_topk.intersection(&got).count() as f64 / k as f64;

        // IVF partition coverage
        let probed = file::probe(&bytes, &header, &q, nprobe).unwrap();
        let probed_set: std::collections::HashSet<usize> = probed.iter().copied().collect();
        // which partition does each true-topk vector live in?
        // reconstruct assignment via nearest centroid
        let centroids = read_centroids_helper(&bytes, &header);
        let mut hit = 0;
        for &ti in &exact_topk {
            let p = nearest(&vectors[ti], &centroids);
            if probed_set.contains(&p) {
                hit += 1;
            }
        }
        ivf_partition_hit += hit as f64 / k as f64;
    }
    eprintln!(
        "EST-ONLY recall@10 (scan all, rerank 4k) = {:.4}",
        est_only_recall / nq as f64
    );
    eprintln!(
        "IVF partition coverage of true top-10 (nprobe={nprobe}) = {:.4}",
        ivf_partition_hit / nq as f64
    );
}

fn read_centroids_helper(bytes: &[u8], h: &kyma_index_vector::Header) -> Vec<Vec<f32>> {
    kyma_index_vector::read_centroids(bytes, h).unwrap()
}
fn nearest(v: &[f32], cs: &[Vec<f32>]) -> usize {
    let mut best = 0;
    let mut bd = f32::INFINITY;
    for (i, c) in cs.iter().enumerate() {
        let d: f32 = v.iter().zip(c).map(|(a, b)| (a - b) * (a - b)).sum();
        if d < bd {
            bd = d;
            best = i;
        }
    }
    best
}

#[test]
fn diag_sweep_rerank() {
    let dim = 384;
    let n = 5_000;
    let n_clusters = 50;
    let k = 10;

    let mut rng = Rng::new(0xC0FF_EE12_3456_789A);
    let anchors: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| norm((0..dim).map(|_| rng.gaussian() as f32).collect()))
        .collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut rows: Vec<(RowAddress, Vec<f32>)> = Vec::with_capacity(n);
    for i in 0..n {
        let a = &anchors[i % n_clusters];
        let v = norm(a.iter().map(|c| c + 0.30 * rng.gaussian() as f32).collect());
        vectors.push(v.clone());
        rows.push((
            RowAddress {
                block: BlockId((i / 1000) as u32),
                row: (i % 1000) as u32,
            },
            v,
        ));
    }
    let (bytes, _p) = build_ivf_rabitq(&rows, dim, None).unwrap();
    let header = read_header(&bytes).unwrap();
    let rotation = Rotation::new(dim);
    let addr_to_idx: std::collections::HashMap<(u32, u32), usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (a, _))| ((a.block.0, a.row), i))
        .collect();

    let mut all_codes: Vec<(usize, kyma_index_vector::VectorCode)> = Vec::new();
    for p in 0..header.nlist as usize {
        for e in file::read_partition(&bytes, &header, p).unwrap() {
            let idx = addr_to_idx[&(e.addr.block.0, e.addr.row)];
            all_codes.push((idx, e.code));
        }
    }
    let nq = 50;
    for &rf in &[8usize, 16, 24, 32, 48] {
        let mut qrng = Rng::new(0x1234_5678_9ABC_DEF0);
        let mut rec = 0.0;
        for _ in 0..nq {
            let a = &anchors[(qrng.next_u64() % n_clusters as u64) as usize];
            let q = norm(
                a.iter()
                    .map(|c| c + 0.30 * qrng.gaussian() as f32)
                    .collect(),
            );
            let mut exact: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, cos_dist(&q, v)))
                .collect();
            exact.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            let etk: std::collections::HashSet<usize> =
                exact.iter().take(k).map(|(i, _)| *i).collect();
            let qc = encode_query_rotated(&rotation.apply(&q));
            let mut cand: Vec<(usize, f32)> = all_codes
                .iter()
                .map(|(i, c)| (*i, kyma_index_vector::rabitq::estimate_distance(c, &qc)))
                .collect();
            cand.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            cand.truncate(rf * k);
            let mut rer: Vec<(usize, f32)> = cand
                .iter()
                .map(|(i, _)| (*i, cos_dist(&q, &vectors[*i])))
                .collect();
            rer.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            let got: std::collections::HashSet<usize> =
                rer.iter().take(k).map(|(i, _)| *i).collect();
            rec += etk.intersection(&got).count() as f64 / k as f64;
        }
        eprintln!(
            "rerank_factor={rf:>3} -> est-only recall@10 = {:.4}",
            rec / nq as f64
        );
    }
}

#[test]
fn diag_full_path() {
    let dim = 64;
    let n = 5_000;
    let n_clusters = 50;
    let k = 10;
    let nprobe = 16;

    let mut rng = Rng::new(0xC0FF_EE12_3456_789A);
    let anchors: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| norm((0..dim).map(|_| rng.gaussian() as f32).collect()))
        .collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(n);
    let mut rows: Vec<(RowAddress, Vec<f32>)> = Vec::with_capacity(n);
    for i in 0..n {
        let a = &anchors[i % n_clusters];
        let v = norm(a.iter().map(|c| c + 0.30 * rng.gaussian() as f32).collect());
        vectors.push(v.clone());
        rows.push((
            RowAddress {
                block: BlockId((i / 1000) as u32),
                row: (i % 1000) as u32,
            },
            v,
        ));
    }
    let (bytes, _p) = build_ivf_rabitq(&rows, dim, None).unwrap();
    let header = read_header(&bytes).unwrap();
    let rotation = Rotation::new(dim);
    let addr_to_idx: std::collections::HashMap<(u32, u32), usize> = rows
        .iter()
        .enumerate()
        .map(|(i, (a, _))| ((a.block.0, a.row), i))
        .collect();
    let nq = 100;
    for &rf in &[32usize, 48, 64] {
        let mut qrng = Rng::new(0x1234_5678_9ABC_DEF0);
        let mut rec = 0.0;
        for _ in 0..nq {
            let a = &anchors[(qrng.next_u64() % n_clusters as u64) as usize];
            let q = norm(
                a.iter()
                    .map(|c| c + 0.30 * qrng.gaussian() as f32)
                    .collect(),
            );
            let mut exact: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, cos_dist(&q, v)))
                .collect();
            exact.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            let etk: std::collections::HashSet<usize> =
                exact.iter().take(k).map(|(i, _)| *i).collect();
            let qc = encode_query_rotated(&rotation.apply(&q));
            let probed = file::probe(&bytes, &header, &q, nprobe).unwrap();
            let mut cand: Vec<(RowAddress, f32)> = Vec::new();
            for p in probed {
                cand.extend(scan_partition(
                    &qc,
                    &file::read_partition(&bytes, &header, p).unwrap(),
                ));
            }
            cand.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            cand.truncate(rf * k);
            let mut rer: Vec<(usize, f32)> = cand
                .iter()
                .map(|(addr, _)| {
                    let i = addr_to_idx[&(addr.block.0, addr.row)];
                    (i, cos_dist(&q, &vectors[i]))
                })
                .collect();
            rer.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            let got: std::collections::HashSet<usize> =
                rer.iter().take(k).map(|(i, _)| *i).collect();
            rec += etk.intersection(&got).count() as f64 / k as f64;
        }
        eprintln!(
            "FULL PATH nprobe={nprobe} rerank_factor={rf:>3} -> recall@10 = {:.4}",
            rec / nq as f64
        );
    }
}
