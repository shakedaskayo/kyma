//! 1-bit RaBitQ quantization + popcount distance estimator.
//!
//! RaBitQ (Gao & Long, SIGMOD 2024, "RaBitQ: Quantizing High-Dimensional
//! Vectors with a Theoretical Error Bound for Approximate Nearest Neighbor
//! Search") quantizes a unit vector to a single bit per dimension while
//! retaining an *unbiased* estimator of the inner product. We implement the
//! 1-bit variant:
//!
//! 1. **Random rotation.** A fixed-seed random orthonormal-ish matrix `P`
//!    rotates every data vector and every query. Rotation spreads each
//!    coordinate's contribution evenly so the sign bits carry uniform
//!    information regardless of the raw basis. We use a Gaussian random matrix
//!    (Johnson–Lindenstrauss style); for the 1-bit estimator the matrix need
//!    not be exactly orthonormal — only shared between data and query — because
//!    the per-vector correction factor absorbs the residual scaling.
//!
//! 2. **Sign codes.** For a rotated, L2-normalized data vector `o`, the code is
//!    `x_i = sign(o_i) ∈ {-1,+1}`, packed one bit per dimension
//!    (`bit=1 ⇔ +1`). The decoded quantized vector is
//!    `x̄ = x / √dim` (unit norm).
//!
//! 3. **Correction factor.** The key RaBitQ quantity is
//!    `⟨o, x̄⟩ = (1/√dim) · Σ|o_i|` — how well the sign pattern aligns with the
//!    actual rotated vector. We store its reciprocal
//!    `corr = 1 / ⟨o, x̄⟩` per vector. Then for any query `q` (rotated, unit
//!    norm) the estimator of the true inner product `⟨o, q⟩` is:
//!
//!    `⟨o, q⟩ ≈ corr · ⟨x̄, q⟩  =  corr · (1/√dim) · Σ x_i · q_i`.
//!
//!    `Σ x_i q_i` with `x_i ∈ {±1}` is a signed sum of the query coordinates.
//!    We further binarize the query to its sign bits so the dot product becomes
//!    a popcount. With both sides quantized to `±1/√dim` unit vectors,
//!    `⟨x̄_o, x̄_q⟩ = (dim − 2·popcount(code_x XOR code_q)) / dim ∈ [-1, 1]` is
//!    the cosine between the two sign patterns, and the estimator is
//!
//!    `⟨o, q⟩ ≈ corr · ⟨x̄_o, x̄_q⟩`,
//!
//!    where `corr = 1/⟨o, x̄_o⟩ ≥ 1` rescales for the data vector's
//!    quantization angle. The popcount form makes the scan branch-free and
//!    cache-friendly. Binarizing the query drops its quantization correction
//!    (a per-query positive constant that does not change the ranking), so the
//!    estimator *ranks* candidates rather than computing exact distances — and
//!    that is exactly its contract: it surfaces the top-(rerank_factor·k)
//!    candidates, which Part B's exact rerank reorders. The recall property
//!    test validates this is good enough (recall@10 ≥ 0.95 after rerank).
//!
//! On the unit sphere, `cosine_distance = 1 − ⟨o,q⟩ = ½‖o−q‖²`, so a larger
//! estimated inner product means a smaller distance. We return an estimated
//! *distance* `1 − ip_est` so smaller is better, matching the cosine metric.

use crate::rng::SplitMix64;

/// Fixed seed for the rotation matrix. Part of the on-object contract: the
/// query side (Part B) must regenerate the identical matrix from this seed +
/// `dim`, so it must never change without a `rabitq_version` bump.
pub const ROTATION_SEED: u64 = 0x5249_4256_5141_4243; // "RIBVQABC"-ish

/// Current RaBitQ on-object format version (stored in params).
pub const RABITQ_VERSION: u32 = 1;

/// Number of bytes a `dim`-bit sign code occupies.
#[inline]
pub fn code_len(dim: usize) -> usize {
    dim.div_ceil(8)
}

/// A deterministic random rotation, materialized as a `dim × dim` row-major
/// Gaussian matrix. Regenerable from `(ROTATION_SEED, dim)` alone.
#[derive(Clone)]
pub struct Rotation {
    dim: usize,
    /// Row-major `dim × dim`.
    mat: Vec<f32>,
}

impl Rotation {
    /// Build the rotation for `dim` dimensions from the fixed seed.
    ///
    /// We draw a Gaussian matrix and orthonormalize its rows with modified
    /// Gram–Schmidt, producing a true orthonormal rotation `P` (`P·Pᵀ = I`).
    /// An exact rotation preserves L2 norms and inner products, so the rotated
    /// vectors keep their neighborhood geometry — that is what lets the sign
    /// bits carry faithful angular information. (A merely row-normalized
    /// Gaussian matrix is *not* orthonormal and badly distorts distances.)
    pub fn new(dim: usize) -> Self {
        let mut rng = SplitMix64::new(ROTATION_SEED ^ (dim as u64).wrapping_mul(0x9E37_79B9));
        // Draw rows as f64 Gaussians, then modified Gram–Schmidt in f64 for
        // numerical stability; store the result as f32.
        let mut rows: Vec<Vec<f64>> = (0..dim)
            .map(|_| (0..dim).map(|_| rng.next_gaussian()).collect())
            .collect();
        for i in 0..dim {
            // Subtract projections onto previously-finalized rows.
            for j in 0..i {
                let (prev, cur) = {
                    // Borrow split: prev = rows[j], cur = rows[i].
                    let (head, tail) = rows.split_at_mut(i);
                    (&head[j], &mut tail[0])
                };
                let dot: f64 = prev.iter().zip(cur.iter()).map(|(a, b)| a * b).sum();
                for (c, p) in cur.iter_mut().zip(prev.iter()) {
                    *c -= dot * p;
                }
            }
            // Normalize row i.
            let n = (rows[i].iter().map(|x| x * x).sum::<f64>()).sqrt();
            if n > 1e-12 {
                for x in rows[i].iter_mut() {
                    *x /= n;
                }
            }
        }
        let mut mat = vec![0.0f32; dim * dim];
        for (r, row) in rows.iter().enumerate() {
            for (c, &x) in row.iter().enumerate() {
                mat[r * dim + c] = x as f32;
            }
        }
        Self { dim, mat }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Apply the rotation: `out = P · v`. `v` must have length `dim`.
    pub fn apply(&self, v: &[f32]) -> Vec<f32> {
        debug_assert_eq!(v.len(), self.dim);
        let mut out = vec![0.0f32; self.dim];
        for r in 0..self.dim {
            let row = &self.mat[r * self.dim..(r + 1) * self.dim];
            let mut acc = 0.0f32;
            for (a, b) in row.iter().zip(v.iter()) {
                acc += a * b;
            }
            out[r] = acc;
        }
        out
    }
}

/// The 1-bit code for one data vector plus its correction factor.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorCode {
    /// Packed sign bits, LSB-first within each byte (`bit set ⇔ coordinate ≥ 0`).
    pub code: Vec<u8>,
    /// `1 / ⟨o, x̄⟩` — the per-vector RaBitQ correction factor.
    pub correction: f32,
}

/// Pack the sign bits of a rotated vector into a byte code, LSB-first.
fn pack_signs(rotated: &[f32]) -> Vec<u8> {
    let dim = rotated.len();
    let mut code = vec![0u8; code_len(dim)];
    for (i, &x) in rotated.iter().enumerate() {
        if x >= 0.0 {
            code[i / 8] |= 1 << (i % 8);
        }
    }
    code
}

/// Encode an already-rotated, L2-normalized vector into its 1-bit RaBitQ code.
///
/// `rotated` is `P · o` where `o` is the unit data vector. The correction
/// factor is `1 / ⟨o, x̄⟩` with `x̄ = sign(rotated)/√dim`, i.e.
/// `corr = √dim / Σ|rotated_i|`.
pub fn encode_rotated(rotated: &[f32]) -> VectorCode {
    let dim = rotated.len();
    let code = pack_signs(rotated);
    let abs_sum: f32 = rotated.iter().map(|x| x.abs()).sum();
    // ⟨o, x̄⟩ = (1/√dim)·Σ|rotated_i|  (since x̄_i = sign(rotated_i)/√dim and
    // rotation preserves norm, ⟨o, x̄⟩ ≈ ⟨rotated, x̄⟩ = Σ|rotated_i|/√dim).
    let dot = abs_sum / (dim as f32).sqrt();
    let correction = if dot > 1e-12 { 1.0 / dot } else { 0.0 };
    VectorCode { code, correction }
}

/// A query prepared for the asymmetric RaBitQ estimator: the full-precision
/// rotated query vector.
///
/// RaBitQ is *asymmetric* — the data vector is quantized to 1 bit/dim but the
/// query keeps full precision. This is the accurate regime: keeping the query
/// real lets the estimator weight each sign bit by the query's actual
/// magnitude in that direction, which a doubly-binarized (popcount-only)
/// estimator cannot do. The data side stays 1-bit, so the index is still
/// compact; only the per-query scan does full-precision arithmetic.
#[derive(Debug, Clone)]
pub struct QueryCode {
    /// Rotated, L2-normalized query, full precision.
    rotated: Vec<f32>,
}

/// Prepare a query for the asymmetric estimator: the caller passes the
/// already-rotated query; we retain it at full precision.
pub fn encode_query_rotated(rotated: &[f32]) -> QueryCode {
    QueryCode {
        rotated: rotated.to_vec(),
    }
}

/// Read the data sign bit at position `i` from a packed code (`true ⇔ +1`).
#[inline]
fn sign_bit(code: &[u8], i: usize) -> bool {
    (code[i / 8] >> (i % 8)) & 1 == 1
}

/// Estimate the cosine *distance* between a stored data vector (`vc`) and a
/// query (`qc`). Smaller is nearer.
///
/// Asymmetric estimator: with `x̄_o = sign(o_rotated)/√dim` the decoded unit
/// data vector,
///
/// `⟨o, q⟩ ≈ corr · ⟨x̄_o, q_rotated⟩ = corr · (1/√dim) · Σ s_i · q_i`,
///
/// where `s_i = +1` if the data sign bit is set else `-1`. We return
/// `1 − ip_est` as the cosine distance.
#[inline]
pub fn estimate_distance(vc: &VectorCode, qc: &QueryCode) -> f32 {
    let dim = qc.rotated.len();
    let mut acc = 0.0f32;
    for (i, &q) in qc.rotated.iter().enumerate() {
        if sign_bit(&vc.code, i) {
            acc += q;
        } else {
            acc -= q;
        }
    }
    let ip_est = vc.correction * acc / (dim as f32).sqrt();
    1.0 - ip_est
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
    fn rotation_is_deterministic() {
        let a = Rotation::new(32);
        let b = Rotation::new(32);
        assert_eq!(a.mat, b.mat);
    }

    #[test]
    fn code_len_rounds_up() {
        assert_eq!(code_len(8), 1);
        assert_eq!(code_len(9), 2);
        assert_eq!(code_len(64), 8);
        assert_eq!(code_len(65), 9);
    }

    #[test]
    fn identical_vectors_estimate_near_zero_distance() {
        let dim = 64;
        let rot = Rotation::new(dim);
        let mut rng = SplitMix64::new(3);
        let v = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
        let rv = rot.apply(&v);
        let vc = encode_rotated(&rv);
        let qc = encode_query_rotated(&rv);
        let d = estimate_distance(&vc, &qc);
        // Same vector → all sign bits agree → quantized_cosine = 1 → ip_est =
        // correction (≈ 1/cos(o,x̄_o) ≈ 1.0–1.3 for a well-spread unit vector),
        // so the estimated self-distance is small in magnitude. It is not
        // exactly 0 — this is a ranking estimator, not an exact distance.
        assert!(d.abs() < 0.5, "self-distance estimate too large: {d}");
    }

    #[test]
    fn nearer_vectors_estimate_smaller_distance() {
        // A vector close to the query should estimate a smaller distance than a
        // random far one, on average.
        let dim = 64;
        let rot = Rotation::new(dim);
        let mut rng = SplitMix64::new(11);
        let q = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
        let qc = encode_query_rotated(&rot.apply(&q));

        // Near: q plus small noise.
        let near = norm(
            q.iter()
                .map(|x| x + 0.05 * rng.next_gaussian() as f32)
                .collect(),
        );
        // Far: independent random.
        let far = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());

        let d_near = estimate_distance(&encode_rotated(&rot.apply(&near)), &qc);
        let d_far = estimate_distance(&encode_rotated(&rot.apply(&far)), &qc);
        assert!(d_near < d_far, "near={d_near} should be < far={d_far}");
    }
}
