//! HTTP query server — phase A.
//!
//! Exposes `POST /v1/query` accepting:
//!   - `X-Database`: database to query (defaults to `default`)
//!   - `Content-Type: application/sql` — the query text is the request body
//!
//! Builds a one-shot DataFusion `SessionContext`, registers every table in
//! the specified database as a [`KymaTable`], executes the SQL, and streams
//! the results back as NDJSON (one JSON object per row).
//!
//! Phase-A simplifications:
//!   - Only SQL (via DataFusion). KQL lands in M2 when `kyma-kql` and the
//!     `QueryFrontend` registry are wired.
//!   - Results are buffered in memory before streaming. Truly streaming
//!     response lands when we implement a custom scan `ExecutionPlan`.

#![forbid(unsafe_code)]

pub mod admin_handler;
pub mod agent;
pub mod auth;
pub mod auth_handler;
pub mod catalog_handler;
pub mod cleanup_handler;
pub mod credentials_handler;
pub mod dashboards_handler;
pub mod discover;
pub mod graph_handler;
pub mod flight;
pub mod capabilities;
mod health;
pub mod icon_config;
pub mod metrics;

#[cfg(feature = "web-ui")]
pub mod web_ui;

/// Build an axum `Router` that serves Arrow Flight over gRPC-web at `/flight/*`.
///
/// The router is **not** auth-wrapped — the caller must add auth middleware,
/// typically via `.layer(axum::middleware::from_fn_with_state(...))`.
///
/// # TODO(task 6.4): verify auth-denied behavior when the real gRPC-web client
/// lands. `require_role_middleware` returns a plain HTTP 401 on rejection, which
/// gRPC-web clients may surface as an opaque transport error rather than
/// UNAUTHENTICATED. If the client can't map it cleanly, the middleware may need
/// to emit gRPC trailers (grpc-status: 16 for UNAUTHENTICATED) for /flight/*.
#[cfg(feature = "web-ui")]
pub fn flight_web_router(state: QueryState) -> Router {
    use flight::{flight_grpc_web_service, FlightState};
    let flight_state = FlightState {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        node_id: state.node_id,
    };
    Router::new().nest_service("/flight", flight_grpc_web_service(flight_state))
}

#[cfg(feature = "test-support")]
pub mod test_support;

use arrow::json::ArrayWriter;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_core::catalog::{Catalog, TableRef};
use kyma_core::segment_format::SegmentFormat;
use kyma_exec::KymaTable;
use serde::Serialize;
use std::sync::Arc;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tracing::{debug, error, info};

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub use kyma_connectors::admin::AdminState as ConnectorAdminState;
pub use kyma_connectors::oauth::OAuthState;

/// Build the connector admin router (auth-eligible — caller wraps with middleware).
pub fn connector_admin_router(state: kyma_connectors::admin::AdminState) -> Router {
    kyma_connectors::admin::router(state)
}

/// Build the authenticated OAuth router (start + poll) — caller wraps with the
/// `Role::Write` middleware.
pub fn oauth_authed_router(state: OAuthState) -> Router {
    kyma_connectors::oauth::oauth_authed_router(state)
}

/// Build the **unauthenticated** OAuth callback router — mount alongside the
/// login route (the IdP redirect carries no bearer; the single-use `state`
/// token is the trust anchor).
pub fn oauth_callback_router(state: OAuthState) -> Router {
    kyma_connectors::oauth::oauth_callback_router(state)
}

/// Shared HTTP-handler state for the query surface.
#[derive(Clone)]
pub struct QueryState {
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
    pub schema_cache: Arc<catalog_handler::SchemaCache>,
    /// Current node's id. Passed into `KymaTable` so the scan path can
    /// fan extents out to peer nodes via the read-router.
    pub node_id: Option<kyma_core::types::NodeId>,
    /// Catalog Postgres pool. Threaded through so non-`Catalog`-trait
    /// surfaces (saved Discover views, etc.) can run SQL directly without
    /// having to downcast the `dyn Catalog`. `None` in **local mode**
    /// (`kyma-local serve`): the pool-only surfaces (saved Discover views)
    /// degrade to empty; query / catalog / graph / discover-search all run
    /// over the catalog + engine and work unchanged.
    pub pg_pool: Option<Arc<sqlx::PgPool>>,
}

/// Build the query router (auth-eligible — caller wraps with middleware).
///
/// Every route mounted here assumes at least `Role::Read`; the caller wraps
/// the entire router with `require_role_middleware(Role::Read)`.
pub fn router(state: QueryState) -> Router {
    use dashboards_handler::{get_dashboard, list_dashboards, DashboardState};
    use discover::saved_views_handler::{list_views, SavedViewsState};
    let dash_read_state = DashboardState {
        catalog: state.catalog.clone(),
    };
    // Dashboard read routes are on their own sub-router with DashboardState.
    let dash_read_router = Router::new()
        .route("/v1/dashboards", get(list_dashboards))
        .route("/v1/dashboards/:id", get(get_dashboard))
        .with_state(dash_read_state);

    // Saved-views list endpoint — read-role; create/update/delete live on
    // the separate write router so they can require Role::Write. In local mode
    // (no pool) saved views are unavailable, so the list is an empty array.
    let views_read_router = match state.pg_pool.clone() {
        Some(pool) => Router::new()
            .route("/v1/explore/views", get(list_views))
            .with_state(SavedViewsState { pool }),
        None => Router::new().route(
            "/v1/explore/views",
            get(|| async { axum::Json(serde_json::json!([])) }),
        ),
    };

    Router::new()
        .route("/v1/query", post(query_handler))
        .route(
            "/v1/explore/search",
            post(discover::handler::discover_search_handler),
        )
        .route("/v1/catalog/schema", get(catalog_handler::schema_handler))
        .with_state(state.clone())
        .merge(dash_read_router)
        .merge(views_read_router)
        .merge(graph_handler::graph_router(state))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}

/// Build the dashboards write router — POST, PATCH, DELETE require `Role::Write`.
///
/// Mount alongside the query router in `main.rs`, wrapped with
/// `require_role_middleware(Role::Write)`.
pub fn dashboards_write_router(catalog: Arc<dyn kyma_core::catalog::Catalog>) -> Router {
    use dashboards_handler::{
        create_dashboard, delete_dashboard, update_dashboard, DashboardState,
    };
    let state = DashboardState { catalog };
    Router::new()
        .route("/v1/dashboards", post(create_dashboard))
        .route(
            "/v1/dashboards/:id",
            axum::routing::patch(update_dashboard).delete(delete_dashboard),
        )
        .with_state(state)
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}

/// Build the Discover saved-views write router — POST, PATCH, DELETE
/// require `Role::Write`.
///
/// Mounts:
///   POST   /v1/explore/views        — create
///   PATCH  /v1/explore/views/:id    — update
///   DELETE /v1/explore/views/:id    — delete
///
/// The `GET /v1/explore/views` list endpoint lives on the read-side router
/// (see [`router`]).
pub fn discover_views_write_router(pool: Arc<sqlx::PgPool>) -> Router {
    use discover::saved_views_handler::{
        create_view, delete_view, update_view, SavedViewsState,
    };
    let state = SavedViewsState { pool };
    Router::new()
        .route("/v1/explore/views", post(create_view))
        .route(
            "/v1/explore/views/:id",
            axum::routing::patch(update_view).delete(delete_view),
        )
        .with_state(state)
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}

/// Build the cleanup write router — POST requires `Role::Write`.
///
/// Mounts `POST /v1/database/:db/table/:table/cleanup`.
/// Mount alongside the query router in `main.rs`, wrapped with
/// `require_role_middleware(Role::Write)`.
pub fn cleanup_write_router(catalog: Arc<dyn kyma_core::catalog::Catalog>) -> Router {
    use cleanup_handler::{cleanup_table, CleanupState};
    let state = CleanupState { catalog };
    Router::new()
        .route(
            "/v1/database/:db/table/:table/cleanup",
            post(cleanup_table),
        )
        .with_state(state)
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}

/// Separate health router — always unauthenticated.
pub fn health_router() -> Router {
    Router::new().route("/health", get(health::health))
}

/// Variant of [`router`] that additionally nests the inline agent surface
/// under `/v1/agent`. Called by `kyma-bin` once the `PgPool` (needed for
/// `agent_runs` persistence) is available.
///
/// The agent surface is guarded against database-scoped tokens (fail closed):
/// the agent's tool loop lets the model address any database (`execute_sql`
/// takes a database argument), bypassing per-handler scope checks. Until the
/// tool context enforces `allowed_databases`, scoped tokens get 403 here —
/// same policy as the Flight surface.
pub fn router_with_agent(state: QueryState, agent_state: agent::AgentState) -> Router {
    router(state).nest(
        "/v1/agent",
        agent::router(agent_state)
            .layer(axum::middleware::from_fn(scoped_token_guard_middleware)),
    )
}

/// Wrap any router with a permissive dev CORS layer so a browser dev-server
/// running on a separate origin (e.g. `http://localhost:5173`) can reach the
/// API. Mirrors the request origin, accepts any method / header, and exposes
/// all response headers so SSE streams + Authorization headers flow through.
///
/// Apply this to the outermost `Router` in `kyma-bin::main`. Production
/// deployments should replace it with a config-driven origin allow-list.
pub fn with_permissive_cors(r: Router) -> Router {
    use tower_http::cors::{AllowOrigin, Any, CorsLayer};
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any);
    r.layer(cors)
}

/// CORS for production: explicit origin allow-list from
/// `KYMA_CORS_ALLOWED_ORIGINS` (comma-separated). Falls back to the
/// permissive mirror behavior when UNSET (dev default). When the variable is
/// set but contains no valid origins (typo, bad syntax), we fail CLOSED with
/// an empty allow-list — a misconfigured production deployment must not
/// silently become world-readable.
pub fn with_configured_cors(r: Router) -> Router {
    use tower_http::cors::{AllowOrigin, Any, CorsLayer};
    let Some(raw) = std::env::var("KYMA_CORS_ALLOWED_ORIGINS").ok() else {
        tracing::warn!("KYMA_CORS_ALLOWED_ORIGINS unset — using permissive CORS (dev only)");
        return with_permissive_cors(r);
    };
    let origins: Vec<axum::http::HeaderValue> = raw
        .split(',')
        .filter_map(|s| s.trim().parse::<axum::http::HeaderValue>().ok())
        .collect();
    if origins.is_empty() {
        tracing::error!(
            value = %raw,
            "KYMA_CORS_ALLOWED_ORIGINS set but contains no valid origins — \
             failing closed (no cross-origin requests allowed)"
        );
    }
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any);
    r.layer(cors)
}

/// Axum middleware that enforces per-database token scope for handlers that
/// resolve their target database from the `x-database` request header.
///
/// This is applied as a layer over routers in separate crates (e.g.
/// `kyma-ingest-rest`) that cannot take a `kyma-server` dependency.
/// The `Principal` is already inserted into request extensions by
/// `require_role_middleware` before this middleware runs.
pub async fn database_scope_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    // Resolve the database the same way the underlying handler will.
    let database = req
        .headers()
        .get("x-database")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_owned();

    if let Some(principal) = req.extensions().get::<crate::auth::Principal>() {
        if let Err((status, msg)) = crate::auth::check_database_scope(principal, &database) {
            let request_id = extract_request_id(req.headers());
            return error_response(status, "forbidden", &msg, &request_id);
        }
    }

    next.run(req).await
}

/// Axum middleware that rejects database-scoped principals on surfaces that
/// can address databases internally, bypassing per-handler scope checks:
/// Arrow Flight (`/flight/*` — tickets name databases), the agent
/// (`/v1/agent/*` — the model's tool loop picks databases), and MCP
/// (`/mcp` — same tool dispatch). Until scope enforcement exists inside
/// those services we fail closed: tokens carrying an `allowed_databases`
/// restriction get 403; unrestricted principals (and auth-disabled
/// deployments, whose synthesized Admin principal has
/// `allowed_databases: None`) are unaffected.
pub async fn scoped_token_guard_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(principal) = req.extensions().get::<crate::auth::Principal>() {
        if principal.allowed_databases.is_some() {
            let request_id = extract_request_id(req.headers());
            return error_response(
                axum::http::StatusCode::FORBIDDEN,
                "forbidden",
                "database-scoped tokens cannot use this interface yet",
                &request_id,
            );
        }
    }

    next.run(req).await
}

#[cfg(test)]
mod cors_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn make_app(origins_env: &str) -> Router {
        // Temporarily set the env var, build the router, then clear it.
        // Tests calling this are serialized via the CORS_TEST_MUTEX.
        std::env::set_var("KYMA_CORS_ALLOWED_ORIGINS", origins_env);
        let r = Router::new().route("/ping", axum::routing::get(|| async { "pong" }));
        let app = with_configured_cors(r);
        std::env::remove_var("KYMA_CORS_ALLOWED_ORIGINS");
        app
    }

    // Serialise env-var mutation across all cors_tests.
    static CORS_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn allowed_origin_gets_acao_header() {
        let _guard = CORS_TEST_MUTEX.lock().unwrap();
        let app = make_app("http://allowed.example.com, http://other.example.com");

        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/ping")
                    .header("origin", "http://allowed.example.com")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let acao = res
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            acao,
            Some("http://allowed.example.com"),
            "expected ACAO header for allowed origin"
        );
    }

    #[tokio::test]
    async fn disallowed_origin_gets_no_acao_header() {
        let _guard = CORS_TEST_MUTEX.lock().unwrap();
        let app = make_app("http://allowed.example.com");

        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/ping")
                    .header("origin", "http://evil.example.com")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // tower-http CORS layer simply omits the ACAO header for disallowed origins.
        let acao = res.headers().get("access-control-allow-origin");
        assert!(
            acao.is_none(),
            "expected no ACAO header for disallowed origin, got: {:?}",
            acao
        );
    }

    #[tokio::test]
    async fn set_but_invalid_origins_fail_closed_not_permissive() {
        let _guard = CORS_TEST_MUTEX.lock().unwrap();
        // A value with only invalid header values (newline is illegal) must
        // NOT fall back to permissive mirroring — no origin gets ACAO.
        let app = make_app("\n");

        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/ping")
                    .header("origin", "http://anything.example.com")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let acao = res.headers().get("access-control-allow-origin");
        assert!(
            acao.is_none(),
            "misconfigured allow-list must fail closed, got ACAO: {:?}",
            acao
        );
    }
}

#[cfg(test)]
mod scoped_token_guard_tests {
    use super::*;
    use crate::auth::{Principal, Role};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn principal(allowed: Option<Vec<&str>>) -> Principal {
        Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
            allowed_databases: allowed
                .map(|v| v.into_iter().map(String::from).collect()),
        }
    }

    /// Builds a guarded route with an injected principal (None = no auth ran,
    /// e.g. auth-disabled deployments before the middleware synthesizes one).
    fn app(p: Option<Principal>) -> Router {
        let inject = axum::middleware::from_fn(
            move |mut req: axum::extract::Request, next: axum::middleware::Next| {
                let p = p.clone();
                async move {
                    if let Some(p) = p {
                        req.extensions_mut().insert(p);
                    }
                    next.run(req).await
                }
            },
        );
        Router::new()
            .route("/flight/x", axum::routing::post(|| async { "ok" }))
            .layer(axum::middleware::from_fn(scoped_token_guard_middleware))
            .layer(inject)
    }

    async fn status_of(app: Router) -> StatusCode {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/flight/x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    #[tokio::test]
    async fn scoped_principal_is_rejected() {
        let s = status_of(app(Some(principal(Some(vec!["staging"]))))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unrestricted_principal_passes() {
        let s = status_of(app(Some(principal(None)))).await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_principal_passes() {
        // No Principal extension (auth fully disabled) — guard is a no-op.
        let s = status_of(app(None)).await;
        assert_eq!(s, StatusCode::OK);
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Debug, Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
    request_id: &'a str,
}

pub(crate) fn error_response(status: StatusCode, code: &str, message: &str, request_id: &str) -> Response {
    ::metrics::counter!("kyma_http_errors_total", "code" => code.to_string()).increment(1);
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code,
                message,
                request_id,
            },
        }),
    )
        .into_response()
}

pub(crate) fn resolve_query_budget(headers: &HeaderMap) -> kyma_core::query_frontend::QueryBudget {
    let mut b = kyma_core::query_frontend::QueryBudget::default();
    if let Some(v) = headers
        .get("x-kyma-max-wall-clock-ms")
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(ms) = v.parse::<u64>() {
            b.max_wall_clock = std::time::Duration::from_millis(ms.max(10));
        }
    }
    if let Some(v) = headers
        .get("x-kyma-max-memory-bytes")
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(n) = v.parse::<u64>() {
            b.max_memory_bytes = n.max(1024 * 1024);
        }
    }
    if let Some(v) = headers
        .get("x-kyma-max-object-store-bytes")
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(n) = v.parse::<u64>() {
            b.max_object_store_bytes = n;
        }
    }
    b
}

fn budget_exceeded_response(
    code: &str,
    message: &str,
    request_id: &str,
    limit: u64,
    unit: &str,
) -> Response {
    let mut resp = error_response(StatusCode::TOO_MANY_REQUESTS, code, message, request_id);
    let hdrs = resp.headers_mut();
    hdrs.insert("retry-after", HeaderValue::from_static("1"));
    if let Ok(h) = HeaderValue::from_str(&format!("{limit} {unit}")) {
        hdrs.insert("x-kyma-budget-limit", h);
    }
    resp
}

pub(crate) fn extract_request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Build a [`kyma_kql::SchemaMap`] from a slice of [`TableRef`]s.
///
/// Each entry maps the table name to its column names in schema order.
/// This is passed to [`kyma_kql::kql_to_sql_with_schemas`] so that KQL
/// `union` can compute the column superset for outer-by-name semantics.
pub(crate) fn build_schema_map(tables: &[TableRef]) -> kyma_kql::SchemaMap {
    tables
        .iter()
        .map(|t| {
            let cols = t.schema.fields().iter().map(|f| f.name().clone()).collect();
            (t.name.clone(), cols)
        })
        .collect()
}

async fn query_handler(State(state): State<QueryState>, req: Request) -> Response {
    let start = std::time::Instant::now();
    let (parts, body) = req.into_parts();
    let headers: &HeaderMap = &parts.headers;
    let request_id = extract_request_id(headers);
    let database = headers
        .get("x-database")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_owned();

    // Enforce per-database token scope if the principal is scoped.
    if let Some(principal) = parts.extensions.get::<crate::auth::Principal>() {
        if let Err((status, msg)) = crate::auth::check_database_scope(principal, &database) {
            return error_response(status, "forbidden", &msg, &request_id);
        }
    }

    let body_bytes: Bytes = match axum::body::to_bytes(body, 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("failed to read query body: {e}"),
                &request_id,
            );
        }
    };

    let raw = match std::str::from_utf8(&body_bytes) {
        Ok(s) => s.trim().to_owned(),
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "bad_encoding",
                "request body is not valid UTF-8",
                &request_id,
            );
        }
    };
    if raw.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "empty_query",
            "empty query body",
            &request_id,
        );
    }

    // Resolve query budget: headers override, else defaults.
    let budget = resolve_query_budget(headers);

    // 1. Load every table in the database and register them in a fresh
    //    DataFusion SessionContext. This costs one catalog round-trip per
    //    query in phase A; SessionContext-level caching lands later.
    //    Tables are listed first so KQL `union` can receive the full schema map.
    let tables = match state.catalog.list_tables_in_database(&database).await {
        Ok(t) => t,
        Err(e) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "database_not_found",
                &format!("failed to list tables in database {database}: {e}"),
                &request_id,
            )
        }
    };

    if tables.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            "database_empty",
            &format!("no tables in database {database}"),
            &request_id,
        );
    }

    // Content-Type routing between SQL and KQL frontends.
    // Build the schema map from the registered tables so `union` can compute
    // the column superset.
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/sql");
    let (language, sql) = if content_type.starts_with("application/x-kql") {
        let schemas = build_schema_map(&tables);
        match kyma_kql::kql_to_sql_with_schemas(&raw, &schemas) {
            Ok(s) => ("kql", s),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "kql_parse_error",
                    &format!("KQL parse: {e}"),
                    &request_id,
                );
            }
        }
    } else {
        ("sql", raw)
    };

    debug!(request_id = %request_id, database = %database, language, sql = %sql,
        budget_memory = budget.max_memory_bytes,
        budget_wall_ms = budget.max_wall_clock.as_millis() as u64,
        "query received");
    ::metrics::counter!("kyma_query_frontend_total", "lang" => language.to_string()).increment(1);

    // Build a SessionContext whose memory pool is bounded by the budget.
    let runtime = match RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(budget.max_memory_bytes as usize)))
        .build()
    {
        Ok(r) => Arc::new(r),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &format!("runtime env: {e}"),
                &request_id,
            );
        }
    };
    let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
    kyma_exec::register_vector_udfs(&ctx);
    for t in tables {
        let table_name = t.name.clone();
        let kyma_tbl: Arc<KymaTable> = match state.node_id {
            Some(nid) => Arc::new(KymaTable::with_node_id(
                t,
                state.catalog.clone(),
                state.format.clone(),
                nid,
                database.clone(),
            )),
            None => Arc::new(KymaTable::new(
                t,
                state.catalog.clone(),
                state.format.clone(),
            )),
        };
        if let Err(e) = ctx.register_table(&table_name, kyma_tbl) {
            error!(request_id = %request_id, table = %table_name, error = %e, "failed to register table");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &format!("failed to register table {table_name}: {e}"),
                &request_id,
            );
        }
    }

    // 2. Parse + execute the SQL. DataFusion returns a stream of Arrow
    //    RecordBatches. For phase A we collect and then serialize to NDJSON.
    let df = match ctx.sql(&sql).await {
        Ok(df) => df,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "sql_parse_error",
                &format!("SQL parse/plan: {e}"),
                &request_id,
            );
        }
    };
    // Enforce wall-clock budget: tokio::time::timeout cancels the future.
    let batches = match tokio::time::timeout(budget.max_wall_clock, df.collect()).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            // ResourcesExhausted from DataFusion signals memory-pool exhaustion.
            let msg = e.to_string();
            if msg.contains("ResourcesExhausted") || msg.contains("Resources exhausted") {
                ::metrics::counter!("kyma_query_budget_exceeded_total", "kind" => "memory")
                    .increment(1);
                return budget_exceeded_response(
                    "memory_exceeded",
                    &msg,
                    &request_id,
                    budget.max_memory_bytes,
                    "memory",
                );
            }
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_execution_error",
                &format!("query execution: {e}"),
                &request_id,
            );
        }
        Err(_elapsed) => {
            ::metrics::counter!("kyma_query_budget_exceeded_total", "kind" => "wall_clock")
                .increment(1);
            return budget_exceeded_response(
                "wall_clock_exceeded",
                &format!(
                    "query exceeded wall-clock budget of {}ms",
                    budget.max_wall_clock.as_millis()
                ),
                &request_id,
                budget.max_wall_clock.as_millis() as u64,
                "wall_clock_ms",
            );
        }
    };
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    info!(request_id = %request_id, database = %database, rows = total_rows, "query completed");

    ::metrics::counter!("kyma_query_requests_total",
        "database" => database.clone(), "result" => "ok")
    .increment(1);
    ::metrics::histogram!("kyma_query_duration_seconds", "database" => database.clone())
        .record(start.elapsed().as_secs_f64());
    ::metrics::histogram!("kyma_query_rows_returned", "database" => database.clone())
        .record(total_rows as f64);

    // 3. Serialize each batch into NDJSON and stream.
    let mut body_bytes: Vec<u8> = Vec::with_capacity(total_rows * 128);
    for batch in &batches {
        let mut writer = ArrayWriter::new(&mut body_bytes);
        if let Err(e) = writer.write(batch) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                &format!("result serialization: {e}"),
                &request_id,
            );
        }
        if let Err(e) = writer.finish() {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                &format!("result serialization finish: {e}"),
                &request_id,
            );
        }
    }

    let rows_ndjson = match collate_ndjson(&body_bytes) {
        Ok(s) => s,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                &format!("NDJSON collation: {e}"),
                &request_id,
            );
        }
    };

    let mut resp = Response::new(Body::from(rows_ndjson));
    let hdrs = resp.headers_mut();
    hdrs.insert(
        "content-type",
        HeaderValue::from_static("application/x-ndjson; charset=utf-8"),
    );
    hdrs.insert(
        "x-kyma-rows",
        HeaderValue::from_str(&total_rows.to_string()).unwrap(),
    );
    if let Ok(rid) = HeaderValue::from_str(&request_id) {
        hdrs.insert("x-request-id", rid);
    }
    resp
}

/// Convert a concatenation of `ArrayWriter`-emitted JSON arrays
/// (`[{...},{...}][{...}]...`) into newline-delimited JSON objects.
///
/// `serde_json::Deserializer::into_iter` streams successive JSON values
/// from the input — each JSON array we emitted becomes one `Value::Array`.
fn collate_ndjson(concatenated_arrays: &[u8]) -> Result<String, String> {
    let mut out = String::with_capacity(concatenated_arrays.len());
    let stream =
        serde_json::Deserializer::from_slice(concatenated_arrays).into_iter::<serde_json::Value>();
    for arr in stream {
        let arr = arr.map_err(|e| format!("json parse: {e}"))?;
        match arr {
            serde_json::Value::Array(rows) => {
                for row in rows {
                    out.push_str(&serde_json::to_string(&row).map_err(|e| e.to_string())?);
                    out.push('\n');
                }
            }
            other => {
                out.push_str(&serde_json::to_string(&other).map_err(|e| e.to_string())?);
                out.push('\n');
            }
        }
    }
    Ok(out)
}
