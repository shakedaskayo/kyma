//! Arrow Flight gRPC query surface.
//!
//! Implements [`FlightService`] with a minimum-viable `do_get` that
//! accepts a JSON ticket, executes the query via DataFusion, and streams
//! `RecordBatch`es back as `FlightData` — zero-copy from Arrow buffers
//! to the wire, no NDJSON or JSON-per-row overhead.
//!
//! # Ticket protocol
//!
//! A Flight ticket is an opaque `bytes`. We define a tiny JSON envelope:
//!
//! ```json
//! {"database": "default", "query": "SELECT * FROM t", "language": "sql"}
//! ```
//!
//! `language` is optional (`"sql"` default). Set `"kql"` for KQL queries.
//! Future: migrate to full Flight-SQL compliance for drop-in DuckDB /
//! DataFusion client support.
//!
//! # Not implemented (stubs return `Unimplemented`)
//!
//! - `handshake` — we accept unauthenticated for now (auth story lands
//!   on the gRPC surface in Phase F hardening).
//! - `get_flight_info` / `get_schema` — clients can skip straight to
//!   `do_get`.
//! - `do_put` — ingest-via-Flight is a future capability.
//! - `do_exchange`, `do_action`, `list_actions`, `list_flights`.

use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::flight_service_server::{FlightService, FlightServiceServer};
use arrow_flight::{
    Action, ActionType, Criteria, Empty, FlightData, FlightDescriptor, FlightInfo,
    HandshakeRequest, HandshakeResponse, PollInfo, PutResult, SchemaResult, Ticket,
};
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use futures::stream::{self, BoxStream, StreamExt};
use pensieve_core::catalog::Catalog;
use pensieve_core::segment_format::SegmentFormat;
use pensieve_exec::PensieveTable;
use std::sync::Arc;
use tonic::{Request, Response, Status, Streaming};
use tracing::debug;

/// State shared between the HTTP and gRPC query surfaces.
#[derive(Clone)]
pub struct FlightState {
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
    /// Current node's id. When set, queries arriving via Flight fan out
    /// to peers via the read-router (same logic as HTTP).
    pub node_id: Option<pensieve_core::types::NodeId>,
}

/// The Flight service implementation.
pub struct FlightQueryService {
    state: FlightState,
}

impl FlightQueryService {
    pub fn new(state: FlightState) -> Self {
        Self { state }
    }

    /// Handle an internal `kind:"extent"` ticket: open the named extent
    /// via the local segment format, decode every block, and stream the
    /// record batches. The caller (peer node's read-router) applies its
    /// own DataFusion filters above the scan, so we don't need to know
    /// the query predicate — we just deliver the raw bytes.
    async fn serve_extent(
        &self,
        ticket: &FlightTicket,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let table = self
            .state
            .catalog
            .lookup_table(&ticket.database, &ticket.table)
            .await
            .map_err(|e| Status::not_found(format!("lookup_table: {e}")))?;

        let reader = self
            .state
            .format
            .open_extent(pensieve_core::segment_format::OpenExtentInput {
                extent_id: pensieve_core::types::ExtentId::new(),
                table_id: table.id,
                schema: table.schema.clone(),
                object_path: ticket.object_path.clone(),
                byte_size: ticket.byte_size,
            })
            .await
            .map_err(|e| Status::internal(format!("open_extent: {e}")))?;

        let block_ids = reader
            .pruned_blocks(&pensieve_core::segment_format::BlockPredicate::All)
            .await
            .map_err(|e| Status::internal(format!("pruned_blocks: {e}")))?;

        let mut batches = Vec::with_capacity(block_ids.len());
        for bid in block_ids {
            let b = reader
                .read_block(bid, &[])
                .await
                .map_err(|e| Status::internal(format!("read_block: {e}")))?;
            batches.push(b);
        }

        ::metrics::counter!("pensieve_flight_serve_extent_total").increment(1);

        let s = stream::iter(
            batches
                .into_iter()
                .map(|b| Ok::<_, arrow_flight::error::FlightError>(b)),
        );
        let encoder = FlightDataEncoderBuilder::new()
            .build(s)
            .map(|r| r.map_err(|e| Status::internal(format!("encode: {e}"))))
            .boxed();
        Ok(Response::new(encoder))
    }
}

/// Client-facing ticket — `{database, query, language}` for user queries,
/// or `{kind:"extent", database, table, object_path, byte_size}` for
/// internal node-to-node extent fetches (read-fan-out router).
#[derive(Debug, serde::Deserialize)]
struct FlightTicket {
    /// "query" (default) or "extent".
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default = "default_database")]
    database: String,
    #[serde(default)]
    query: String,
    #[serde(default = "default_language")]
    language: String,
    // Only used when kind == "extent":
    #[serde(default)]
    table: String,
    #[serde(default)]
    object_path: String,
    #[serde(default)]
    byte_size: u64,
}

fn default_kind() -> String {
    "query".to_string()
}
fn default_database() -> String {
    "default".to_string()
}
fn default_language() -> String {
    "sql".to_string()
}

#[tonic::async_trait]
impl FlightService for FlightQueryService {
    // Stream types. `BoxStream<'static, …>` is idiomatic for Flight servers.
    type HandshakeStream = BoxStream<'static, Result<HandshakeResponse, Status>>;
    type ListFlightsStream = BoxStream<'static, Result<FlightInfo, Status>>;
    type DoGetStream = BoxStream<'static, Result<FlightData, Status>>;
    type DoPutStream = BoxStream<'static, Result<PutResult, Status>>;
    type DoActionStream = BoxStream<'static, Result<arrow_flight::Result, Status>>;
    type ListActionsStream = BoxStream<'static, Result<ActionType, Status>>;
    type DoExchangeStream = BoxStream<'static, Result<FlightData, Status>>;

    async fn handshake(
        &self,
        _req: Request<Streaming<HandshakeRequest>>,
    ) -> Result<Response<Self::HandshakeStream>, Status> {
        // Accept unauthenticated for MVP.
        let s = stream::empty::<Result<HandshakeResponse, Status>>().boxed();
        Ok(Response::new(s))
    }

    async fn list_flights(
        &self,
        _req: Request<Criteria>,
    ) -> Result<Response<Self::ListFlightsStream>, Status> {
        Err(Status::unimplemented(
            "list_flights not supported; issue do_get with a JSON ticket",
        ))
    }

    async fn get_flight_info(
        &self,
        _req: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        Err(Status::unimplemented(
            "get_flight_info not supported; issue do_get directly",
        ))
    }

    async fn get_schema(
        &self,
        _req: Request<FlightDescriptor>,
    ) -> Result<Response<SchemaResult>, Status> {
        Err(Status::unimplemented("get_schema not supported"))
    }

    async fn do_get(
        &self,
        request: Request<Ticket>,
    ) -> Result<Response<Self::DoGetStream>, Status> {
        let ticket = request.into_inner();
        let ticket: FlightTicket = serde_json::from_slice(&ticket.ticket)
            .map_err(|e| Status::invalid_argument(format!("bad ticket JSON: {e}")))?;

        // Internal node-to-node extent fetch from the read-router.
        if ticket.kind == "extent" {
            return self.serve_extent(&ticket).await;
        }

        debug!(db = %ticket.database, lang = %ticket.language, sql_len = ticket.query.len(), "flight do_get");

        // Load tables first so KQL `union` has schema context for the
        // column-superset computation.
        let tables = self
            .state
            .catalog
            .list_tables_in_database(&ticket.database)
            .await
            .map_err(|e| Status::not_found(format!("list_tables: {e}")))?;
        if tables.is_empty() {
            return Err(Status::not_found(format!(
                "no tables in database {}",
                ticket.database
            )));
        }

        // Translate to SQL if needed.
        let sql = match ticket.language.as_str() {
            "kql" => {
                let schemas = crate::build_schema_map(&tables);
                pensieve_kql::kql_to_sql_with_schemas(&ticket.query, &schemas)
                    .map_err(|e| Status::invalid_argument(format!("KQL parse: {e}")))?
            }
            "sql" => ticket.query,
            other => {
                return Err(Status::invalid_argument(format!(
                    "unknown language `{other}`; use `sql` or `kql`"
                )))
            }
        };

        // Build a one-shot SessionContext with a budget the same way HTTP does.
        let runtime = Arc::new(
            RuntimeEnvBuilder::new()
                .with_memory_pool(Arc::new(GreedyMemoryPool::new(4 * 1024 * 1024 * 1024)))
                .build()
                .map_err(|e| Status::internal(format!("runtime: {e}")))?,
        );
        let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
        pensieve_exec::register_vector_udfs(&ctx);
        for t in tables {
            let name = t.name.clone();
            let tbl: Arc<PensieveTable> = match self.state.node_id {
                Some(nid) => Arc::new(PensieveTable::with_node_id(
                    t,
                    self.state.catalog.clone(),
                    self.state.format.clone(),
                    nid,
                    ticket.database.clone(),
                )),
                None => Arc::new(PensieveTable::new(
                    t,
                    self.state.catalog.clone(),
                    self.state.format.clone(),
                )),
            };
            ctx.register_table(&name, tbl)
                .map_err(|e| Status::internal(format!("register_table {name}: {e}")))?;
        }

        let df = ctx
            .sql(&sql)
            .await
            .map_err(|e| Status::invalid_argument(format!("sql plan: {e}")))?;
        let stream = df
            .execute_stream()
            .await
            .map_err(|e| Status::internal(format!("execute: {e}")))?;

        // Convert the DataFusion RecordBatch stream into what
        // `FlightDataEncoderBuilder` wants: `Stream<Item=Result<RecordBatch, FlightError>>`.
        let mapped = stream
            .map(|r| r.map_err(|e| arrow_flight::error::FlightError::ExternalError(Box::new(e))));

        // The Arrow Flight encoder emits the schema header automatically,
        // then chunks each RecordBatch into one or more FlightData frames.
        let encoder = FlightDataEncoderBuilder::new()
            .build(mapped)
            .map(|r| r.map_err(|e| Status::internal(format!("encode: {e}"))))
            .boxed();

        ::metrics::counter!("pensieve_flight_do_get_total", "lang" => ticket.language).increment(1);
        Ok(Response::new(encoder))
    }

    async fn do_put(
        &self,
        _req: Request<Streaming<FlightData>>,
    ) -> Result<Response<Self::DoPutStream>, Status> {
        Err(Status::unimplemented(
            "do_put not supported; use POST /v1/ingest for now",
        ))
    }

    async fn do_action(
        &self,
        _req: Request<Action>,
    ) -> Result<Response<Self::DoActionStream>, Status> {
        Err(Status::unimplemented("do_action not supported"))
    }

    async fn list_actions(
        &self,
        _req: Request<Empty>,
    ) -> Result<Response<Self::ListActionsStream>, Status> {
        Err(Status::unimplemented("list_actions not supported"))
    }

    async fn do_exchange(
        &self,
        _req: Request<Streaming<FlightData>>,
    ) -> Result<Response<Self::DoExchangeStream>, Status> {
        Err(Status::unimplemented("do_exchange not supported"))
    }

    async fn poll_flight_info(
        &self,
        _req: Request<FlightDescriptor>,
    ) -> Result<Response<PollInfo>, Status> {
        Err(Status::unimplemented("poll_flight_info not supported"))
    }
}

/// Convenience: wrap the service in a `tonic` `Server` ready to serve.
pub fn flight_server(state: FlightState) -> FlightServiceServer<FlightQueryService> {
    FlightServiceServer::new(FlightQueryService::new(state))
}

/// Build an axum-compatible service that serves the Flight gRPC-web API.
///
/// Wraps `FlightQueryService` with `tonic_web::GrpcWebLayer` so browsers can
/// issue gRPC-web requests over HTTP/1.1.  Auth enforcement is the caller's
/// responsibility — typically by wrapping the returned service (or the enclosing
/// axum `Router`) with `axum::middleware::from_fn_with_state`.
///
/// The body type is adapted from tonic's `BoxBody` to `axum::body::Body`
/// so the service is compatible with `axum::Router::nest_service`.
#[cfg(feature = "web-ui")]
pub fn flight_grpc_web_service(state: FlightState) -> FlightGrpcWebService {
    use tower::ServiceBuilder;
    let svc = FlightServiceServer::new(FlightQueryService::new(state));
    let inner = ServiceBuilder::new()
        .layer(tonic_web::GrpcWebLayer::new())
        .service(svc);
    FlightGrpcWebService { inner }
}

/// Type-erased axum-compatible service wrapping the Flight gRPC-web stack.
#[cfg(feature = "web-ui")]
#[derive(Clone)]
pub struct FlightGrpcWebService {
    inner: tonic_web::GrpcWebService<FlightServiceServer<FlightQueryService>>,
}

#[cfg(feature = "web-ui")]
impl tower::Service<axum::http::Request<axum::body::Body>> for FlightGrpcWebService {
    type Response = axum::http::Response<axum::body::Body>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        tower::Service::poll_ready(&mut self.inner, cx).map_err(|e| match e {})
    }

    fn call(&mut self, req: axum::http::Request<axum::body::Body>) -> Self::Future {
        use http_body_util::BodyExt as _;
        // Convert axum body → tonic BoxBody.
        let (parts, body) = req.into_parts();
        let tonic_body: tonic::body::BoxBody = body
            .map_err(|e| tonic::Status::internal(e.to_string()))
            .boxed_unsync();
        let tonic_req = axum::http::Request::from_parts(parts, tonic_body);

        let fut = tower::Service::call(&mut self.inner, tonic_req);
        Box::pin(async move {
            // GrpcWebService<FlightServiceServer<_>> has Error = Infallible.
            #[allow(clippy::expect_used)]
            let resp = fut.await.expect("infallible");
            // Convert tonic BoxBody response → axum body response.
            let (parts, body) = resp.into_parts();
            let axum_body = axum::body::Body::new(body.map_err(axum::Error::new));
            Ok(axum::http::Response::from_parts(parts, axum_body))
        })
    }
}
