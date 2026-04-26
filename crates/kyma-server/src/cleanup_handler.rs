//! HTTP handler for the cleanup (hard-delete GC) endpoint.
//!
//! Exposes:
//!   POST /v1/database/:db/table/:table/cleanup?before=<RFC3339>
//!
//! Hard-deletes extents for the given `(database, table)` pair that have been
//! soft-deleted (`deleted_at IS NOT NULL`) before the given `before` timestamp.
//!
//! Returns a JSON body with aggregate counts:
//!   `{ "extents_deleted": N, "rows_freed": M, "bytes_freed": K }`
//!
//! Requires `Role::Write`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use kyma_core::catalog::{Catalog, CleanupResult};
use kyma_core::errors::{CatalogError, Error as KymaError};
use serde::Deserialize;
use std::sync::Arc;

/// Shared state for the cleanup handler — just the catalog.
#[derive(Clone)]
pub struct CleanupState {
    pub catalog: Arc<dyn Catalog>,
}

/// Query parameters for the cleanup endpoint.
#[derive(Debug, Deserialize)]
pub struct CleanupQuery {
    /// Hard-delete soft-deleted extents whose `deleted_at` is strictly before
    /// this timestamp. RFC 3339 / ISO 8601, e.g. `2025-01-01T00:00:00Z`.
    pub before: DateTime<Utc>,
}

/// `POST /v1/database/:db/table/:table/cleanup`
pub async fn cleanup_table(
    State(state): State<CleanupState>,
    Path((db, table)): Path<(String, String)>,
    Query(q): Query<CleanupQuery>,
) -> Result<Json<CleanupResult>, ApiError> {
    let result = state
        .catalog
        .cleanup_soft_deleted_extents(&db, &table, q.before)
        .await?;
    Ok(Json(result))
}

// -------------------------------------------------------------------------
// Error type
// -------------------------------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    Catalog(KymaError),
}

impl From<KymaError> for ApiError {
    fn from(e: KymaError) -> Self {
        ApiError::Catalog(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Catalog(KymaError::Catalog(CatalogError::TableNotFound {
                database,
                name,
            })) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("table '{database}'.'{name}' not found")
                })),
            )
                .into_response(),
            ApiError::Catalog(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    }
}
