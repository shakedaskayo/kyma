//! UI static-serving routes. Compiled in only when the `web-ui` feature is on.

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use kyma_web_assets::{mime_for, DIST};

/// Router exposing: `GET /`, `GET /assets/*`, and a SPA fallback.
pub fn router() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset))
        .fallback(serve_spa_fallback)
}

async fn serve_index() -> Response {
    match DIST.get_file("index.html") {
        Some(f) => html_response(f.contents()),
        None => (
            StatusCode::NOT_FOUND,
            "index.html missing from embedded assets",
        )
            .into_response(),
    }
}

/// Safe: DIST is compile-time, not filesystem — no traversal possible.
async fn serve_asset(uri: Uri) -> Response {
    // path strips the leading "/"
    let raw = uri.path().trim_start_matches('/');
    match DIST.get_file(raw) {
        Some(f) => asset_response(raw, f.contents()),
        None => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

/// Unknown paths fall back to `index.html` so client-side routing works on
/// reload / deep-link — EXCEPT API prefixes. An API route that isn't mounted
/// on this deployment (e.g. control-plane-only surfaces in local mode) must
/// be a real JSON 404, not a 200 with HTML: clients expecting JSON would
/// choke on the body and the missing route would be invisible.
async fn serve_spa_fallback(uri: Uri) -> Response {
    let p = uri.path();
    let is_api = p.starts_with("/v1/")
        || p.starts_with("/flight")
        || p.starts_with("/mcp")
        || p == "/metrics"
        || p == "/health";
    if is_api {
        return (
            StatusCode::NOT_FOUND,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            format!(r#"{{"error":"endpoint not available on this server","path":{p:?}}}"#),
        )
            .into_response();
    }
    match DIST.get_file("index.html") {
        Some(f) => html_response(f.contents()),
        None => (StatusCode::NOT_FOUND, "index.html missing").into_response(),
    }
}

fn html_response(body: &'static [u8]) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(Body::from(body))
        .unwrap()
}

fn asset_response(path: &str, body: &'static [u8]) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static(mime_for(path)),
        )
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        )
        .body(Body::from(body))
        .unwrap()
}
