//! Integration tests for per-database token scope enforcement (Task 2.4).
//!
//! A stub `AuthBackend` returns a `Principal` scoped to `["allowed_db"]`.
//! We assert:
//!   - `GET /v1/catalog/schema` with x-database: other_db → 403 (not
//!     directly, since schema_handler filters — instead we assert the db is
//!     absent from the filtered response)
//!   - `GET /v1/catalog/schema` with x-database: allowed_db → only that db
//!     appears in the response
//!   - `POST /v1/query` with x-database: other_db → 403
//!   - `POST /v1/query` with x-database: allowed_db → 200 (or 400 on empty
//!     body, i.e. past the scope gate)

#![cfg(feature = "test-support")]

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_server::auth::{AuthBackend, AuthError, AuthLayerState, Principal, Role};
use std::sync::Arc;
use tower::ServiceExt;

/// Stub backend: any non-empty token → Admin scoped to ["allowed_db"].
struct ScopedAuth;

#[async_trait]
impl AuthBackend for ScopedAuth {
    fn enabled(&self) -> bool {
        true
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if token.is_empty() {
            return Err(AuthError::MissingToken);
        }
        Ok(Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: Some("test-scoped-user".to_string()),
            allowed_databases: Some(vec!["allowed_db".to_string()]),
            allowed_realms: None,
        })
    }
}

fn build_app(state: kyma_server::QueryState) -> axum::Router {
    let backend: Arc<dyn AuthBackend> = Arc::new(ScopedAuth);
    let layer = AuthLayerState {
        backend,
        required: Role::Read,
    };
    kyma_server::router(state).layer(axum::middleware::from_fn_with_state(
        layer,
        kyma_server::auth::require_role_middleware,
    ))
}

async fn one(state: kyma_server::QueryState, req: Request<Body>) -> (StatusCode, String) {
    let app = build_app(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn auth_header(b: axum::http::request::Builder) -> axum::http::request::Builder {
    b.header("authorization", "Bearer any-token")
}

// ---------------------------------------------------------------------------
// /v1/catalog/schema — filtered by scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn catalog_schema_excludes_unscoped_databases() {
    // The seeded state has an "obs" database. Our token is scoped to
    // "allowed_db" only. The schema response should contain no databases
    // (since "obs" ≠ "allowed_db").
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let req = auth_header(Request::builder().uri("/v1/catalog/schema"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let dbs = v["databases"].as_array().unwrap();
    assert!(
        dbs.is_empty(),
        "expected empty databases (obs is not in scope), got: {dbs:?}"
    );
}

#[tokio::test]
async fn catalog_schema_includes_scoped_database() {
    // Seed a state with "allowed_db" and verify the handler includes it.
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    // Create "allowed_db" in the catalog.
    state
        .catalog
        .create_database("allowed_db")
        .await
        .expect("create allowed_db");

    let req = auth_header(Request::builder().uri("/v1/catalog/schema"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let dbs = v["databases"].as_array().unwrap();
    assert!(
        dbs.iter().any(|d| d["name"] == "allowed_db"),
        "expected allowed_db in schema, got: {dbs:?}"
    );
    // "obs" must not appear (not in allowed_databases).
    assert!(
        !dbs.iter().any(|d| d["name"] == "obs"),
        "obs should be filtered out, but got: {dbs:?}"
    );
}

// ---------------------------------------------------------------------------
// /v1/query — enforced at query_handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn query_handler_rejects_unscoped_database() {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let req = auth_header(
        Request::builder()
            .method("POST")
            .uri("/v1/query")
            .header("x-database", "other_db")
            .header("content-type", "application/sql"),
    )
    .body(Body::from("SELECT 1"))
    .unwrap();
    let (status, body) = one(state, req).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "expected 403 for unscoped database, body: {body}"
    );
}

// ---------------------------------------------------------------------------
// /v1/graph/schema — the synthetic schema graph spans all databases and must
// be filtered by the principal's scope (metadata-leak guard).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn schema_graph_excludes_unscoped_databases() {
    // Seeded state has an "obs" database with tables; token is scoped to
    // "allowed_db" only — nothing from "obs" may appear in the schema graph.
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let req = auth_header(Request::builder().uri("/v1/graph/schema/overview"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("obs"),
        "schema graph leaked unscoped database metadata: {body}"
    );
}

#[tokio::test]
async fn schema_graph_includes_scoped_database() {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    // The schema graph only renders databases that have tables — give
    // allowed_db one so it produces nodes.
    let db_id = state
        .catalog
        .create_database("allowed_db")
        .await
        .expect("create allowed_db");
    let schema = std::sync::Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "id",
        arrow_schema::DataType::Utf8,
        false,
    )]));
    state
        .catalog
        .create_table(
            db_id,
            "events",
            schema,
            kyma_core::catalog::TableConfig::default(),
        )
        .await
        .expect("create_table events");
    let req = auth_header(Request::builder().uri("/v1/graph/schema/overview"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("allowed_db"),
        "schema graph should include the scoped database: {body}"
    );
    assert!(!body.contains("\"obs\""), "obs must stay filtered: {body}");
}

#[tokio::test]
async fn query_handler_allows_scoped_database() {
    // Token is scoped to "allowed_db". The database exists (we create it).
    // The query itself may fail with a SQL error since there are no tables,
    // but we must get past the scope gate (no 403).
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    state
        .catalog
        .create_database("allowed_db")
        .await
        .expect("create allowed_db");

    let req = auth_header(
        Request::builder()
            .method("POST")
            .uri("/v1/query")
            .header("x-database", "allowed_db")
            .header("content-type", "application/sql"),
    )
    .body(Body::from("SELECT 1"))
    .unwrap();
    let (status, _body) = one(state, req).await;
    // 200 (empty result) or any non-403 status means we passed the scope gate.
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "expected to pass scope gate for allowed_db"
    );
}
