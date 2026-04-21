//! Only compiles/runs when the `web-ui` feature is on.
#![cfg(feature = "web-ui")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for .oneshot

#[tokio::test]
async fn serves_index_at_root() {
    let app = kyma_server::web_ui::router();
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().starts_with("text/html"));
}

#[tokio::test]
async fn spa_fallback_to_index_for_unknown_path() {
    let app = kyma_server::web_ui::router();
    let req = Request::builder()
        .uri("/some/client-route")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().starts_with("text/html"));
}

#[tokio::test]
async fn hashed_asset_is_cache_immutable() {
    let app = kyma_server::web_ui::router();
    // Discover a real asset at runtime so the test isn't hash-coupled.
    let asset_path = kyma_web_assets::DIST
        .get_dir("assets")
        .expect("web/dist/assets/ missing — did you run `pnpm -C web build`?")
        .files()
        .next()
        .expect("no files under web/dist/assets/")
        .path()
        .to_string_lossy()
        .into_owned();
    let req = Request::builder()
        .uri(format!("/{asset_path}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn index_is_no_cache() {
    let app = kyma_server::web_ui::router();
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-cache");
}
