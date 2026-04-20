//! HTTP handler for `GET /v1/catalog/schema`.
//!
//! Returns the full schema tree (databases → tables → columns) as JSON,
//! with a 5-second server-side cache backed by a `tokio::sync::Mutex`.

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

const TTL: Duration = Duration::from_secs(5);

/// Server-side cache for the schema document.
///
/// `None` means the cache is cold; `Some((timestamp, doc))` holds the last
/// fetched document. Stale entries (older than `TTL`) are evicted on the next
/// read.
#[derive(Default)]
pub struct SchemaCache {
    inner: Mutex<Option<(Instant, Arc<SchemaDoc>)>>,
}

impl SchemaCache {
    pub fn new() -> Self {
        Self::default()
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
    let mut guard = cache.inner.lock().await;
    if let Some((t, doc)) = guard.as_ref() {
        if t.elapsed() < TTL {
            return (StatusCode::OK, Json((**doc).clone())).into_response();
        }
    }

    let doc = match build(&*state.catalog).await {
        Ok(d) => Arc::new(d),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":{"code":"catalog","message": e.to_string()}})),
            )
                .into_response();
        }
    };
    *guard = Some((Instant::now(), doc.clone()));
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
