//! Prometheus metrics — recorder setup + `/metrics` route.
//!
//! The `metrics` facade crate is populated from every part of the engine
//! (ingest, catalog, exec). This module installs a single
//! `PrometheusRecorder` at process startup and exposes a scraping endpoint.

use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the Prometheus recorder. Call once at process startup; calling
/// again is a no-op.
pub fn install() -> &'static PrometheusHandle {
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Suffix("_duration_seconds".to_string()),
                &[
                    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                ],
            )
            .expect("valid histogram buckets")
            .install_recorder()
            .expect("metrics recorder install")
    })
}

/// Build the `/metrics` Axum router.
pub fn router() -> Router {
    Router::new().route("/metrics", get(scrape))
}

async fn scrape() -> impl IntoResponse {
    let Some(handle) = HANDLE.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder not installed",
        )
            .into_response();
    };
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        handle.render(),
    )
        .into_response()
}
