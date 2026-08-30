//! Deterministic, dependency-free PRNGs used by the index builder.
//!
//! Index builds must be reproducible: the same (rows, dim) input must always
//! produce byte-identical sidecars so re-runs of the `index_build` job
//! converge (see [`pensieve_core::index_sidecar::SidecarBuilder`]). We therefore
//! never touch `thread_rng` / `fastrand` / the OS RNG. Everything seeds from a
//! fixed constant.
//!
//! - [`SplitMix64`] seeds the k-means initialization and the RaBitQ rotation.
//! - [`Xorshift128`] is the cheap per-step stream for k-means mini-batch
//!   sampling.

/// SplitMix64 — a tiny, well-distributed 64-bit generator (Steele, Lea & Flood
/// 2014). Used to seed other generators and to draw the Gaussian entries of the
/// RaBitQ rotation matrix.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform f64 in [0, 1).
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa worth of randomness.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard-normal sample via Box–Muller (deterministic).
    #[inline]
    pub fn next_gaussian(&mut self) -> f64 {
        // Guard u1 away from 0 so ln() is finite.
        let u1 = (self.next_f64()).max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// xorshift128+ — fast deterministic stream for k-means mini-batch index
/// sampling. Seeded (indirectly) from a fixed constant via [`SplitMix64`].
#[derive(Clone, Debug)]
pub struct Xorshift128 {
    s0: u64,
    s1: u64,
}

impl Xorshift128 {
    pub fn new(seed: u64) -> Self {
        // Expand the seed through SplitMix64 so a zero seed still gives a
        // non-degenerate state.
        let mut sm = SplitMix64::new(seed);
        let s0 = sm.next_u64() | 1;
        let s1 = sm.next_u64() | 1;
        Self { s0, s1 }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.s0;
        let y = self.s1;
        self.s0 = y;
        x ^= x << 23;
        x ^= x >> 17;
        x ^= y ^ (y >> 26);
        self.s1 = x;
        x.wrapping_add(y)
    }

    /// Uniform integer in `[0, n)`. `n` must be > 0.
    #[inline]
    pub fn next_below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic() {
        let a: Vec<u64> = (0..8)
            .scan(SplitMix64::new(42), |s, _| Some(s.next_u64()))
            .collect();
        let b: Vec<u64> = (0..8)
            .scan(SplitMix64::new(42), |s, _| Some(s.next_u64()))
            .collect();
        assert_eq!(a, b);
        // Different seed → different stream.
        let c: Vec<u64> = (0..8)
            .scan(SplitMix64::new(43), |s, _| Some(s.next_u64()))
            .collect();
        assert_ne!(a, c);
    }

    #[test]
    fn xorshift_below_in_range() {
        let mut r = Xorshift128::new(7);
        for _ in 0..1000 {
            assert!(r.next_below(10) < 10);
        }
    }

    #[test]
    fn gaussian_finite() {
        let mut r = SplitMix64::new(1);
        for _ in 0..1000 {
            assert!(r.next_gaussian().is_finite());
        }
    }
}
