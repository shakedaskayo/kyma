//! Artifact retrieval — `GET /v1/artifacts/:id`.
//!
//! Fetches a byte window of a stored object-store artifact (CI job log,
//! contributed file, fs-watch snapshot), scoped to the request's tenant via the
//! auth [`Principal`]. Range reads (`offset`/`limit`) let a UI or coding agent
//! page a large log without pulling the whole blob.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::Principal;

/// Default window when the caller doesn't specify `limit`.
const DEFAULT_LIMIT: u64 = 64 * 1024; // 64 KiB
/// Hard cap so one request can't pull an unbounded blob into memory.
const MAX_LIMIT: u64 = 4 * 1024 * 1024; // 4 MiB

#[derive(Clone)]
pub struct ArtifactsState {
    pub catalog: Arc<dyn Catalog>,
    pub store: Arc<dyn ObjectStore>,
}

#[derive(Debug, Deserialize)]
struct RangeQuery {
    #[serde(default)]
    offset: u64,
    limit: Option<u64>,
}

/// `GET /v1/artifacts/:id?offset=&limit=` — tenant-scoped byte-window read.
async fn get_artifact(
    State(state): State<ArtifactsState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> Response {
    // Artifact tracking is Postgres-only (like retention); local mode has no GC.
    let Some(pg) = state.catalog.as_ref_any().downcast_ref::<PostgresCatalog>() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "artifact retrieval requires Postgres",
        )
            .into_response();
    };

    // Tenant-scoped lookup: a caller in another tenant gets 404 even with a
    // valid id, and the object key never leaves the catalog.
    let rec = match pg.get_artifact_in_tenant(principal.tenant, id).await {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::NOT_FOUND, "artifact not found").into_response(),
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("lookup: {e}")).into_response()
        }
    };

    let offset = q.offset as usize;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let path = ObjPath::from(rec.object_path.as_str());
    let (size, content) = match read_window(&state.store, &path, offset, limit).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    Json(serde_json::json!({
        "id": id,
        "object_path": rec.object_path,
        "sha256": rec.sha256,
        "artifact_class": rec.artifact_class,
        "source": rec.source,
        "size_bytes": size,
        "offset": offset,
        "returned_bytes": content.len(),
        "eof": offset.saturating_add(content.len()) >= size,
        "content": content,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct PathQuery {
    /// Full object-store key, e.g. a `LogFile` node's `object_path`.
    path: String,
    #[serde(default)]
    offset: u64,
    limit: Option<u64>,
}

/// `GET /v1/artifacts/by-path?path=&offset=&limit=` — fetch by object key.
///
/// The graph's `LogFile` nodes carry an `object_path` (not the artifact id), so
/// the UI fetches by path. Tenant isolation is enforced by the key prefix: every
/// artifact key embeds its tenant uuid, so a caller can only read
/// `artifacts/<their-tenant>/…`.
async fn get_artifact_by_path(
    State(state): State<ArtifactsState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<PathQuery>,
) -> Response {
    let prefix = format!("artifacts/{}/", principal.tenant.as_uuid());
    if !q.path.starts_with(&prefix) {
        return (
            StatusCode::FORBIDDEN,
            "path is not in your tenant's artifact namespace",
        )
            .into_response();
    }
    let offset = q.offset as usize;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let path = ObjPath::from(q.path.as_str());
    let (size, content) = match read_window(&state.store, &path, offset, limit).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    Json(serde_json::json!({
        "object_path": q.path,
        "size_bytes": size,
        "offset": offset,
        "returned_bytes": content.len(),
        "eof": offset.saturating_add(content.len()) >= size,
        "content": content,
    }))
    .into_response()
}

/// Read a clamped byte window of `path`, returning `(total_size, content)` or an
/// error response. A past-end offset yields an empty slice rather than erroring.
async fn read_window(
    store: &Arc<dyn ObjectStore>,
    path: &ObjPath,
    offset: usize,
    limit: usize,
) -> Result<(usize, String), Response> {
    let size = match store.head(path).await {
        Ok(meta) => meta.size,
        Err(object_store::Error::NotFound { .. }) => {
            return Err((StatusCode::NOT_FOUND, "artifact blob missing").into_response())
        }
        Err(e) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("head: {e}")).into_response())
        }
    };
    if offset >= size {
        return Ok((size, String::new()));
    }
    let end = offset.saturating_add(limit).min(size);
    match store.get_range(path, offset..end).await {
        Ok(bytes) => Ok((size, String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")).into_response()),
    }
}

/// Build the artifact-retrieval router. Mount behind `require_role_middleware`
/// (read role) alongside the query router; the auth middleware injects the
/// [`Principal`] this handler scopes by.
pub fn artifacts_router(catalog: Arc<dyn Catalog>, store: Arc<dyn ObjectStore>) -> Router {
    Router::new()
        // Static `by-path` is registered first; matchit prioritises it over the
        // `:id` param so it never collides.
        .route("/v1/artifacts/by-path", get(get_artifact_by_path))
        .route("/v1/artifacts/:id", get(get_artifact))
        .with_state(ArtifactsState { catalog, store })
}
