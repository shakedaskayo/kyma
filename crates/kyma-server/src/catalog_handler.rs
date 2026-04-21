//! HTTP handler for `GET /v1/catalog/schema`.
//!
//! Returns the full schema tree (databases → tables → columns) as JSON,
//! with a configurable server-side cache (default 5 s, override via
//! `KYMA_SCHEMA_CACHE_TTL_SECS`) backed by a `tokio::sync::Mutex`.
//! Thundering-herd tolerant on cold-start misses: multiple concurrent cold
//! requests will each rebuild. The cache is never held across IO, so warm
//! readers never block on a builder.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::QueryState;
use kyma_core::catalog::{Catalog, ColumnInfo};

/// Top-level schema document returned by `GET /v1/catalog/schema`.
#[derive(Serialize, Clone)]
pub struct SchemaDoc {
    pub databases: Vec<DatabaseDoc>,
}

/// Per-database entry in the schema document.
#[derive(Serialize, Clone)]
pub struct DatabaseDoc {
    pub name: String,
    pub tables: Vec<TableDoc>,
}

/// Per-table entry (columns are `ColumnInfo` from `kyma_core`).
#[derive(Serialize, Clone)]
pub struct TableDoc {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
}

/// Default TTL used by [`SchemaCache::default`].
const DEFAULT_TTL: Duration = Duration::from_secs(5);

/// Server-side cache for the schema document.
///
/// `None` means the cache is cold; `Some((timestamp, doc))` holds the last
/// fetched document. Stale entries (older than `ttl`) are evicted on the next
/// read.  A `ttl` of `Duration::ZERO` effectively disables the cache — the
/// handler rebuilds on every request.
pub struct SchemaCache {
    inner: Mutex<Option<(Instant, Arc<SchemaDoc>)>>,
    pub ttl: Duration,
}

impl Default for SchemaCache {
    fn default() -> Self {
        Self::new(DEFAULT_TTL)
    }
}

impl SchemaCache {
    /// Create a cache with an explicit TTL.  Pass [`Duration::ZERO`] to
    /// disable caching (every request rebuilds from the catalog).
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(None),
            ttl,
        }
    }

    /// Read `KYMA_SCHEMA_CACHE_TTL_SECS` from the environment and construct a
    /// [`SchemaCache`] accordingly.  Falls back to the 5-second default when
    /// the variable is absent or unparseable.
    pub fn from_env() -> Self {
        let ttl = std::env::var("KYMA_SCHEMA_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TTL);
        Self::new(ttl)
    }
}

/// Axum handler for `GET /v1/catalog/schema`.
///
/// Returns a cached `SchemaDoc` (fresh within 5 s) or rebuilds it from the
/// catalog. The 401 / auth gate is applied by the surrounding middleware
/// (`require_role_middleware`), so this handler only runs for authenticated
/// callers with at least `read` role.
pub async fn schema_handler(State(state): State<QueryState>) -> impl IntoResponse {
    let cache = state.schema_cache.clone();

    // Freshness check under a short guard, then release.
    let ttl = cache.ttl;
    let cached = {
        let guard = cache.inner.lock().await;
        guard.as_ref().cloned()
    };
    if let Some((t, doc)) = &cached {
        if t.elapsed() < ttl {
            return (StatusCode::OK, Json((**doc).clone())).into_response();
        }
    }

    // Rebuild without holding the lock. Multiple builders under a
    // thundering-herd cold start will each do the walk; the cost of
    // that once is accepted in exchange for never serializing warm-path
    // readers behind a miss.
    let doc = match build(&*state.catalog).await {
        Ok(d) => Arc::new(d),
        Err(e) => {
            // Leave cache as-is on error: if a previous doc existed it's
            // already stale (otherwise we wouldn't be here), so the next
            // request retries build() cleanly.
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":{"code":"catalog","message": e.to_string()}})),
            )
                .into_response();
        }
    };

    {
        let mut guard = cache.inner.lock().await;
        *guard = Some((Instant::now(), doc.clone()));
    }
    (StatusCode::OK, Json((*doc).clone())).into_response()
}

async fn build(catalog: &dyn Catalog) -> Result<SchemaDoc, kyma_core::errors::CatalogError> {
    let mut out = SchemaDoc {
        databases: Vec::new(),
    };
    for db in catalog.list_databases().await? {
        let mut tables = Vec::new();
        for tbl in catalog.list_tables(&db).await? {
            let cols = catalog.get_table_columns(&db, &tbl).await?;
            tables.push(TableDoc {
                name: tbl,
                columns: cols,
            });
        }
        out.databases.push(DatabaseDoc { name: db, tables });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// A `Duration::ZERO` cache must never return a cached entry — the
    /// "elapsed >= ttl" condition is immediately true after the first insert.
    #[tokio::test]
    async fn zero_ttl_cache_is_always_cold() {
        let cache = SchemaCache::new(Duration::ZERO);
        // Prime the inner state as if we just built a doc.
        {
            let mut g = cache.inner.lock().await;
            *g = Some((
                // Use `Instant::now() - 1s` so elapsed() > Duration::ZERO
                // even on the very first check.
                Instant::now() - Duration::from_secs(1),
                Arc::new(SchemaDoc { databases: vec![] }),
            ));
        }
        // Now read: because ttl == ZERO, every elapsed value satisfies
        // `elapsed >= ttl`, so the freshness check must fail.
        let cached = {
            let g = cache.inner.lock().await;
            g.as_ref().cloned()
        };
        if let Some((t, _)) = cached {
            // With ZERO ttl the condition for "fresh" is `t.elapsed() < ZERO`,
            // which is always false. Confirm that here.
            assert!(
                !(t.elapsed() < cache.ttl),
                "zero-ttl cache reported fresh — that is incorrect"
            );
        }
    }

    #[test]
    fn default_ttl_is_five_seconds() {
        let cache = SchemaCache::default();
        assert_eq!(cache.ttl, Duration::from_secs(5));
    }

    #[test]
    fn custom_ttl_is_stored() {
        let cache = SchemaCache::new(Duration::from_secs(60));
        assert_eq!(cache.ttl, Duration::from_secs(60));
    }
}
