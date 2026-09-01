//! Seeded synthetic vector datasets.
//!
//! Reproducibility is the contract: the same `(n, dim, …, seed)` arguments
//! produce bit-identical `Vec<Vec<f32>>` output on every platform and every
//! run, so the bench bin can regenerate the exact dataset it ingested and
//! hand it to the oracle without persisting anything.
//!
//! RNG: a local xorshift64* generator rather than `fastrand`. `fastrand`'s
//! seeded stream is deterministic *within* a release but is not a documented
//! stability guarantee across versions; the 15 lines of xorshift below make
//! the byte-stability promise ours instead of a dependency's.

/// Minimal xorshift64* PRNG — deterministic, seedable, no_std-grade.
#[derive(Debug, Clone)]
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    /// Seed the generator. A zero seed is remapped (xorshift fixed point).
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
        }
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform f64 in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        // 53 mantissa bits → exact dyadic rationals; identical everywhere.
        #[allow(clippy::cast_precision_loss)]
        {
            (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Uniform usize in [0, bound). `bound` must be > 0.
    pub fn next_below(&mut self, bound: usize) -> usize {
        debug_assert!(bound > 0);
        #[allow(clippy::cast_possible_truncation)]
        {
            (self.next_u64() % bound as u64) as usize
        }
    }

    /// Standard normal sample via Box–Muller. Uses two uniforms per call
    /// (no caching, keeps the stream position trivially predictable).
    pub fn next_gaussian(&mut self) -> f64 {
        // Guard u1 away from 0 so ln() stays finite.
        let u1 = self.next_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

fn l2_normalize_in_place(v: &mut [f32]) {
    let norm = v
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            #[allow(clippy::cast_possible_truncation)]
            {
                *x = (f64::from(*x) / norm) as f32;
            }
        }
    }
}

fn gaussian_vec(rng: &mut XorShift64, dim: usize) -> Vec<f32> {
    #[allow(clippy::cast_possible_truncation)]
    (0..dim).map(|_| rng.next_gaussian() as f32).collect()
}

/// Clustered dataset: `n_clusters` centers drawn uniformly on the unit
/// sphere (gaussian + normalize), points assigned round-robin as
/// `center + N(0, 0.15²)` noise, then L2-normalized. Round-robin assignment
/// keeps cluster sizes balanced and the generation order deterministic.
///
/// `n_clusters == 0` is treated as 1.
pub fn clustered_gaussian(n: usize, dim: usize, n_clusters: usize, seed: u64) -> Vec<Vec<f32>> {
    const NOISE_SIGMA: f64 = 0.15;
    let n_clusters = n_clusters.max(1);
    let mut rng = XorShift64::new(seed);

    let centers: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| {
            let mut c = gaussian_vec(&mut rng, dim);
            l2_normalize_in_place(&mut c);
            c
        })
        .collect();

    (0..n)
        .map(|i| {
            let center = &centers[i % n_clusters];
            let mut v: Vec<f32> = center
                .iter()
                .map(|c| {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        c + (rng.next_gaussian() * NOISE_SIGMA) as f32
                    }
                })
                .collect();
            l2_normalize_in_place(&mut v);
            v
        })
        .collect()
}

/// Uniform random dataset: each component uniform in [-1, 1). Not
/// normalized — exercises varying-norm code paths.
pub fn uniform_random(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = XorShift64::new(seed);
    (0..n)
        .map(|_| {
            (0..dim)
                .map(|_| {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        (rng.next_f64() * 2.0 - 1.0) as f32
                    }
                })
                .collect()
        })
        .collect()
}

/// Sample `n_queries` query vectors from `dataset`: pick a row uniformly,
/// add `N(0, perturb²)` per-component noise, L2-normalize. Seeded
/// independently from the dataset so the same dataset can be probed with
/// many query sets.
///
/// Panics if `dataset` is empty.
pub fn queries_from_dataset(
    dataset: &[Vec<f32>],
    n_queries: usize,
    perturb: f64,
    seed: u64,
) -> Vec<Vec<f32>> {
    assert!(!dataset.is_empty(), "queries_from_dataset: empty dataset");
    let mut rng = XorShift64::new(seed);
    (0..n_queries)
        .map(|_| {
            let base = &dataset[rng.next_below(dataset.len())];
            let mut q: Vec<f32> = base
                .iter()
                .map(|x| {
                    #[allow(clippy::cast_possible_truncation)]
                    {
                        x + (rng.next_gaussian() * perturb) as f32
                    }
                })
                .collect();
            l2_normalize_in_place(&mut q);
            q
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(data: &[Vec<f32>]) -> Vec<u8> {
        data.iter()
            .flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes()))
            .collect()
    }

    #[test]
    fn clustered_same_seed_identical_bytes() {
        let a = clustered_gaussian(64, 16, 4, 42);
        let b = clustered_gaussian(64, 16, 4, 42);
        assert_eq!(bytes(&a), bytes(&b));
    }

    #[test]
    fn clustered_different_seed_differs() {
        let a = clustered_gaussian(64, 16, 4, 42);
        let b = clustered_gaussian(64, 16, 4, 43);
        assert_ne!(bytes(&a), bytes(&b));
    }

    #[test]
    fn uniform_same_seed_identical_bytes() {
        let a = uniform_random(64, 16, 7);
        let b = uniform_random(64, 16, 7);
        assert_eq!(bytes(&a), bytes(&b));
        assert_ne!(bytes(&a), bytes(&uniform_random(64, 16, 8)));
    }

    #[test]
    fn queries_same_seed_identical_bytes() {
        let data = clustered_gaussian(32, 8, 2, 1);
        let a = queries_from_dataset(&data, 10, 0.05, 99);
        let b = queries_from_dataset(&data, 10, 0.05, 99);
        assert_eq!(bytes(&a), bytes(&b));
        assert_ne!(bytes(&a), bytes(&queries_from_dataset(&data, 10, 0.05, 100)));
    }

    #[test]
    fn shapes_are_correct() {
        let d = clustered_gaussian(10, 5, 3, 0);
        assert_eq!(d.len(), 10);
        assert!(d.iter().all(|v| v.len() == 5));
        let u = uniform_random(3, 7, 0);
        assert_eq!((u.len(), u[0].len()), (3, 7));
        let q = queries_from_dataset(&d, 4, 0.1, 1);
        assert_eq!((q.len(), q[0].len()), (4, 5));
    }

    #[test]
    fn clustered_vectors_are_unit_norm() {
        for v in clustered_gaussian(20, 12, 3, 5) {
            let n = v.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
            assert!((n - 1.0).abs() < 1e-3, "norm {n} not ~1");
        }
    }

    #[test]
    fn uniform_components_in_range() {
        for v in uniform_random(50, 8, 11) {
            assert!(v.iter().all(|x| (-1.0..1.0).contains(x)));
        }
    }

    #[test]
    fn perturbed_queries_stay_near_source_cluster() {
        // With small perturbation the nearest neighbor of a query should be
        // very close in cosine terms (sanity that queries are "on-manifold").
        let data = clustered_gaussian(100, 16, 4, 3);
        let queries = queries_from_dataset(&data, 5, 0.01, 17);
        for q in &queries {
            let best = crate::oracle::ExactOracle::top_k(&data, q, 1, crate::oracle::Metric::Cosine);
            assert!(best[0].1 < 0.05, "nearest cosine distance {} too large", best[0].1);
        }
    }
}
