//! Query-side global ANN tree: load the table's tree, verify it is fresh, and
//! turn a query into a per-extent partition selection (S1.3).
//!
//! `ann_topk` calls [`select`] before its extent loop. When a fresh tree exists,
//! it returns a map `extent → [local IVF partition indices]` and the query
//! probes ONLY those — extents absent from the map are skipped entirely. When no
//! tree exists, or the tree is stale (its `extent_fingerprint` no longer matches
//! the current sidecar-bearing extents), `select` returns `None` and the caller
//! falls back to per-extent fan-out. Correctness is unchanged either way: the
//! exact rerank over whatever pool is scanned still decides the result.
//!
//! Parsed trees are cached in-process keyed by (table, column, model,
//! generation); a new generation evicts the old one for that key.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use kyma_core::catalog::Catalog;
use kyma_core::tenant::TenantId;
use kyma_core::types::{ExtentId, TableId};
use kyma_index_vector::global_tree::{self, GlobalTree};
use object_store::{path::Path as ObjPath, ObjectStore};
use uuid::Uuid;

type CacheKey = (TableId, String, Option<String>, i64);

fn cache() -> &'static Mutex<HashMap<CacheKey, Arc<GlobalTree>>> {
    static C: OnceLock<Mutex<HashMap<CacheKey, Arc<GlobalTree>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load the parsed tree for `(table, column, model, generation)`, fetching +
/// parsing the blob on a cache miss and evicting any older generation for the
/// same `(table, column, model)`. Returns `None` on any I/O / parse error
/// (caller falls back to fan-out).
async fn load_tree(
    store: &Arc<dyn ObjectStore>,
    key: &CacheKey,
    object_path: &str,
) -> Option<Arc<GlobalTree>> {
    if let Some(t) = cache().lock().ok()?.get(key).cloned() {
        return Some(t);
    }
    let bytes = store
        .get(&ObjPath::from(object_path))
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    let tree = Arc::new(GlobalTree::read(&bytes).ok()?);
    if let Ok(mut c) = cache().lock() {
        // Evict stale generations for this (table, column, model).
        c.retain(|(t, col, m, _gen), _| !(*t == key.0 && col == &key.1 && m == &key.2));
        c.insert(key.clone(), tree.clone());
    }
    Some(tree)
}

/// For a normalized query `q`, return the per-extent partition selection from a
/// fresh global tree, or `None` to fall back to per-extent fan-out.
///
/// `sidecar_extent_ids` is the set of extents that currently carry an IvfRabitq
/// sidecar for the column (the query already computed this). `model` is their
/// shared embedding model. `top_p` is the number of nearest global centroids to
/// expand.
pub async fn select(
    catalog: &Arc<dyn Catalog>,
    tenant: TenantId,
    store: &Arc<dyn ObjectStore>,
    table_id: TableId,
    column: &str,
    model: Option<&str>,
    sidecar_extent_ids: &[ExtentId],
    q: &[f32],
    top_p: usize,
) -> Option<HashMap<ExtentId, Vec<usize>>> {
    let desc = catalog
        .get_ann_tree(tenant, table_id, column, model)
        .await
        .ok()??;

    // Staleness guard: the tree must have been built from exactly the current
    // sidecar-bearing extent set. Otherwise (extent added/removed since the last
    // ann_maintain) fall back to fan-out so newly-indexed extents aren't missed.
    let current: Vec<Uuid> = sidecar_extent_ids.iter().map(|e| *e.as_uuid()).collect();
    if global_tree::extent_fingerprint(&current) != desc.extent_fingerprint {
        return None;
    }

    let key: CacheKey = (
        table_id,
        column.to_string(),
        model.map(str::to_string),
        desc.generation,
    );
    let tree = load_tree(store, &key, &desc.object_path).await?;
    if tree.dim != q.len() {
        return None; // dim mismatch (wrong model) → fan out
    }

    let refs = tree.select_partitions(q, top_p);
    if refs.is_empty() {
        return None;
    }
    let mut map: HashMap<ExtentId, Vec<usize>> = HashMap::new();
    for r in refs {
        map.entry(ExtentId::from_uuid(r.extent))
            .or_default()
            .push(r.partition as usize);
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_distinguishes_generation_and_model() {
        let t = TableId::new();
        let a: CacheKey = (t, "embedding".into(), Some("m".into()), 1);
        let b: CacheKey = (t, "embedding".into(), Some("m".into()), 2);
        let c: CacheKey = (t, "embedding".into(), Some("n".into()), 1);
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
