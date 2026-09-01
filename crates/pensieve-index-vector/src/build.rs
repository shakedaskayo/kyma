//! End-to-end index construction: rows → IVF clustering → RaBitQ codes →
//! serialized sidecar.
//!
//! [`build_ivf_rabitq`] is the pure core the [`crate::builder::IvfRabitqBuilder`]
//! wraps. It takes already-normalized rows (each paired with its
//! [`RowAddress`]) and returns the sidecar bytes plus the effective build
//! params. It is fully deterministic for a given input.

use crate::file::{self, Entry, METRIC_COSINE};
use crate::ivf;
use crate::rabitq::{self, Rotation};
use bytes::Bytes;
use pensieve_core::index_sidecar::RowAddress;
use serde_json::{json, Value as Json};

/// Build the IVF + 1-bit RaBitQ sidecar from `rows`.
///
/// `rows` are `(address, vector)` pairs; vectors must already be L2-normalized
/// and of length `dim`. Returns the serialized sidecar bytes and the params
/// JSON (`{nlist, bits, dim, metric, rabitq_version}`).
///
/// Errors if `dim == 0`, if `rows` is empty, or if any row's length != `dim`.
pub fn build_ivf_rabitq(
    rows: &[(RowAddress, Vec<f32>)],
    dim: usize,
    model_id: Option<&str>,
) -> anyhow::Result<(Bytes, Json)> {
    if dim == 0 {
        anyhow::bail!("cannot build vector index with dim=0");
    }
    if rows.is_empty() {
        anyhow::bail!("cannot build vector index over zero rows");
    }
    for (addr, v) in rows {
        if v.len() != dim {
            anyhow::bail!(
                "row {:?} has length {} but index dim is {}",
                addr,
                v.len(),
                dim
            );
        }
    }

    // --- IVF clustering over the normalized rows. ---
    let nlist = ivf::choose_nlist(rows.len());
    let just_vecs: Vec<Vec<f32>> = rows.iter().map(|(_, v)| v.clone()).collect();
    let clustering = ivf::cluster(&just_vecs, dim, nlist);
    let nlist = clustering.centroids.len(); // cluster() may clamp to row count

    // --- RaBitQ encode each row, bucketed into its partition. ---
    let rotation = Rotation::new(dim);
    let mut partitions: Vec<Vec<Entry>> = vec![Vec::new(); nlist];
    for (i, (addr, v)) in rows.iter().enumerate() {
        let rotated = rotation.apply(v);
        let code = rabitq::encode_rotated(&rotated);
        let p = clustering.assignments[i];
        partitions[p].push(Entry { addr: *addr, code });
    }

    let bytes = file::write(
        dim,
        METRIC_COSINE,
        &clustering.centroids,
        &partitions,
        model_id,
    );

    let params = json!({
        "nlist": nlist,
        "bits": 1,
        "dim": dim,
        "metric": "cosine",
        "rabitq_version": rabitq::RABITQ_VERSION,
    });

    Ok((bytes, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::read_header;
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

    fn synth_rows(n: usize, dim: usize, seed: u64) -> Vec<(RowAddress, Vec<f32>)> {
        let mut rng = SplitMix64::new(seed);
        (0..n)
            .map(|i| {
                let v = norm((0..dim).map(|_| rng.next_gaussian() as f32).collect());
                (
                    RowAddress {
                        block: BlockId((i / 100) as u32),
                        row: (i % 100) as u32,
                    },
                    v,
                )
            })
            .collect()
    }

    #[test]
    fn build_is_deterministic() {
        let rows = synth_rows(800, 32, 1);
        let (a, pa) = build_ivf_rabitq(&rows, 32, Some("m")).unwrap();
        let (b, pb) = build_ivf_rabitq(&rows, 32, Some("m")).unwrap();
        assert_eq!(a, b);
        assert_eq!(pa, pb);
    }

    #[test]
    fn build_params_shape() {
        let rows = synth_rows(400, 16, 2);
        let (_b, params) = build_ivf_rabitq(&rows, 16, None).unwrap();
        assert_eq!(params["bits"], 1);
        assert_eq!(params["dim"], 16);
        assert_eq!(params["metric"], "cosine");
        assert_eq!(params["rabitq_version"], 1);
        assert_eq!(params["nlist"], 20); // round(sqrt(400))=20, within [16,256]
    }

    #[test]
    fn build_header_consistent() {
        let rows = synth_rows(400, 16, 3);
        let (b, params) = build_ivf_rabitq(&rows, 16, Some("fastembed/bge-small")).unwrap();
        let h = read_header(&b).unwrap();
        assert_eq!(h.dim, 16);
        assert_eq!(h.nlist as u64, params["nlist"].as_u64().unwrap());
        assert_eq!(h.model_id.as_deref(), Some("fastembed/bge-small"));
    }

    #[test]
    fn empty_rows_err() {
        assert!(build_ivf_rabitq(&[], 16, None).is_err());
    }

    #[test]
    fn dim_mismatch_err() {
        let rows = vec![(
            RowAddress {
                block: BlockId(0),
                row: 0,
            },
            vec![1.0, 0.0],
        )];
        assert!(build_ivf_rabitq(&rows, 4, None).is_err());
    }
}
