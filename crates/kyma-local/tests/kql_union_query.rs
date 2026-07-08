//! Integration test: KQL `union` over two tables in the same database.
//!
//! Exercises the full path from `POST /v1/ingest` → `POST /v1/query`
//! (`Content-Type: application/x-kql`) using the real `build_local_app`
//! router assembly (SQLite catalog + in-memory store), verifying that:
//!
//! 1. Rows from **both** tables are returned.
//! 2. The `withsource=_source` column correctly labels each row.
//! 3. Columns not present in a given branch are null-filled.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_format_tlm::TelemetryFormat;
use kyma_local::build_local_app;
use kyma_server::auth::EnvAuthBackend;
use object_store::memory::InMemory;
use std::sync::Arc;
use tower::ServiceExt as _;

/// Build the full local app with both read and write tokens.
async fn local_app() -> axum::Router {
    let catalog: Arc<dyn Catalog> = Arc::new(
        SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory SQLite catalog"),
    );
    let store = Arc::new(InMemory::new());
    let format = Arc::new(TelemetryFormat::new(store, "kyma-test"));
    // Mint two tokens: one for writing (ingest), one for reading (query).
    let backend: Arc<dyn kyma_server::auth::AuthBackend> =
        Arc::new(EnvAuthBackend::from_str("write-token:write,read-token:read"));
    let (app, _agent_state) = build_local_app(catalog, format, backend, None, None, None, None, None, kyma_local::watcher_status::LocalWatcherStatus::default(), None);
    app
}

/// Ingest NDJSON rows into `testdb.<table>` using the write token.
async fn ingest(app: axum::Router, table: &str, ndjson: &str) -> axum::Router {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/ingest")
        .header("X-Database", "testdb")
        .header("X-Table", table)
        .header("Content-Type", "application/x-ndjson")
        .header("Authorization", "Bearer write-token")
        .body(Body::from(ndjson.to_owned()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    if status != StatusCode::OK {
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap_or_default();
        panic!(
            "ingest into {table} failed: status={status} body={}",
            std::str::from_utf8(&body).unwrap_or("<binary>")
        );
    }
    app
}

#[tokio::test]
async fn kql_union_two_tables_with_source() {
    let app = local_app().await;

    // Ingest two rows into `logs` {timestamp, message}.
    let app = ingest(
        app,
        "logs",
        r#"{"timestamp":"2026-01-01T00:00:00Z","message":"hello"}
{"timestamp":"2026-01-01T00:01:00Z","message":"world"}"#,
    )
    .await;

    // Ingest two rows into `audits` {timestamp, actor}.
    let app = ingest(
        app,
        "audits",
        r#"{"timestamp":"2026-01-01T00:00:00Z","actor":"alice"}
{"timestamp":"2026-01-01T00:01:00Z","actor":"bob"}"#,
    )
    .await;

    // POST a KQL union query.
    let kql = "union withsource=_source logs, audits | take 10";
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query")
        .header("X-Database", "testdb")
        .header("Content-Type", "application/x-kql")
        .header("Authorization", "Bearer read-token")
        .body(Body::from(kql))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "union query must return 200");

    // Collect the body (NDJSON — one JSON object per line).
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("read response body");
    let body_str = std::str::from_utf8(&body_bytes).expect("response is UTF-8");

    // Parse each non-empty line as a JSON object.
    let rows: Vec<serde_json::Value> = body_str
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is valid JSON"))
        .collect();

    assert!(!rows.is_empty(), "expected at least one row from the union");

    // At least one row must come from `logs`.
    let logs_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.get("_source").and_then(|v| v.as_str()) == Some("logs"))
        .collect();
    assert!(
        !logs_rows.is_empty(),
        "expected at least one row with _source == \"logs\"; rows: {rows:?}"
    );

    // At least one row must come from `audits`.
    let audits_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.get("_source").and_then(|v| v.as_str()) == Some("audits"))
        .collect();
    assert!(
        !audits_rows.is_empty(),
        "expected at least one row with _source == \"audits\"; rows: {rows:?}"
    );

    // Sanity: total row count is exactly 4 (2 logs + 2 audits).
    assert_eq!(
        rows.len(),
        4,
        "expected 4 rows total (2 per table); got {}; rows: {rows:?}",
        rows.len()
    );

    // Assert outer-union null-fill: rows from logs must have non-null message
    // and absent or null actor; rows from audits must have non-null actor and
    // absent or null message.
    for row in &logs_rows {
        assert!(
            row.get("message").map_or(false, |v| !v.is_null()),
            "logs row must have non-null message; row: {row:?}"
        );
        let actor = row.get("actor");
        assert!(
            actor.is_none() || actor.map_or(false, |v| v.is_null()),
            "logs row must have absent or null actor; row: {row:?}"
        );
    }

    for row in &audits_rows {
        assert!(
            row.get("actor").map_or(false, |v| !v.is_null()),
            "audits row must have non-null actor; row: {row:?}"
        );
        let message = row.get("message");
        assert!(
            message.is_none() || message.map_or(false, |v| v.is_null()),
            "audits row must have absent or null message; row: {row:?}"
        );
    }
}
