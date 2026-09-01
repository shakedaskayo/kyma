//! In-partition candidate scan.
//!
//! Given a query already prepared for the RaBitQ estimator (see
//! [`crate::rabitq::encode_query_rotated`]) and a decoded partition (the
//! posting-list entries from [`crate::file::read_partition`]), produce
//! `(RowAddress, approx_distance)` candidates. The approximate distance is the
//! RaBitQ popcount estimate of cosine distance; smaller is nearer.
//!
//! This is the build-side primitive the Part B `ann_topk` query path composes:
//! probe → scan each probed partition → collect candidates → exact-rerank the
//! top `rerank_factor·k`. Part B owns the rerank; here we only rank.

use crate::file::Entry;
use crate::rabitq::{estimate_distance, QueryCode};
use pensieve_core::index_sidecar::RowAddress;

/// Estimate the cosine distance from `query` to every entry in `partition`,
/// returning `(address, approx_distance)` pairs in the partition's stored
/// order. The caller merges results across probed partitions and reranks.
pub fn scan_partition(query: &QueryCode, partition: &[Entry]) -> Vec<(RowAddress, f32)> {
    partition
        .iter()
        .map(|e| (e.addr, estimate_distance(&e.code, query)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::Entry;
    use crate::rabitq::{encode_query_rotated, encode_rotated, Rotation};
    use crate::rng::SplitMix64;
    use pensieve_core::segment_format::BlockId;

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
    fn scan_returns_one_per_entry_and_ranks_self_best() {
        let dim = 32;
        let rot = Rotation::new(dim);
        let mut rng = SplitMix64::new(8);
        let q = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
        let qc = encode_query_rotated(&rot.apply(&q));

        let mut partition = Vec::new();
        // Entry 0 is the query itself; the rest are random.
        partition.push(Entry {
            addr: RowAddress {
                block: BlockId(0),
                row: 0,
            },
            code: encode_rotated(&rot.apply(&q)),
        });
        for row in 1..10u32 {
            let v = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
            partition.push(Entry {
                addr: RowAddress {
                    block: BlockId(0),
                    row,
                },
                code: encode_rotated(&rot.apply(&v)),
            });
        }
        let scored = scan_partition(&qc, &partition);
        assert_eq!(scored.len(), 10);
        // The self entry should be the minimum-distance candidate.
        let best = scored
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        assert_eq!(best.0.row, 0);
    }
}
