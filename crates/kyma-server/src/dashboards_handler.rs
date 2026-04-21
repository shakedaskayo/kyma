//! HTTP handlers for the Dashboards CRUD API.
//!
//! Exposes:
//!   GET    /v1/dashboards                → list
//!   POST   /v1/dashboards                → create (with optional panels)
//!   GET    /v1/dashboards/:id            → get with panels
//!   PATCH  /v1/dashboards/:id            → update (scalar fields + atomic panel replace)
//!   DELETE /v1/dashboards/:id            → delete

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use kyma_core::catalog::{
    Catalog, Dashboard, DashboardPanelInput, DashboardUpdate, DashboardWithPanels,
};
use kyma_core::errors::CatalogError;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// Shared state for dashboard handlers — just the catalog.
#[derive(Clone)]
pub struct DashboardState {
    pub catalog: Arc<dyn Catalog>,
}

// -------------------------------------------------------------------------
// Request bodies
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateDashboardBody {
    pub name: String,
    pub description: Option<String>,
    pub panels: Option<Vec<DashboardPanelInput>>,
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

/// `GET /v1/dashboards`
pub async fn list_dashboards(
    State(state): State<DashboardState>,
) -> Result<Json<Vec<Dashboard>>, ApiError> {
    let dashboards = state.catalog.list_dashboards().await?;
    Ok(Json(dashboards))
}

/// `POST /v1/dashboards`
pub async fn create_dashboard(
    State(state): State<DashboardState>,
    Json(body): Json<CreateDashboardBody>,
) -> Result<(StatusCode, Json<DashboardWithPanels>), ApiError> {
    let dashboard = state
        .catalog
        .create_dashboard(&body.name, body.description.as_deref())
        .await?;

    // If panels were provided, replace them atomically.
    let dashboard = if let Some(panels) = body.panels {
        state
            .catalog
            .update_dashboard(
                dashboard.id,
                DashboardUpdate {
                    panels: Some(panels),
                    ..Default::default()
                },
            )
            .await?
    } else {
        dashboard
    };

    // Fetch with panels to return the full shape.
    let with_panels = state
        .catalog
        .get_dashboard(dashboard.id)
        .await?
        .ok_or(CatalogError::DashboardNotFound { id: dashboard.id })?;

    Ok((StatusCode::CREATED, Json(with_panels)))
}

/// `GET /v1/dashboards/:id`
pub async fn get_dashboard(
    State(state): State<DashboardState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DashboardWithPanels>, ApiError> {
    let with_panels = state
        .catalog
        .get_dashboard(id)
        .await?
        .ok_or(CatalogError::DashboardNotFound { id })?;
    Ok(Json(with_panels))
}

/// `PATCH /v1/dashboards/:id`
pub async fn update_dashboard(
    State(state): State<DashboardState>,
    Path(id): Path<Uuid>,
    Json(patch): Json<DashboardUpdate>,
) -> Result<Json<DashboardWithPanels>, ApiError> {
    state
        .catalog
        .update_dashboard(id, patch)
        .await
        .map_err(|e| {
            if matches!(e, CatalogError::DashboardNotFound { .. }) {
                ApiError::NotFound(id)
            } else {
                ApiError::Catalog(e)
            }
        })?;

    // Return the full shape with panels after update.
    let with_panels = state
        .catalog
        .get_dashboard(id)
        .await?
        .ok_or(ApiError::NotFound(id))?;

    Ok(Json(with_panels))
}

/// `DELETE /v1/dashboards/:id`
pub async fn delete_dashboard(
    State(state): State<DashboardState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let deleted = state.catalog.delete_dashboard(id).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(id))
    }
}

// -------------------------------------------------------------------------
// Error type
// -------------------------------------------------------------------------

#[derive(Debug)]
pub enum ApiError {
    NotFound(Uuid),
    Catalog(CatalogError),
}

impl From<CatalogError> for ApiError {
    fn from(e: CatalogError) -> Self {
        match e {
            CatalogError::DashboardNotFound { id } => ApiError::NotFound(id),
            other => ApiError::Catalog(other),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::NotFound(id) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("dashboard {id} not found") })),
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
