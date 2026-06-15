//! Integration tests for the `application/x-cypher` query frontend.
//!
//! A Cypher query runs against ONE registered graph, resolved from the
//! `x-graph` request header (`"<db>/<graph>"` or `"<graph>"`, falling back to
//! `x-database`). We assert:
//!   - POST /v1/query with content-type: application/x-cypher + x-graph:<db>/<g>
//!     and a valid single-hop MATCH → 200 (SQL compiled + executed; the graph
//!     tables are schema-only so 0 rows come back, which is still a 200).
//!   - missing x-graph when zero or many graphs are registered → 400.
//!   - x-graph naming an unknown graph → 400 "graph not found: <name>".
//!   - malformed / unsupported Cypher → 400 carrying the ParseError text.
//!
//! These exercise the full HTTP path through `query_handler`'s
//! `application/x-cypher` branch + `resolve_graph_binding`.

#![cfg(feature = "test-support")]

use arrow_schema::{DataType, Field, Schema};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_core::catalog::{GraphSpec, TableConfig};
use std::sync::Arc;
use tower::ServiceExt;

/// Seed two schema-only tables (`gnodes(id,name)`, `gedges(src,dst,type)`) in
/// `obs` and register a conventional graph "kg" over them. The columns are the
/// ones the cypher→kql→sql chain references for
/// `MATCH (a)-[r]->(b) RETURN a.name`, so the emitted SQL is valid against the
/// (empty) tables and executes to 0 rows.
async fn seeded_state_with_graph() -> kyma_server::QueryState {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;

    // Resolve the "obs" database id (seeded by the fixture) for create_table.
    let obs_id = state
        .catalog
        .lookup_database("obs")
        .await
        .expect("lookup_database obs")
        .expect("obs database exists");

    let node_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    state
        .catalog
        .create_table(obs_id, "gnodes", node_schema, TableConfig::default())
        .await
        .expect("create_table gnodes");

    let edge_schema = Arc::new(Schema::new(vec![
        Field::new("src", DataType::Utf8, false),
        Field::new("dst", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, true),
    ]));
    state
        .catalog
        .create_table(obs_id, "gedges", edge_schema, TableConfig::default())
        .await
        .expect("create_table gedges");

    // Register a conventional graph: nodes(id,name) + edges(src,dst,type).
    let spec = GraphSpec::with_defaults("gnodes", "gedges");
    state
        .catalog
        .create_graph("obs", "kg", spec)
        .await
        .expect("create_graph kg");

    state
}

async fn run(state: kyma_server::QueryState, req: Request<Body>) -> (StatusCode, String) {
    let app = kyma_server::router(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn cypher_req(x_graph: Option<&str>, db: &str, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/v1/query")
        .header("content-type", "application/x-cypher")
        .header("x-database", db);
    if let Some(g) = x_graph {
        b = b.header("x-graph", g);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn cypher_with_explicit_graph_returns_200() {
    let state = seeded_state_with_graph().await;
    let req = cypher_req(
        Some("obs/kg"),
        "obs",
        "MATCH (a)-[r]->(b) RETURN a.name LIMIT 5",
    );
    let (status, body) = run(state, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "expected 200 for valid cypher against a registered graph, body: {body}"
    );
}

#[tokio::test]
async fn cypher_with_bare_graph_name_uses_x_database() {
    let state = seeded_state_with_graph().await;
    // x-graph is just the graph name ("kg"); db comes from x-database ("obs").
    let req = cypher_req(Some("kg"), "obs", "MATCH (a)-[r]->(b) RETURN a.name LIMIT 5");
    let (status, body) = run(state, req).await;
    assert_eq!(status, StatusCode::OK, "bare graph name path, body: {body}");
}

#[tokio::test]
async fn cypher_missing_x_graph_with_one_graph_resolves() {
    // Exactly one graph registered in scope → x-graph may be omitted.
    let state = seeded_state_with_graph().await;
    let req = cypher_req(None, "obs", "MATCH (a)-[r]->(b) RETURN a.name LIMIT 5");
    let (status, body) = run(state, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "single registered graph should be auto-selected, body: {body}"
    );
}

#[tokio::test]
async fn cypher_unknown_graph_is_400() {
    let state = seeded_state_with_graph().await;
    let req = cypher_req(Some("obs/nope"), "obs", "MATCH (a)-[r]->(b) RETURN a.name");
    let (status, body) = run(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(
        body.contains("graph not found") && body.contains("nope"),
        "expected 'graph not found: nope', got: {body}"
    );
}

#[tokio::test]
async fn cypher_missing_x_graph_when_no_graph_is_400() {
    // No graph registered → cannot auto-select → 400 instructing x-graph.
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let req = cypher_req(None, "obs", "MATCH (a)-[r]->(b) RETURN a.name");
    let (status, body) = run(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(
        body.contains("x-graph"),
        "expected guidance to specify x-graph, got: {body}"
    );
}

#[tokio::test]
async fn cypher_malformed_is_400_with_parse_error() {
    let state = seeded_state_with_graph().await;
    // Genuinely malformed Cypher → ParseError. (Multi-hop MATCH, the old
    // probe here, is supported since the multi-hop graph-match work.)
    let req = cypher_req(Some("obs/kg"), "obs", "FROBNICATE (a) RETURN a.name");
    let (status, body) = run(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(
        body.contains("expected MATCH"),
        "expected ParseError text in the 400 body, got: {body}"
    );
}

#[tokio::test]
async fn cypher_multi_hop_match_is_supported() {
    let state = seeded_state_with_graph().await;
    // Regression guard: this exact shape was a 400 ("multi-hop not
    // supported") before kyma-kql learned multi-hop graph-match.
    let req = cypher_req(
        Some("obs/kg"),
        "obs",
        "MATCH (a)-[r]->(b)-[s]->(c) RETURN a.name",
    );
    let (status, body) = run(state, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}

#[tokio::test]
async fn graph_analytics_endpoint_dispatches_by_kind() {
    // The schema-only graph has no edges, so every analytic returns an empty
    // result — enough to validate the route + resolve + JSON envelope. (Algorithm
    // correctness is differentially covered in kyma-graph-topo / kyma-graph.)
    let state = seeded_state_with_graph().await;
    let app = kyma_server::router(state);

    for kind in ["components", "communities", "pagerank"] {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/v1/graph/kg/analytics?kind={kind}"))
            .header("x-database", "obs")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(status, StatusCode::OK, "kind={kind} body: {body}");
        assert!(body.contains(&format!("\"kind\":\"{kind}\"")), "kind={kind} body: {body}");
        assert!(body.contains("\"results\""), "kind={kind} body: {body}");
    }

    // Unknown kind → 400.
    let bad = Request::builder()
        .method("GET")
        .uri("/v1/graph/kg/analytics?kind=bogus")
        .header("x-database", "obs")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(bad).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cypher_variable_length_match_is_supported() {
    let state = seeded_state_with_graph().await;
    // Regression guard: `-[*M..N]->` was a 400 ("variable-length not
    // supported") before kyma-kql learned graph-var-match. The full HTTP path
    // (cypher → graph-var-match → recursive-CTE SQL → DataFusion) must plan and
    // execute to 200 (schema-only tables ⇒ 0 rows, still a 200).
    let req = cypher_req(
        Some("obs/kg"),
        "obs",
        "MATCH (a)-[r*1..3]->(b) RETURN a.name, b.name LIMIT 5",
    );
    let (status, body) = run(state, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "variable-length cypher should plan + execute, body: {body}"
    );
}
