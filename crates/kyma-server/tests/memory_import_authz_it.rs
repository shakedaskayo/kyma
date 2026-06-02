//! `POST /v1/agent/memory/import` is a bulk memory WRITE (the receive side of
//! sync). The surrounding agent router is mounted at `Role::Read`, so the
//! handler gates itself at `Role::Write` in-handler. This test pins that: a
//! read-only token must be refused (403) and an unauthenticated request must be
//! rejected (401), so a read token can never push memory into the store.
//!
//! We assert only the refusal paths — they short-circuit before the handler
//! touches the embedding backend (which would load an ONNX model). The
//! write-allowed path is exercised by the live sync flow, not here.

#![cfg(feature = "test-support")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_server::agent::local::{
    NullCredentialStore, NullEnabledSkillsStore, NullEnginePreferenceStore,
};
use kyma_server::agent::AgentState;
use kyma_server::auth::{require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role};
use std::sync::Arc;
use tower::ServiceExt;

/// Build the agent router with a backend that knows a read-role and a
/// write-role token, wrapped in the same `Role::Read` middleware production uses.
fn agent_app(state: &kyma_server::QueryState) -> axum::Router {
    let agent_state = AgentState {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: None,
        engines: Arc::new(NullEnginePreferenceStore),
        credentials: Arc::new(NullCredentialStore),
        tenant: DEFAULT_TENANT,
        skills: Arc::new(NullEnabledSkillsStore),
        mcp_url: None,
    };
    let backend: Arc<dyn AuthBackend> =
        Arc::new(EnvAuthBackend::from_str("read-token:read,write-token:write"));
    kyma_server::agent::router(agent_state).layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend,
            required: Role::Read,
        },
        require_role_middleware,
    ))
}

async fn post_import(app: axum::Router, auth: Option<&str>) -> StatusCode {
    let mut req = Request::builder()
        .method("POST")
        .uri("/memory/import")
        .header("content-type", "application/json");
    if let Some(tok) = auth {
        req = req.header("authorization", format!("Bearer {tok}"));
    }
    let req = req
        .body(Body::from(r#"{"memory_nodes":[],"memory_edges":[]}"#))
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn import_refuses_read_token_and_anonymous() {
    let state = kyma_server::test_support::seeded_state_empty().await;

    // Read-only token passes the Role::Read middleware but the handler's
    // Role::Write gate must refuse it.
    assert_eq!(
        post_import(agent_app(&state), Some("read-token")).await,
        StatusCode::FORBIDDEN,
        "a read-only token must not be able to import (write) memory",
    );

    // No bearer at all — rejected by the middleware before the handler.
    assert_eq!(
        post_import(agent_app(&state), None).await,
        StatusCode::UNAUTHORIZED,
        "unauthenticated import must be rejected",
    );
}
