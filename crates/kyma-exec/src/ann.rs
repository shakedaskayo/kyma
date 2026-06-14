//! `ann_topk` — the ANN query path over per-extent IVF + 1-bit RaBitQ sidecars.
//!
//! This is S1.1 Part B's query side. Part A built the sidecars (one immutable
//! IVF+RaBitQ blob per extent, stored next to the extent under
//! `{tenant}/indexes/{extent}/{column}.ivf_rabitq`; see
//! [`kyma_index_vector`] and [`kyma_core::index_sidecar`]). Here we read them.
//!
//! For a query embedding `qvec` and a vector column, [`ann_topk`]:
//!
//! 1. **L2-normalizes** `qvec` to the unit sphere — matching the writer's /
//!    builder's normalization exactly (cosine on unit vectors = `1 − ⟨a,b⟩`).
//! 2. **Lists live extents** for the table, applying the caller's `prefilter`
//!    predicates (time-range / equality) through the catalog. We do **not**
//!    apply the centroid+radius `VectorDistance` prune here: we don't know a
//!    distance threshold a priori, and skipping a far extent by a loose bound
//!    risks dropping a true top-k row. Per-extent IVF probing already restricts
//!    the work to the nearest partitions, so unpruned extents are cheap.
//! 3. For each surviving extent, looks up its `IvfRabitq` sidecar:
//!    - **sidecar present** → fetch via the [`SidecarCache`], probe the IVF for
//!      the `nprobe` nearest partitions, scan them with the RaBitQ estimator,
//!      and collect `(extent, RowAddress, approx_dist)` candidates;
//!    - **no sidecar** → *fallback*: read the whole extent's vector column and
//!      compute exact cosine for every row. These rows enter the pool already
//!      exact-scored, so an un-indexed extent never loses recall.
//! 4. Keeps the global top `rerank_factor·k` sidecar candidates by approximate
//!    distance, then **exact-reranks** them: group by `(extent, block)`, read
//!    each block once, pull the vector at `addr.row`, compute exact cosine via
//!    [`crate::udfs_vector::cosine_distance_vec`]. Fallback rows are already
//!    exact and merge in directly.
//! 5. Returns the global top-`k` by exact cosine distance.
//!
//! The probe→scan→rerank recipe mirrors the validated sequence in
//! `kyma-index-vector/tests/recall.rs` (recall@10 = 0.976 at dim=384).

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch};
use kyma_core::catalog::{Catalog, ExtentManifest, PrunePredicate, TableRef};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, RowAddress, SidecarKind};
use kyma_core::segment_format::{BlockId, BlockPredicate, OpenExtentInput, SegmentFormat};
use kyma_core::tenant::TenantId;
use kyma_core::types::ExtentId;
use kyma_index_vector::{encode_query_rotated, file, read_header, scan_partition, Rotation};
use kyma_storage::sidecar_cache::{SidecarCache, SidecarCacheKey};
use object_store::ObjectStore;

use crate::udfs_vector::cosine_distance_vec;

/// Tunable knobs for the ANN query path. Defaults match the validated recall
/// sweep (`kyma-index-vector/tests/recall.rs`): `nprobe = 16`,
/// `rerank_factor = 48`.
#[derive(Debug, Clone, Copy)]
pub struct AnnParams {
    /// Top-k to return.
    pub k: usize,
    /// IVF partitions to probe per extent (scaled up for large `nlist`).
    pub nprobe: usize,
    /// Exact-rerank pool size = `rerank_factor · k` candidates.
    pub rerank_factor: usize,
    /// Filtered-search planner threshold (S1.2). When the caller's `prefilter`
    /// prunes the table to at most this many surviving rows (summed over the
    /// surviving extents' row counts), the query exact-scans the whole filtered
    /// set instead of probing sidecars. Two reasons: (1) over a small set,
    /// brute-force is cheaper than IVF probe + rerank, and (2) a selective
    /// filter leaves so few vectors that approximate probing risks dropping
    /// true neighbours — exact scan is recall-1.0 by construction. Above the
    /// threshold (broad / unfiltered queries) the ANN path runs. `0` disables
    /// the exact branch (always ANN where a sidecar exists).
    pub exact_scan_max_rows: u64,
}

impl AnnParams {
    /// `k` results with the validated `nprobe`/`rerank_factor` defaults.
    pub fn with_k(k: usize) -> Self {
        Self {
            k,
            nprobe: 16,
            rerank_factor: 48,
            exact_scan_max_rows: 50_000,
        }
    }
}

impl Default for AnnParams {
    fn default() -> Self {
        Self::with_k(10)
    }
}

/// One ANN hit: the row's extent address and its **exact** cosine distance.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnHit {
    pub extent_id: ExtentId,
    pub addr: RowAddress,
    /// Exact cosine distance (`1 − cos`), range `[0, 2]`. Smaller is nearer.
    pub distance: f64,
}

/// Error type for the ANN query path. Most failures degrade rather than abort
/// (a missing sidecar falls back to exact scan), so this only surfaces
/// catalog / object-store / format I/O errors the caller can't recover from.
#[derive(Debug, thiserror::Error)]
pub enum AnnError {
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("format: {0}")]
    Format(String),
    #[error("sidecar decode: {0}")]
    Sidecar(String),
}

type Result<T> = std::result::Result<T, AnnError>;

/// A candidate surfaced by the IVF probe of a sidecar-indexed extent, scored
/// only by the (approximate) RaBitQ estimator. The exact rerank fetches the
/// real vector and recomputes cosine.
#[derive(Clone)]
struct ApproxCand {
    extent_id: ExtentId,
    addr: RowAddress,
    approx_dist: f32,
}

/// L2-normalize a vector to the unit sphere. Matches the builder/writer:
/// accumulate the squared norm in f64, and leave a (near-)zero vector
/// untouched (it has no meaningful direction; cosine to it is undefined).
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= 1e-12 {
        return v.to_vec();
    }
    v.iter().map(|x| (*x as f64 / norm) as f32).collect()
}

/// Cache of the deterministic dim×dim rotation matrix, keyed by `dim`. The
/// rotation depends only on `dim` (fixed seed), so we build it once and reuse
/// it across every extent of the same width — avoiding an O(dim²) rebuild per
/// extent.
#[derive(Default)]
struct RotationCache {
    by_dim: HashMap<usize, Arc<Rotation>>,
}

impl RotationCache {
    fn get(&mut self, dim: usize) -> Arc<Rotation> {
        self.by_dim
            .entry(dim)
            .or_insert_with(|| Arc::new(Rotation::new(dim)))
            .clone()
    }
}

/// Run the ANN top-k query over the table's per-extent IVF+RaBitQ sidecars,
/// falling back to exact cosine scan for any extent that has no sidecar.
///
/// `prefilter` is an optional, already-built [`PrunePredicate`] (time-range /
/// equality / in-set) applied at the catalog level before any sidecar or block
/// I/O. Pass `None` for an unfiltered ANN. Any `VectorDistance` entry in the
/// predicate is ignored here — IVF probing supersedes it.
///
/// Returns the exact top-`params.k` rows by cosine distance, nearest first.
#[allow(clippy::too_many_arguments)]
pub async fn ann_topk(
    catalog: &Arc<dyn Catalog>,
    tenant: TenantId,
    format: &Arc<dyn SegmentFormat>,
    store: &Arc<dyn ObjectStore>,
    sidecar_cache: &SidecarCache,
    table: &TableRef,
    column: &str,
    qvec: &[f32],
    params: &AnnParams,
    prefilter: Option<&PrunePredicate>,
) -> Result<Vec<AnnHit>> {
    // (a) Normalize the query exactly as rows were normalized on write.
    let q = l2_normalize(qvec);

    // (b) List live extents, applying the caller's prefilter predicates.
    let prune = sanitize_prune(prefilter);
    let extents = catalog
        .list_extents_in_tenant(tenant, table.id, table.current_snapshot_id, &prune)
        .await
        .map_err(|e| AnnError::Catalog(e.to_string()))?;

    if extents.is_empty() {
        return Ok(Vec::new());
    }

    // (c) Filtered-search planner (S1.2): if the prefilter pruned the table to a
    //     small surviving set, exact-scan it and skip sidecars entirely. The
    //     row-count sum over surviving extents is an upper bound on the
    //     candidates to scan; when even that bound is small, brute-force is both
    //     cheaper than ANN and exactly correct (no approximate recall loss on a
    //     heavily filtered set). A predicate-free query has every extent survive
    //     and the sum is the full table, so this only triggers for selective
    //     filters.
    let surviving_rows: u64 = extents.iter().map(|m| m.row_count).sum();
    let force_exact = plan_force_exact(
        prefilter.is_some(),
        surviving_rows,
        params.exact_scan_max_rows,
    );

    // Which extents carry an IvfRabitq sidecar for this column? (Skipped when
    // the planner chose the exact branch.)
    let sidecar_by_extent: HashMap<ExtentId, IndexSidecarDescriptor> = if force_exact {
        HashMap::new()
    } else {
        let extent_ids: Vec<ExtentId> = extents.iter().map(|m| m.id).collect();
        let sidecars = catalog
            .list_index_sidecars(tenant, table.id, &extent_ids, Some(SidecarKind::IvfRabitq))
            .await
            .map_err(|e| AnnError::Catalog(e.to_string()))?;
        sidecars
            .into_iter()
            .filter(|d| d.column == column)
            .map(|d| (d.extent_id, d))
            .collect()
    };

    let manifest_by_extent: HashMap<ExtentId, &ExtentManifest> =
        extents.iter().map(|m| (m.id, m)).collect();

    let mut rotations = RotationCache::default();
    let mut approx: Vec<ApproxCand> = Vec::new();
    // Fallback rows are scored exactly during the scan, so they enter the final
    // pool directly without going through the approximate-pool truncation.
    let mut fallback_exact: Vec<AnnHit> = Vec::new();

    for manifest in &extents {
        match sidecar_by_extent.get(&manifest.id) {
            Some(desc) => {
                // ---- sidecar path: fetch → probe → scan ----
                let key = SidecarCacheKey {
                    extent_id: manifest.id,
                    column: column.to_string(),
                    kind: SidecarKind::IvfRabitq,
                    model_id: desc.embedding_model_id.clone(),
                };
                let local_path = sidecar_cache
                    .get_or_fetch(store, &desc.object_path, &key)
                    .await
                    .map_err(|e| AnnError::Storage(e.to_string()))?;
                let bytes = std::fs::read(&local_path)
                    .map_err(|e| AnnError::Storage(format!("read cached sidecar: {e}")))?;
                let header = read_header(&bytes).map_err(|e| AnnError::Sidecar(e.to_string()))?;

                let dim = header.dim as usize;
                if q.len() != dim {
                    // Query/index dim mismatch (wrong column / stale model) —
                    // skip this extent rather than abort the whole query.
                    continue;
                }
                let rotation = rotations.get(dim);
                let qc = encode_query_rotated(&rotation.apply(&q));

                // Probe a healthy fraction of the partitions; scale with nlist.
                let nprobe = effective_nprobe(params.nprobe, header.nlist as usize);
                let probed = file::probe(&bytes, &header, &q, nprobe)
                    .map_err(|e| AnnError::Sidecar(e.to_string()))?;
                for p in probed {
                    let part = file::read_partition(&bytes, &header, p)
                        .map_err(|e| AnnError::Sidecar(e.to_string()))?;
                    for (addr, approx_dist) in scan_partition(&qc, &part) {
                        approx.push(ApproxCand {
                            extent_id: manifest.id,
                            addr,
                            approx_dist,
                        });
                    }
                }
            }
            None => {
                // ---- fallback: exact-scan the whole extent's vector column ----
                let reader = format
                    .open_extent(OpenExtentInput {
                        extent_id: manifest.id,
                        table_id: manifest.table_id,
                        schema: table.schema.clone(),
                        object_path: manifest.object_path.clone(),
                        byte_size: manifest.byte_size,
                    })
                    .await
                    .map_err(|e| AnnError::Format(e.to_string()))?;
                let blocks = reader
                    .pruned_blocks(&BlockPredicate::All)
                    .await
                    .map_err(|e| AnnError::Format(e.to_string()))?;
                for bid in blocks {
                    let batch = reader
                        .read_block(bid, &[])
                        .await
                        .map_err(|e| AnnError::Format(e.to_string()))?;
                    let Some(col) = column_vectors(&batch, column) else {
                        continue;
                    };
                    for (row, vec) in col {
                        if let Some(dist) = cosine_distance_vec(&q, &vec) {
                            fallback_exact.push(AnnHit {
                                extent_id: manifest.id,
                                addr: RowAddress {
                                    block: bid,
                                    row: row as u32,
                                },
                                distance: dist,
                            });
                        }
                    }
                }
            }
        }
    }

    // (d) Keep the global top (rerank_factor · k) sidecar candidates by the
    //     approximate estimator. Fallback rows are already exact — kept whole.
    let pool = truncate_pool(
        approx,
        params.rerank_factor.saturating_mul(params.k).max(params.k),
    );

    // (e) Exact rerank the sidecar pool: group by (extent, block), read each
    //     block once, extract the vector at addr.row, compute exact cosine.
    let reranked = exact_rerank(format, table, &q, column, &manifest_by_extent, pool).await?;

    // Merge sidecar-reranked + fallback-exact, take the global top-k.
    let mut all: Vec<AnnHit> = reranked;
    all.extend(fallback_exact);
    sort_hits(&mut all);
    all.truncate(params.k);
    Ok(all)
}

/// Sort hits by ascending exact distance with a deterministic tie-break on
/// `(extent_id, block, row)`.
fn sort_hits(hits: &mut [AnnHit]) {
    hits.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.extent_id.to_string().cmp(&b.extent_id.to_string()))
            .then_with(|| a.addr.block.0.cmp(&b.addr.block.0))
            .then_with(|| a.addr.row.cmp(&b.addr.row))
    });
}

/// Filtered-search planner decision (S1.2): take the exact-scan branch when a
/// prefilter is present AND it pruned the table to at most `max_rows` surviving
/// rows. `max_rows == 0` disables the exact branch. Pure function — unit-tested.
fn plan_force_exact(has_prefilter: bool, surviving_rows: u64, max_rows: u64) -> bool {
    has_prefilter && max_rows > 0 && surviving_rows <= max_rows
}

/// Keep the `pool_size` nearest approximate candidates (ascending estimator
/// distance). Pure function — unit-tested.
fn truncate_pool(mut approx: Vec<ApproxCand>, pool_size: usize) -> Vec<ApproxCand> {
    approx.sort_by(|a, b| {
        a.approx_dist
            .partial_cmp(&b.approx_dist)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    approx.truncate(pool_size);
    approx
}

/// Fetch each sidecar candidate's true vector (reading each `(extent, block)`
/// once) and recompute exact cosine distance.
async fn exact_rerank(
    format: &Arc<dyn SegmentFormat>,
    table: &TableRef,
    q: &[f32],
    column: &str,
    manifests: &HashMap<ExtentId, &ExtentManifest>,
    cands: Vec<ApproxCand>,
) -> Result<Vec<AnnHit>> {
    // Group wanted rows by (extent, block).
    let mut by_extent_block: HashMap<(ExtentId, u32), Vec<u32>> = HashMap::new();
    for c in &cands {
        by_extent_block
            .entry((c.extent_id, c.addr.block.0))
            .or_default()
            .push(c.addr.row);
    }

    // Cache one opened reader per extent so multiple probed blocks of the same
    // extent share a single `open_extent`.
    let mut readers: HashMap<ExtentId, Arc<dyn kyma_core::segment_format::ExtentReader>> =
        HashMap::new();
    let mut out: Vec<AnnHit> = Vec::with_capacity(cands.len());

    // Deterministic block iteration order keeps the result reproducible.
    let mut keys: Vec<(ExtentId, u32)> = by_extent_block.keys().copied().collect();
    keys.sort_by(|a, b| {
        a.0.to_string()
            .cmp(&b.0.to_string())
            .then_with(|| a.1.cmp(&b.1))
    });

    for (extent_id, block) in keys {
        let Some(manifest) = manifests.get(&extent_id) else {
            continue;
        };
        let reader = match readers.get(&extent_id) {
            Some(r) => r.clone(),
            None => {
                let r = format
                    .open_extent(OpenExtentInput {
                        extent_id,
                        table_id: manifest.table_id,
                        schema: table.schema.clone(),
                        object_path: manifest.object_path.clone(),
                        byte_size: manifest.byte_size,
                    })
                    .await
                    .map_err(|e| AnnError::Format(e.to_string()))?;
                readers.insert(extent_id, r.clone());
                r
            }
        };
        let batch = reader
            .read_block(BlockId(block), &[])
            .await
            .map_err(|e| AnnError::Format(e.to_string()))?;
        let Some(vectors) = column_vectors_indexed(&batch, column) else {
            continue;
        };
        for &row in &by_extent_block[&(extent_id, block)] {
            let Some(vec) = vectors.get(row as usize).and_then(|v| v.as_ref()) else {
                continue;
            };
            if let Some(dist) = cosine_distance_vec(q, vec) {
                out.push(AnnHit {
                    extent_id,
                    addr: RowAddress {
                        block: BlockId(block),
                        row,
                    },
                    distance: dist,
                });
            }
        }
    }
    Ok(out)
}

/// Decide the effective `nprobe`: never more than `nlist`, and scale up a bit
/// for large partition counts so recall holds as the corpus grows. Pure
/// function — unit-tested.
fn effective_nprobe(base: usize, nlist: usize) -> usize {
    if nlist == 0 {
        return 0;
    }
    // The recall gate used nprobe=16 at nlist≈71. Probe ~max(base, ⌈nlist/4⌉)
    // partitions, clamped to nlist, so larger indexes get proportional probing.
    base.max(nlist.div_ceil(4)).min(nlist).max(1)
}

/// Extract `(row_in_block, normalized_vector)` for every non-null vector in the
/// named `FixedSizeList<Float32>` column of `batch`. Returns `None` if the
/// column is absent or the wrong type (the caller skips the block).
fn column_vectors(batch: &RecordBatch, column: &str) -> Option<Vec<(usize, Vec<f32>)>> {
    let indexed = column_vectors_indexed(batch, column)?;
    Some(
        indexed
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|vec| (i, vec)))
            .collect(),
    )
}

/// Like [`column_vectors`] but returns a dense `Vec` indexed by row, with
/// `None` for null/absent rows — so the rerank can index directly by
/// `addr.row`. Vectors are L2-normalized.
fn column_vectors_indexed(batch: &RecordBatch, column: &str) -> Option<Vec<Option<Vec<f32>>>> {
    let idx = batch.schema().index_of(column).ok()?;
    let col = batch.column(idx);
    let arr = col.as_any().downcast_ref::<FixedSizeListArray>()?;
    let floats = arr.values().as_any().downcast_ref::<Float32Array>()?;
    let dim = arr.value_length() as usize;
    if dim == 0 {
        return None;
    }
    let mut out: Vec<Option<Vec<f32>>> = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        if arr.is_null(i) {
            out.push(None);
            continue;
        }
        let start = i * dim;
        let raw: Vec<f32> = (0..dim).map(|j| floats.value(start + j)).collect();
        out.push(Some(l2_normalize(&raw)));
    }
    Some(out)
}

/// Strip any `VectorDistance` entries from the caller's prefilter — ANN probing
/// supersedes the centroid+radius prune — and return an owned [`PrunePredicate`]
/// for the catalog listing.
fn sanitize_prune(prefilter: Option<&PrunePredicate>) -> PrunePredicate {
    let Some(p) = prefilter else {
        return PrunePredicate::default();
    };
    let column_predicates = p
        .column_predicates
        .iter()
        .filter(|(_, v)| !matches!(v, kyma_core::catalog::ColumnPrune::VectorDistance { .. }))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    PrunePredicate {
        time_range: p.time_range.clone(),
        required_paths: p.required_paths.clone(),
        column_predicates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kyma_core::segment_format::BlockId;

    #[test]
    fn l2_normalize_makes_unit_and_preserves_zero() {
        let v = l2_normalize(&[3.0, 4.0]);
        let n = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((n - 1.0).abs() < 1e-6, "norm={n}");
        // Zero vector left untouched (no meaningful direction).
        assert_eq!(l2_normalize(&[0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn rotation_cache_returns_same_arc_per_dim() {
        let mut c = RotationCache::default();
        let a = c.get(16);
        let b = c.get(16);
        assert!(Arc::ptr_eq(&a, &b), "same dim must reuse one Rotation");
        let d = c.get(32);
        assert!(
            !Arc::ptr_eq(&a, &d),
            "different dim is a different Rotation"
        );
    }

    #[test]
    fn plan_force_exact_only_for_selective_filtered_queries() {
        // Selective filter (few surviving rows) → exact scan.
        assert!(plan_force_exact(true, 1_000, 50_000));
        assert!(plan_force_exact(true, 50_000, 50_000)); // boundary inclusive
                                                         // Broad/unfiltered: every extent survives → too many rows → ANN path.
        assert!(!plan_force_exact(true, 50_001, 50_000));
        // No prefilter → always ANN, regardless of size.
        assert!(!plan_force_exact(false, 10, 50_000));
        // Threshold 0 disables the exact branch.
        assert!(!plan_force_exact(true, 1, 0));
    }

    #[test]
    fn effective_nprobe_clamps_and_scales() {
        assert_eq!(effective_nprobe(16, 0), 0);
        // base wins when nlist is small.
        assert_eq!(effective_nprobe(16, 8), 8); // clamped to nlist
        assert_eq!(effective_nprobe(16, 40), 16); // ceil(40/4)=10 < 16 → base
        assert_eq!(effective_nprobe(16, 200), 50); // ceil(200/4)=50 > 16
        assert_eq!(effective_nprobe(4, 4), 4);
    }

    #[test]
    fn truncate_pool_keeps_nearest_by_approx() {
        let e = ExtentId::new();
        let mk = |row: u32, d: f32| ApproxCand {
            extent_id: e,
            addr: RowAddress {
                block: BlockId(0),
                row,
            },
            approx_dist: d,
        };
        let pool = truncate_pool(vec![mk(0, 0.9), mk(1, 0.1), mk(2, 0.5), mk(3, 0.3)], 2);
        assert_eq!(pool.len(), 2);
        // The two smallest approx distances (0.1, 0.3) survive.
        let rows: Vec<u32> = pool.iter().map(|c| c.addr.row).collect();
        assert_eq!(rows, vec![1, 3]);
    }

    #[test]
    fn sort_hits_orders_by_distance_then_address() {
        let e = ExtentId::new();
        let mut hits = vec![
            AnnHit {
                extent_id: e,
                addr: RowAddress {
                    block: BlockId(0),
                    row: 5,
                },
                distance: 0.5,
            },
            AnnHit {
                extent_id: e,
                addr: RowAddress {
                    block: BlockId(0),
                    row: 1,
                },
                distance: 0.2,
            },
        ];
        sort_hits(&mut hits);
        assert_eq!(hits[0].distance, 0.2);
        assert_eq!(hits[1].distance, 0.5);
    }

    #[test]
    fn sanitize_prune_drops_vector_distance_keeps_others() {
        use kyma_core::catalog::ColumnPrune;
        let mut cp = HashMap::new();
        cp.insert(
            "embedding".to_string(),
            ColumnPrune::VectorDistance {
                query: vec![1.0, 0.0],
                threshold: 0.5,
            },
        );
        cp.insert(
            "service".to_string(),
            ColumnPrune::Equals(serde_json::json!("api")),
        );
        let p = PrunePredicate {
            time_range: None,
            required_paths: vec![],
            column_predicates: cp,
        };
        let out = sanitize_prune(Some(&p));
        assert!(!out.column_predicates.contains_key("embedding"));
        assert!(out.column_predicates.contains_key("service"));
    }
}
