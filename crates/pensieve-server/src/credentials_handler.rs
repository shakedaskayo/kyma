//! HTTP admin API for the system-wide credentials store (`/v1/credentials`).
//!
//! Credentials are typed, encrypted secrets referenced by id. The list and get
//! endpoints return [`CredentialSummary`] (no plaintext, just a masked
//! preview); plaintext only flows internally to subsystems via
//! [`PgCredentialStore::fetch`] keyed by a stable id.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use pensieve_core::credentials::{CredentialStore, CredentialValue};
use pensieve_core::tenant::TenantId;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct CredentialsState {
    pub store: Arc<dyn CredentialStore>,
}

pub fn router(state: CredentialsState) -> Router {
    Router::new()
        .route("/v1/credentials", post(create).get(list))
        .route("/v1/credentials/:id", get(get_one).delete(delete_one))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateReq {
    label: String,
    /// The typed credential value (the `kind` field selects the variant).
    value: CredentialValue,
    #[serde(default)]
    metadata: Value,
}

async fn create(
    Extension(tenant): Extension<TenantId>,
    State(s): State<CredentialsState>,
    Json(req): Json<CreateReq>,
) -> impl IntoResponse {
    if req.label.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "label is required" })),
        )
            .into_response();
    }
    let meta = if req.metadata.is_null() {
        Value::Object(Default::default())
    } else {
        req.metadata
    };
    match s
        .store
        .create(tenant, req.label.trim(), &req.value, meta)
        .await
    {
        Ok(summary) => (StatusCode::CREATED, Json(summary)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn list(
    Extension(tenant): Extension<TenantId>,
    State(s): State<CredentialsState>,
) -> impl IntoResponse {
    match s.store.list(tenant).await {
        Ok(items) => (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_one(
    Extension(tenant): Extension<TenantId>,
    State(s): State<CredentialsState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.store.fetch(tenant, id).await {
        // Return a Summary, never the plaintext value.
        Ok(c) => {
            let preview = c.value.preview();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": c.id,
                    "label": c.label,
                    "kind": c.kind,
                    "preview": preview,
                    "metadata": c.metadata,
                    "created_at": c.created_at,
                    "updated_at": c.updated_at,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                StatusCode::NOT_FOUND.into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": msg })),
                )
                    .into_response()
            }
        }
    }
}

async fn delete_one(
    Extension(tenant): Extension<TenantId>,
    State(s): State<CredentialsState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.store.delete(tenant, id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
