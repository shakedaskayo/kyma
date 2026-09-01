//! CRUD endpoints for saved Discover views.
//!
//! GET    /v1/explore/views        — list (owner-scoped, read role)
//! POST   /v1/explore/views        — create (write role)
//! PATCH  /v1/explore/views/:id    — update (write role)
//! DELETE /v1/explore/views/:id    — delete (write role)
//!
//! Auth context:
//!  - Scoped to the authenticated tenant via `Extension<TenantId>`.
//!  - Owner is the principal's `subject`. Auth-disabled deployments inject a
//!    placeholder admin principal with `subject = None`; we reject those
//!    requests with `403 missing_subject` rather than silently coalescing
//!    everyone into a single null-owner bucket.

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pensieve_catalog::saved_views::{
    create as cat_create, delete as cat_delete, list as cat_list, update as cat_update,
    NewSavedView, SavedViewError, UpdateSavedView,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::Principal;
use pensieve_core::tenant::TenantId;

/// Shared state for the saved-views CRUD handlers.
#[derive(Clone)]
pub struct SavedViewsState {
    pub pool: Arc<PgPool>,
}

/// `GET /v1/explore/views` — list saved views owned by the caller.
pub async fn list_views(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
) -> Response {
    let subject = match principal.subject.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return error(
                "missing_subject",
                "saved views require an authenticated subject",
                StatusCode::FORBIDDEN,
            )
        }
    };
    match cat_list(&state.pool, tenant.as_uuid(), subject).await {
        Ok(views) => Json(views).into_response(),
        Err(e) => map_error(e),
    }
}

/// `POST /v1/explore/views` — create a saved view owned by the caller.
pub async fn create_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<NewSavedView>,
) -> Response {
    let subject = match principal.subject.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return error(
                "missing_subject",
                "saved views require an authenticated subject",
                StatusCode::FORBIDDEN,
            )
        }
    };
    if body.name.trim().is_empty() {
        return error("bad_request", "name is required", StatusCode::BAD_REQUEST);
    }
    if body.sources.is_empty() {
        return error(
            "bad_request",
            "sources must be non-empty",
            StatusCode::BAD_REQUEST,
        );
    }
    match cat_create(&state.pool, tenant.as_uuid(), subject, body).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => map_error(e),
    }
}

/// `PATCH /v1/explore/views/:id` — partial update of a saved view.
pub async fn update_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSavedView>,
) -> Response {
    let subject = match principal.subject.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return error(
                "missing_subject",
                "saved views require an authenticated subject",
                StatusCode::FORBIDDEN,
            )
        }
    };
    match cat_update(&state.pool, tenant.as_uuid(), subject, id, body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => map_error(e),
    }
}

/// `DELETE /v1/explore/views/:id` — delete a saved view.
pub async fn delete_view(
    State(state): State<SavedViewsState>,
    Extension(tenant): Extension<TenantId>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Response {
    let subject = match principal.subject.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return error(
                "missing_subject",
                "saved views require an authenticated subject",
                StatusCode::FORBIDDEN,
            )
        }
    };
    match cat_delete(&state.pool, tenant.as_uuid(), subject, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => map_error(e),
    }
}

fn map_error(e: SavedViewError) -> Response {
    match e {
        SavedViewError::NotFound => error(
            "not_found",
            "saved view not found",
            StatusCode::NOT_FOUND,
        ),
        SavedViewError::NameConflict => error(
            "name_conflict",
            "a saved view with this name already exists",
            StatusCode::CONFLICT,
        ),
        SavedViewError::Db(msg) => {
            error("db_error", &msg, StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn error(code: &str, msg: &str, status: StatusCode) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":msg}})),
    )
        .into_response()
}
