//! `bm25_topk` — the BM25 lexical query path over per-extent tantivy FTS
//! sidecars (S1.4 Part B). Part A ([`kyma_index_fts`]) built one immutable
//! tantivy index blob per extent under
//! `{tenant}/indexes/{extent}/{column}.tantivy_fts`; here we read them.
//!
//! For a free-text `query` and a text column, [`bm25_topk`]:
//!
//! 1. **Lists live extents** for the table, applying the caller's `prefilter`
//!    (time-range / equality) through the catalog.
//! 2. Looks up each surviving extent's `TantivyFts` sidecar for the column. If
//!    **none of the surviving extents are indexed**, returns `None` — the caller
//!    falls back to its existing `LIKE`/token-prune path unchanged.
//! 3. For each indexed extent: fetch the blob via the [`SidecarCache`], open the
//!    tantivy index, run a BM25 top-`k` search, and collect
//!    `(extent, RowAddress, score)`.
//! 4. Returns the global top-`k` by BM25 score, highest first.
//!
//! **Coverage note (v1):** an extent ingested but not yet indexed by the async
//! `IndexBuildExecutor` is momentarily absent from BM25 results (it is still
//! catalog-token-prunable, just not BM25-ranked); the build job converges it
//! quickly. Scores are per-extent BM25 (each sidecar computes IDF over its own
//! docs) — a global two-pass IDF is the documented v2 refinement.

use std::collections::HashMap;
use std::sync::Arc;

use kyma_core::catalog::{Catalog, PrunePredicate, TableRef};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, RowAddress, SidecarKind};
use kyma_core::tenant::TenantId;
use kyma_core::types::ExtentId;
use kyma_index_fts::{bm25_search, open_index};
use kyma_storage::sidecar_cache::{SidecarCache, SidecarCacheKey};
use object_store::ObjectStore;

/// One BM25 hit: the row's extent address and its BM25 relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct FtsHit {
    pub extent_id: ExtentId,
    pub addr: RowAddress,
    /// BM25 score (per-extent IDF). Higher is more relevant.
    pub score: f32,
}

/// Error type for the BM25 query path. Like the ANN path, most failures degrade
/// (a single unreadable sidecar is skipped) rather than abort.
#[derive(Debug, thiserror::Error)]
pub enum FtsError {
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("storage: {0}")]
    Storage(String),
    #[error("sidecar: {0}")]
    Sidecar(String),
}

type Result<T> = std::result::Result<T, FtsError>;

/// Run a BM25 top-`top_k` lexical search over the table's per-extent tantivy
/// FTS sidecars for `column`.
///
/// Returns `Ok(None)` when no surviving extent has a sidecar for the column —
/// the caller should fall back to its `LIKE`/token path. `Ok(Some(hits))`
/// otherwise (possibly empty if the query matched nothing in the indexed
/// extents).
#[allow(clippy::too_many_arguments)]
pub async fn bm25_topk(
    catalog: &Arc<dyn Catalog>,
    tenant: TenantId,
    store: &Arc<dyn ObjectStore>,
    sidecar_cache: &SidecarCache,
    table: &TableRef,
    column: &str,
    query: &str,
    top_k: usize,
    prefilter: Option<&PrunePredicate>,
) -> Result<Option<Vec<FtsHit>>> {
    if query.trim().is_empty() || top_k == 0 {
        return Ok(None);
    }

    let prune = prefilter.cloned().unwrap_or_default();
    let extents = catalog
        .list_extents_in_tenant(tenant, table.id, table.current_snapshot_id, &prune)
        .await
        .map_err(|e| FtsError::Catalog(e.to_string()))?;
    if extents.is_empty() {
        return Ok(None);
    }

    let extent_ids: Vec<ExtentId> = extents.iter().map(|m| m.id).collect();
    let sidecars = catalog
        .list_index_sidecars(tenant, table.id, &extent_ids, Some(SidecarKind::TantivyFts))
        .await
        .map_err(|e| FtsError::Catalog(e.to_string()))?;
    let sidecar_by_extent: HashMap<ExtentId, IndexSidecarDescriptor> = sidecars
        .into_iter()
        .filter(|d| d.column == column)
        .map(|d| (d.extent_id, d))
        .collect();

    // No coverage → tell the caller to use its LIKE fallback.
    if sidecar_by_extent.is_empty() {
        return Ok(None);
    }

    let mut hits: Vec<FtsHit> = Vec::new();
    for (extent_id, desc) in &sidecar_by_extent {
        let key = SidecarCacheKey {
            extent_id: *extent_id,
            column: column.to_string(),
            kind: SidecarKind::TantivyFts,
            model_id: None,
        };
        let local_path = sidecar_cache
            .get_or_fetch(store, &desc.object_path, &key)
            .await
            .map_err(|e| FtsError::Storage(e.to_string()))?;
        let bytes = std::fs::read(&local_path)
            .map_err(|e| FtsError::Storage(format!("read cached: {e}")))?;
        // A single corrupt/unreadable sidecar must not sink the whole query.
        let index = match open_index(&bytes) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!(extent = %extent_id, error = %e, "skipping unreadable FTS sidecar");
                continue;
            }
        };
        match bm25_search(&index, query, top_k) {
            Ok(rows) => {
                for (addr, score) in rows {
                    hits.push(FtsHit {
                        extent_id: *extent_id,
                        addr,
                        score,
                    });
                }
            }
            Err(e) => {
                tracing::warn!(extent = %extent_id, error = %e, "FTS search failed for extent");
            }
        }
    }

    // Global top-k by BM25 score (descending), deterministic tie-break.
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.extent_id.to_string().cmp(&b.extent_id.to_string()))
            .then_with(|| a.addr.block.0.cmp(&b.addr.block.0))
            .then_with(|| a.addr.row.cmp(&b.addr.row))
    });
    hits.truncate(top_k);
    Ok(Some(hits))
}
