//! Retrieval-quality metrics: recall@k, NDCG@k, MRR, latency percentiles.
//!
//! All functions are pure and deterministic. Conventions:
//!
//! - Ranked lists are best-first (`retrieved[0]` is rank 1).
//! - `recall@k` is the ANN flavor: |retrieved[..k] ∩ truth[..k]| / k —
//!   measured against the exact oracle's top-k id set.
//! - NDCG supports both gain functions: [`Gain::Linear`] (`rel / log2(i+1)`,
//!   the classic Wikipedia worked example) and [`Gain::Exponential`]
//!   (`(2^rel - 1) / log2(i+1)`, the standard for graded web relevance).
//!   The golden runner uses [`Gain::Exponential`].

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;

/// Recall@k of a retrieved ranking against the ground-truth top-k set.
///
/// Both lists are truncated to `k` before intersecting; the result is
/// `|retrieved[..k] ∩ truth[..k]| / k`. Brute-force (exact) retrieval scores
/// exactly 1.0 by construction. `k == 0` returns 0.0.
pub fn recall_at_k<T: Eq + Hash>(retrieved: &[T], ground_truth: &[T], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let truth: HashSet<&T> = ground_truth.iter().take(k).collect();
    if truth.is_empty() {
        return 0.0;
    }
    let hits = retrieved
        .iter()
        .take(k)
        .filter(|r| truth.contains(r))
        .count();
    #[allow(clippy::cast_precision_loss)]
    {
        hits as f64 / k as f64
    }
}

/// Gain function for DCG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gain {
    /// `rel_i / log2(i + 1)` — the classic formulation.
    Linear,
    /// `(2^rel_i - 1) / log2(i + 1)` — emphasizes highly-relevant docs.
    Exponential,
}

fn gain(g: Gain, rel: f64) -> f64 {
    match g {
        Gain::Linear => rel,
        Gain::Exponential => rel.exp2() - 1.0,
    }
}

/// Discounted cumulative gain at `k` over grades in rank order
/// (`grades[0]` = rank 1).
pub fn dcg_at_k(grades: &[f64], k: usize, g: Gain) -> f64 {
    grades
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| {
            #[allow(clippy::cast_precision_loss)]
            let discount = ((i + 2) as f64).log2(); // rank 1 → log2(2) = 1
            gain(g, rel) / discount
        })
        .sum()
}

/// NDCG@k: `DCG@k(ranked_grades) / DCG@k(ideal_grades sorted desc)`.
///
/// `ranked_grades` are the grades of the documents the system returned, in
/// the system's rank order (missing/unjudged docs should carry grade 0).
/// `all_judged_grades` is the full pool of judged grades for the query; the
/// ideal ranking is that pool sorted descending. Returns 0.0 when the ideal
/// DCG is 0 (no relevant documents — vacuous query).
pub fn ndcg_at_k(ranked_grades: &[f64], all_judged_grades: &[f64], k: usize, g: Gain) -> f64 {
    let mut ideal: Vec<f64> = all_judged_grades.to_vec();
    ideal.sort_by(|a, b| b.partial_cmp(a).expect("grades must not be NaN"));
    let idcg = dcg_at_k(&ideal, k, g);
    if idcg == 0.0 {
        return 0.0;
    }
    dcg_at_k(ranked_grades, k, g) / idcg
}

/// Reciprocal rank of the first relevant item: `1 / rank`, or 0.0 if no
/// retrieved item is in `relevant`.
#[allow(clippy::implicit_hasher)] // callers all use the default hasher; generality is noise here
pub fn reciprocal_rank<T: Eq + Hash>(retrieved: &[T], relevant: &HashSet<T>) -> f64 {
    for (i, item) in retrieved.iter().enumerate() {
        if relevant.contains(item) {
            #[allow(clippy::cast_precision_loss)]
            return 1.0 / (i + 1) as f64;
        }
    }
    0.0
}

/// Arithmetic mean; 0.0 for an empty slice.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// p50/p95/p99 latency summary computed from raw samples.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LatencyStats {
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl LatencyStats {
    /// Aggregate from raw [`Duration`] samples using the nearest-rank method
    /// (`ceil(p · n)`-th sorted sample), which is deterministic and never
    /// interpolates. Empty input → all zeros.
    pub fn from_durations(samples: &[Duration]) -> Self {
        let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(|a, b| a.partial_cmp(b).expect("durations are never NaN"));
        let pick = |p: f64| -> f64 {
            if ms.is_empty() {
                return 0.0;
            }
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let idx = ((p * ms.len() as f64).ceil() as usize).clamp(1, ms.len()) - 1;
            ms[idx]
        };
        Self {
            count: ms.len(),
            p50_ms: pick(0.50),
            p95_ms: pick(0.95),
            p99_ms: pick(0.99),
            max_ms: ms.last().copied().unwrap_or(0.0),
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    // ── recall@k ──────────────────────────────────────────────────────

    #[test]
    fn recall_perfect() {
        assert_eq!(recall_at_k(&[1, 2, 3], &[3, 2, 1], 3), 1.0);
    }

    #[test]
    fn recall_partial() {
        // retrieved top-4 = {1,2,3,4}, truth top-4 = {3,4,5,6} → 2/4
        assert_eq!(recall_at_k(&[1, 2, 3, 4], &[3, 4, 5, 6], 4), 0.5);
    }

    #[test]
    fn recall_zero_overlap_and_k_zero() {
        assert_eq!(recall_at_k(&[1, 2], &[3, 4], 2), 0.0);
        assert_eq!(recall_at_k(&[1, 2], &[1, 2], 0), 0.0);
    }

    #[test]
    fn recall_truncates_to_k() {
        // Item 9 appears beyond rank k in `retrieved`; must not count.
        assert_eq!(recall_at_k(&[1, 2, 9], &[9, 1], 2), 0.5);
    }

    #[test]
    fn recall_empty_truth() {
        assert_eq!(recall_at_k::<u32>(&[1, 2], &[], 2), 0.0);
    }

    // ── DCG / NDCG ────────────────────────────────────────────────────

    /// The classic worked example (Wikipedia, linear gain):
    /// grades in ranked order [3, 2, 3, 0, 1, 2]:
    ///   DCG6  = 3 + 2/log2(3) + 3/2 + 0 + 1/log2(6) + 2/log2(7) ≈ 6.86113
    ///   ideal = [3, 3, 2, 2, 1, 0] → IDCG6 ≈ 7.14086
    ///   NDCG6 ≈ 0.96083
    #[test]
    fn ndcg_classic_worked_example_linear() {
        let ranked = [3.0, 2.0, 3.0, 0.0, 1.0, 2.0];
        let dcg = dcg_at_k(&ranked, 6, Gain::Linear);
        assert_close(dcg, 6.861_126_688_593_502);
        let idcg = dcg_at_k(&[3.0, 3.0, 2.0, 2.0, 1.0, 0.0], 6, Gain::Linear);
        assert_close(idcg, 7.140_995_184_095_7);
        let ndcg = ndcg_at_k(&ranked, &ranked, 6, Gain::Linear);
        assert_close(ndcg, 6.861_126_688_593_502 / 7.140_995_184_095_7);
        assert!((ndcg - 0.960_808).abs() < 5e-6);
    }

    /// Hand-computed exponential-gain example, grades [3, 0, 2] with judged
    /// pool [3, 2, 0]:
    ///   DCG3  = (2^3-1)/log2(2) + 0 + (2^2-1)/log2(4) = 7 + 0 + 1.5 = 8.5
    ///   IDCG3 = 7 + 3/log2(3) + 0 = 7 + 1.892789... = 8.892789...
    #[test]
    fn ndcg_exponential_hand_computed() {
        let ranked = [3.0, 0.0, 2.0];
        let judged = [3.0, 2.0, 0.0];
        assert_close(dcg_at_k(&ranked, 3, Gain::Exponential), 8.5);
        let idcg = 7.0 + 3.0 / 3.0_f64.log2();
        assert_close(dcg_at_k(&judged, 3, Gain::Exponential), idcg);
        assert_close(ndcg_at_k(&ranked, &judged, 3, Gain::Exponential), 8.5 / idcg);
    }

    #[test]
    fn ndcg_perfect_ranking_is_one() {
        let judged = [3.0, 2.0, 1.0, 0.0];
        assert_close(ndcg_at_k(&judged, &judged, 4, Gain::Exponential), 1.0);
        assert_close(ndcg_at_k(&judged, &judged, 4, Gain::Linear), 1.0);
    }

    #[test]
    fn ndcg_no_relevant_docs_is_zero() {
        assert_eq!(ndcg_at_k(&[0.0, 0.0], &[0.0, 0.0], 10, Gain::Exponential), 0.0);
        assert_eq!(ndcg_at_k(&[], &[], 10, Gain::Exponential), 0.0);
    }

    #[test]
    fn ndcg_k_truncates() {
        // Only rank 1 counts at k=1: ranked [0, 3] → DCG1 = 0 → NDCG = 0.
        assert_eq!(ndcg_at_k(&[0.0, 3.0], &[3.0, 0.0], 1, Gain::Exponential), 0.0);
    }

    // ── MRR ───────────────────────────────────────────────────────────

    #[test]
    fn reciprocal_rank_first() {
        let rel: HashSet<u32> = [7].into_iter().collect();
        assert_eq!(reciprocal_rank(&[7, 1, 2], &rel), 1.0);
    }

    #[test]
    fn reciprocal_rank_third() {
        let rel: HashSet<u32> = [9].into_iter().collect();
        assert_close(reciprocal_rank(&[1, 2, 9], &rel), 1.0 / 3.0);
    }

    #[test]
    fn reciprocal_rank_miss() {
        let rel: HashSet<u32> = [42].into_iter().collect();
        assert_eq!(reciprocal_rank(&[1, 2, 3], &rel), 0.0);
        assert_eq!(reciprocal_rank(&[], &rel), 0.0);
    }

    #[test]
    fn mean_basics() {
        assert_eq!(mean(&[]), 0.0);
        assert_close(mean(&[1.0, 2.0, 3.0]), 2.0);
    }

    // ── latency ───────────────────────────────────────────────────────

    #[test]
    fn latency_percentiles_nearest_rank() {
        // 1..=100 ms: p50 = 50th sample = 50ms, p95 = 95ms, p99 = 99ms.
        let samples: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let s = LatencyStats::from_durations(&samples);
        assert_eq!(s.count, 100);
        assert_close(s.p50_ms, 50.0);
        assert_close(s.p95_ms, 95.0);
        assert_close(s.p99_ms, 99.0);
        assert_close(s.max_ms, 100.0);
    }

    #[test]
    fn latency_single_sample() {
        let s = LatencyStats::from_durations(&[Duration::from_millis(7)]);
        assert_close(s.p50_ms, 7.0);
        assert_close(s.p99_ms, 7.0);
    }

    #[test]
    fn latency_empty() {
        let s = LatencyStats::from_durations(&[]);
        assert_eq!(s.count, 0);
        assert_eq!(s.p50_ms, 0.0);
        assert_eq!(s.max_ms, 0.0);
    }

    #[test]
    fn latency_unsorted_input() {
        let samples = [3, 1, 2].map(Duration::from_millis);
        let s = LatencyStats::from_durations(&samples);
        assert_close(s.p50_ms, 2.0);
        assert_close(s.max_ms, 3.0);
    }
}
