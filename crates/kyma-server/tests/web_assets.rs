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
    let req = Request::builder().uri("/some/client-route").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().starts_with("text/html"));
}
