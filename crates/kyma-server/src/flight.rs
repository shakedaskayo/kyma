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

use arrow_array::RecordBatch;
use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::flight_service_server::{FlightService, FlightServiceServer};
use arrow_flight::{
    Action, ActionType, Criteria, Empty, FlightData, FlightDescriptor, FlightInfo,
    HandshakeRequest, HandshakeResponse, PollInfo, PutResult, SchemaResult, Ticket,
};
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::{RuntimeConfig, RuntimeEnv};
use datafusion::prelude::{SessionConfig, SessionContext};
use futures::stream::{self, BoxStream, StreamExt};
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_exec::KymaTable;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, error, info};

/// State shared between the HTTP and gRPC query surfaces.
#[derive(Clone)]
pub struct FlightState {
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
}

/// The Flight service implementation.
pub struct FlightQueryService {
    state: FlightState,
}

impl FlightQueryService {
    pub fn new(state: FlightState) -> Self {
        Self { state }
    }
}

#[derive(Debug, serde::Deserialize)]
struct FlightTicket {
    #[serde(default = "default_database")]
    database: String,
    query: String,
    #[serde(default = "default_language")]
    language: String,
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

        debug!(db = %ticket.database, lang = %ticket.language, sql_len = ticket.query.len(), "flight do_get");

        // Translate to SQL if needed.
        let sql = match ticket.language.as_str() {
            "kql" => kyma_kql::kql_to_sql(&ticket.query)
                .map_err(|e| Status::invalid_argument(format!("KQL parse: {e}")))?,
            "sql" => ticket.query,
            other => {
                return Err(Status::invalid_argument(format!(
                    "unknown language `{other}`; use `sql` or `kql`"
                )))
            }
        };

        // Build a one-shot SessionContext with a budget the same way HTTP does.
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
        let runtime = Arc::new(
            RuntimeEnv::new(
                RuntimeConfig::new()
                    .with_memory_pool(Arc::new(GreedyMemoryPool::new(4 * 1024 * 1024 * 1024))),
            )
            .map_err(|e| Status::internal(format!("runtime: {e}")))?,
        );
        let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
        for t in tables {
            let name = t.name.clone();
            let tbl = Arc::new(KymaTable::new(
                t,
                self.state.catalog.clone(),
                self.state.format.clone(),
            ));
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

        ::metrics::counter!("kyma_flight_do_get_total", "lang" => ticket.language).increment(1);
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

// --- workaround: fully-qualify the associated types so the trait's
// default associated-type bounds compile without pulling in extras ---
#[allow(dead_code)]
fn _assert_stream_bound<T: futures::Stream>(_: T) {}

// A Pin alias to avoid mouthful types in user code if this is ever split out.
#[allow(dead_code)]
type PinnedStream<T> = Pin<Box<dyn futures::Stream<Item = Result<T, Status>> + Send + 'static>>;
