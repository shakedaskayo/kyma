//! Health check endpoint.

use axum::{http::StatusCode, Json};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct HealthResponse<'a> {
    status: &'a str,
    version: &'a str,
}

pub(crate) async fn health() -> (StatusCode, Json<HealthResponse<'static>>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok",
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
}
