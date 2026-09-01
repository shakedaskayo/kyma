//! Exact (brute-force) top-k oracle.
//!
//! The reference every approximate index is graded against: O(n·d) scan,
//! deterministic tie-break by index. Distance math intentionally mirrors
//! `crates/pensieve-exec/src/udfs_vector.rs` — f32 inputs, f64 accumulation —
//! so an oracle-vs-engine comparison on the same data measures the *index*,
//! not floating-point drift between two different implementations.

/// Distance metric. Scores are "smaller = better" for all variants so a
/// single ascending sort ranks every metric (`Dot` is negated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// `1 - cos(a, b)` — matches the `cosine_distance` UDF. Range [0, 2].
    Cosine,
    /// Euclidean distance — matches the `l2_distance` UDF.
    L2,
    /// Negated inner product (so ascending = best) — ranks identically to
    /// the `inner_product` UDF sorted descending.
    Dot,
}

/// Cosine distance with the same accumulation order as the engine UDF.
/// Returns 2.0 (worst possible) for degenerate inputs (mismatched length,
/// empty, or zero-norm) instead of the UDF's NULL — the oracle must always
/// produce a total order.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 2.0;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for i in 0..a.len() {
        let (x, y) = (f64::from(a[i]), f64::from(b[i]));
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 2.0;
    }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}

/// Euclidean distance (f64 accumulation, same as the engine UDF).
pub fn l2_distance(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    let mut s = 0f64;
    for i in 0..a.len() {
        let d = f64::from(a[i]) - f64::from(b[i]);
        s += d * d;
    }
    s.sqrt()
}

/// Inner product (f64 accumulation, same as the engine UDF).
pub fn inner_product(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::NEG_INFINITY;
    }
    let mut s = 0f64;
    for i in 0..a.len() {
        s += f64::from(a[i]) * f64::from(b[i]);
    }
    s
}

/// Score a single pair under `metric` ("smaller = better").
pub fn score(metric: Metric, a: &[f32], b: &[f32]) -> f64 {
    match metric {
        Metric::Cosine => cosine_distance(a, b),
        Metric::L2 => l2_distance(a, b),
        Metric::Dot => -inner_product(a, b),
    }
}

/// Exact top-k oracle over an in-memory dataset.
pub struct ExactOracle;

impl ExactOracle {
    /// Brute-force top-k: returns `(dataset index, score)` pairs sorted by
    /// ascending score, ties broken by ascending index. `k` is clamped to
    /// the dataset size. Deterministic for identical inputs.
    pub fn top_k(
        dataset: &[Vec<f32>],
        query: &[f32],
        k: usize,
        metric: Metric,
    ) -> Vec<(usize, f64)> {
        let mut scored: Vec<(usize, f64)> = dataset
            .iter()
            .enumerate()
            .map(|(i, v)| (i, score(metric, v, query)))
            .collect();
        // Total order: score asc, then index asc. Scores are never NaN
        // (degenerate inputs map to sentinel worst-values above).
        scored.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .expect("oracle scores are never NaN")
                .then(a.0.cmp(&b.0))
        });
        scored.truncate(k.min(dataset.len()));
        scored
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn l2_normalize(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
        if n == 0.0 {
            return v.to_vec();
        }
        #[allow(clippy::cast_possible_truncation)]
        v.iter().map(|x| (f64::from(*x) / n) as f32).collect()
    }

    #[test]
    fn top_k_hand_computed_cosine() {
        // Axis-aligned 3-vector dataset, query near +x.
        let data = vec![
            vec![0.0_f32, 1.0, 0.0], // 0: orthogonal → 1.0
            vec![1.0_f32, 0.0, 0.0], // 1: same direction → ~0.0
            vec![-1.0_f32, 0.0, 0.0], // 2: opposite → ~2.0
        ];
        let q = [1.0_f32, 0.0, 0.0];
        let top = ExactOracle::top_k(&data, &q, 2, Metric::Cosine);
        assert_eq!(top[0].0, 1);
        assert!(top[0].1.abs() < 1e-12);
        assert_eq!(top[1].0, 0);
        assert!((top[1].1 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn top_k_hand_computed_l2_and_dot() {
        let data = vec![vec![0.0_f32, 0.0], vec![3.0_f32, 4.0], vec![1.0_f32, 1.0]];
        let q = [0.0_f32, 0.0];
        let l2 = ExactOracle::top_k(&data, &q, 3, Metric::L2);
        assert_eq!(l2.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0, 2, 1]);
        assert_eq!(l2[2].1, 5.0);

        let dot = ExactOracle::top_k(&data, &[1.0_f32, 1.0], 3, Metric::Dot);
        // inner products: 0, 7, 2 → negated/ascending → 1, 2, 0
        assert_eq!(dot.iter().map(|t| t.0).collect::<Vec<_>>(), vec![1, 2, 0]);
        assert_eq!(dot[0].1, -7.0);
    }

    #[test]
    fn tie_break_by_index() {
        let data = vec![vec![1.0_f32, 0.0], vec![1.0_f32, 0.0], vec![1.0_f32, 0.0]];
        let top = ExactOracle::top_k(&data, &[1.0_f32, 0.0], 3, Metric::Cosine);
        assert_eq!(top.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn k_clamped_to_dataset() {
        let data = vec![vec![1.0_f32], vec![2.0_f32]];
        assert_eq!(ExactOracle::top_k(&data, &[1.0], 10, Metric::L2).len(), 2);
        assert!(ExactOracle::top_k(&[], &[1.0], 5, Metric::L2).is_empty());
    }

    #[test]
    fn degenerate_inputs_rank_last() {
        let data = vec![vec![0.0_f32, 0.0], vec![1.0_f32, 0.0]];
        let top = ExactOracle::top_k(&data, &[1.0_f32, 0.0], 2, Metric::Cosine);
        assert_eq!(top[0].0, 1);
        assert_eq!(top[1], (0, 2.0)); // zero-norm → sentinel 2.0
    }

    proptest! {
        /// Oracle top-1 equals argmin over an independently-written naive
        /// distance loop, for every metric.
        #[test]
        fn top1_matches_naive_argmin(
            data in prop::collection::vec(prop::collection::vec(-10.0f32..10.0, 4), 1..40),
            q in prop::collection::vec(-10.0f32..10.0, 4),
            metric_pick in 0usize..3,
        ) {
            let metric = [Metric::Cosine, Metric::L2, Metric::Dot][metric_pick];
            let top = ExactOracle::top_k(&data, &q, 1, metric);
            // Naive loop: track best (score, index) with strict-< update so
            // ties resolve to the lowest index, same as the oracle contract.
            let mut best = (0usize, score(metric, &data[0], &q));
            for (i, v) in data.iter().enumerate().skip(1) {
                let s = score(metric, v, &q);
                if s < best.1 {
                    best = (i, s);
                }
            }
            prop_assert_eq!(top[0].0, best.0);
            prop_assert_eq!(top[0].1, best.1);
        }

        /// On L2-normalized vectors, cosine ordering equals dot-based
        /// ordering: for any pair with a non-trivial inner-product gap, the
        /// cosine distances order the same way.
        #[test]
        fn cosine_of_normalized_matches_dot_ranking(
            raw in prop::collection::vec(prop::collection::vec(-10.0f32..10.0, 8), 2..20),
            rq in prop::collection::vec(-10.0f32..10.0, 8),
        ) {
            // Skip near-zero vectors (normalization undefined).
            prop_assume!(rq.iter().any(|x| x.abs() > 1e-3));
            prop_assume!(raw.iter().all(|v| v.iter().any(|x| x.abs() > 1e-3)));
            let data: Vec<Vec<f32>> = raw.iter().map(|v| l2_normalize(v)).collect();
            let q = l2_normalize(&rq);

            for i in 0..data.len() {
                for j in (i + 1)..data.len() {
                    let di = inner_product(&data[i], &q);
                    let dj = inner_product(&data[j], &q);
                    // Only assert where the dot gap clears f32-normalization
                    // noise — exact ties are tie-broken by index, not score.
                    if (di - dj).abs() > 1e-4 {
                        let ci = cosine_distance(&data[i], &q);
                        let cj = cosine_distance(&data[j], &q);
                        prop_assert_eq!(
                            di > dj,
                            ci < cj,
                            "dot/cosine ranking disagree: di={} dj={} ci={} cj={}",
                            di,
                            dj,
                            ci,
                            cj
                        );
                    }
                }
            }
        }
    }
}
