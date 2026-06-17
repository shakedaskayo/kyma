//! The four inline tools wired into the kyma data-assistant agent.
//!
//! Each tool is a thin wrapper over [`kyma_core::catalog::Catalog`] +
//! DataFusion [`SessionContext`](datafusion::prelude::SessionContext) that
//! returns JSON suitable for ADK's `FunctionTool` contract.
//!
//! The tools intentionally hold minimal shared state via [`SharedToolCtx`]:
//!
//! - `list_databases` / `describe_table` — catalog-only.
//! - `run_sql` / `sample_rows` — build a fresh `SessionContext` per call,
//!   register every table in the requested database, and execute the query.
//!
//! All four tools return `serde_json::Value` and are never `panic!`-y: any
//! catalog / DataFusion error becomes a JSON `{"error": "..."}` payload so
//! the LLM can self-correct instead of aborting the run.

use adk_rust::tool::FunctionTool;
use adk_rust::{Tool, ToolContext};
use arrow::json::ArrayWriter;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;
use kyma_exec::KymaTable;
use kyma_ingest_core::{ConsumerAction, ConsumerActivity, ConsumerEvents};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;

/// Hard cap on per-tool memory to prevent a single agent turn from dragging
/// the server OOM. 256 MiB is generous for point-lookup / small-aggregation
/// style queries the agent produces while researching a question.
const TOOL_MEMORY_POOL_BYTES: usize = 256 * 1024 * 1024;

/// Shared, cheap-to-clone context passed into every tool handler closure.
#[derive(Clone)]
pub struct SharedToolCtx {
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
    /// Optional Postgres pool — used by graph-style tools (`find_references_to`)
    /// and to read per-tenant memory settings. `None` in **local single-binary
    /// mode** (`kyma local`), where there is no Postgres: pool-only tools
    /// degrade gracefully and memory settings fall back to defaults. The hot
    /// memory recall/save paths run entirely over the catalog + engine, so they
    /// work unchanged with `None`.
    pub pool: Option<PgPool>,
    /// Async memory ingest queue. `Some` ⇒ memory write tools enqueue and
    /// return immediately (batched embedding + bulk commits land in the
    /// background); read tools issue a realm-scoped flush barrier first so
    /// read-your-own-writes holds. `None` ⇒ the original synchronous write
    /// path (tests, `kyma sync`, `KYMA_MEMORY_ASYNC=0`).
    pub memory: Option<kyma_memory::MemoryQueue>,
    /// HITL approval chokepoint for *autonomous* memory mutations. `Some` only
    /// for the dreaming housekeeping toolset (built from the run's settings);
    /// `None` for the interactive agent — user-driven tool calls are already
    /// human-in-the-loop, so they apply directly. Wrapped in `Arc` so this ctx
    /// stays cheaply `Clone`.
    pub hitl: Option<std::sync::Arc<super::memory_gate::HitlGate>>,
    /// Live-proxy runtime for federated tables (Microsoft Fabric, …). `None`
    /// ⇒ `run_sql` against a database containing federated tables returns a
    /// clear error instead of empty results.
    pub federation: Option<std::sync::Arc<kyma_federation::FederationRuntime>>,
    /// Live consumer-activity sink. `Some` only on the interactive memory tool
    /// paths (MCP recall/search/remember, the HTTP memory routes) so the graph
    /// explorer's realtime overlay can see which agents touch which memories.
    /// `None` everywhere else (tests, sync jobs) — emits become no-ops.
    pub consumer_sink: Option<ConsumerSink>,
}

/// Pairs the consumer-activity broadcast bus with the tenant to stamp on
/// tool-path emits. The tool layer runs below the auth middleware, so there is
/// no `Principal` here — the tenant travels with the bus handle instead.
#[derive(Clone)]
pub struct ConsumerSink {
    pub events: ConsumerEvents,
    pub tenant: TenantId,
}

impl SharedToolCtx {
    /// Publish a consumer-activity event to the live bus, if one is wired.
    /// Best-effort and lossy — never blocks or fails the calling tool. The
    /// consumer `kind` is resolved from the process-global MCP client identity
    /// (`claude-code`, `cursor`, …), so even without an auth subject the
    /// overlay can attribute activity to the right agent.
    pub fn emit_consumer(
        &self,
        action: ConsumerAction,
        node_ids: Vec<String>,
        namespaces: Vec<String>,
        query_preview: Option<String>,
    ) {
        let Some(sink) = &self.consumer_sink else {
            return;
        };
        let kind = super::identity::consumer_kind();
        let ip = super::identity::peer_ip();
        let pid = super::identity::peer_pid();
        // Distinguish distinct client processes when we can (pid, else ip).
        let ident = pid
            .map(|p| p.to_string())
            .or_else(|| ip.clone())
            .unwrap_or_else(|| "anon".into());
        sink.events.publish(ConsumerActivity {
            consumer_id: format!("{kind}:{ident}"),
            label: kind.clone(),
            kind,
            subject: None,
            tenant: sink.tenant.to_string(),
            action,
            node_ids,
            namespaces,
            query_preview,
            ts: chrono::Utc::now().timestamp_millis(),
            host: Some(super::identity::host_name()),
            client_version: super::identity::client_version(),
            transport: super::identity::transport(),
            ip,
            pid,
        });
    }
}

impl SharedToolCtx {
    /// Realm-scoped flush barrier for read paths: waits (bounded) until every
    /// memory op queued to `realms` before this call has been attempted.
    /// Empty slice = all realms. No-op without a queue or when idle.
    pub async fn memory_barrier(&self, realms: &[String]) {
        if let Some(q) = &self.memory {
            if !q.barrier(realms).await {
                tracing::warn!(
                    ?realms,
                    "memory flush barrier timed out; read may miss just-queued writes"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool 1: list_databases
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct NoArgs {}

const LIST_DATABASES_DESC: &str = "List every database in the kyma cluster. \
Call first to discover what databases exist. \
Returns an array of database names.";

pub fn tool_list_databases(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "list_databases",
            LIST_DATABASES_DESC,
            move |_tc: Arc<dyn ToolContext>, _args: Value| {
                let shared = shared.clone();
                async move {
                    match shared.catalog.list_databases().await {
                        Ok(names) => Ok(json!({"databases": names})),
                        Err(e) => Ok(json!({"error": format!("list_databases: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<NoArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 2: describe_table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct DescribeArgs {
    /// Database name.
    database: String,
    /// Table name inside that database.
    table: String,
}

const DESCRIBE_TABLE_DESC: &str = "Describe the columns of a table: names, \
Arrow data types, nullability. Call this before writing a SQL query against \
an unfamiliar table.";

pub fn tool_describe_table(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "describe_table",
            DESCRIBE_TABLE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: DescribeArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    match shared
                        .catalog
                        .lookup_table(&parsed.database, &parsed.table)
                        .await
                    {
                        Ok(t) => {
                            let cols: Vec<Value> = t
                                .schema
                                .fields()
                                .iter()
                                .map(|f| {
                                    json!({
                                        "name": f.name(),
                                        "type": format!("{:?}", f.data_type()),
                                        "nullable": f.is_nullable(),
                                    })
                                })
                                .collect();
                            let mut out = json!({
                                "database": parsed.database,
                                "table": parsed.table,
                                "columns": cols,
                            });
                            if let Some(fed) = t.config.federated.as_ref() {
                                // Tell the agent this table is live-proxied:
                                // queries run on the remote platform's compute.
                                out["federated"] = json!({
                                    "platform": fed.platform,
                                    "remote": format!(
                                        "{}/{}.{}",
                                        fed.remote_database, fed.remote_schema, fed.remote_table
                                    ),
                                });
                            }
                            Ok(out)
                        }
                        Err(e) => Ok(json!({"error": format!("lookup_table: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<DescribeArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 3: run_sql
// ---------------------------------------------------------------------------

fn default_max_rows() -> usize {
    200
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RunSqlArgs {
    /// Database whose tables should be registered into the DataFusion
    /// session for this query.
    database: String,
    /// Full SQL query text. Only SELECT / SHOW / EXPLAIN are accepted.
    sql: String,
    /// Row cap applied to the JSON response. Default 200.
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

const RUN_SQL_DESC: &str = "Execute a read-only SQL query via DataFusion. \
Use cosine_distance / l2_distance UDFs for vector similarity. \
Returns up to max_rows (default 200) rows as JSON. \
Queries that modify data are rejected (SELECT only; SHOW/EXPLAIN also allowed).";

pub fn tool_run_sql(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "run_sql",
            RUN_SQL_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: RunSqlArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    if !is_read_only_sql(&parsed.sql) {
                        return Ok(json!({
                            "error": "only SELECT / SHOW / EXPLAIN supported",
                        }));
                    }
                    Ok(execute_sql(&shared, &parsed.database, &parsed.sql, parsed.max_rows).await)
                }
            },
        )
        .with_parameters_schema::<RunSqlArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 4: run_kql — kyma's native query surface
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RunKqlArgs {
    /// Database whose tables should be registered into the DataFusion
    /// session for this query.
    database: String,
    /// KQL query text. Starts with a table reference, pipe-separated operators:
    /// `tbl | where cond | summarize agg by col | take N`.
    kql: String,
    /// Row cap applied to the JSON response. Default 200.
    #[serde(default = "default_max_rows")]
    max_rows: usize,
}

const RUN_KQL_DESC: &str = "Execute a KQL query against kyma — the PRIMARY \
query tool. KQL is pipe-syntax: \
`requests | where status >= 500 | summarize n=count() by url | top 10 by n`. \
Supports: where, project, project-away, extend, summarize…by…, take, limit, \
sort, top, count, distinct, graph-traverse, graph-shortest-path. Functions: \
now, ago, bin, startofhour/day, strcat, tolower, iff, count, sum, avg, min, \
max, dcount. String ops: contains, startswith, endswith, has. \
For vector similarity the operator is not yet wired — drop to run_sql with \
cosine_distance(col, make_array(...)).";

pub fn tool_run_kql(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "run_kql",
            RUN_KQL_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: RunKqlArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    // Compile KQL → SQL with full schema context so that
                    // `union` can compute the column superset; surface parse
                    // errors to the model so it can self-correct the syntax.
                    let sql = match kql_to_sql_for_database(&shared, &parsed.database, &parsed.kql)
                        .await
                    {
                        Ok(s) => s,
                        Err(e) => return Ok(e),
                    };
                    let mut out =
                        execute_sql(&shared, &parsed.database, &sql, parsed.max_rows).await;
                    // Attach the compiled SQL for debugging / tracing.
                    if let Value::Object(ref mut m) = out {
                        m.insert("compiled_sql".into(), Value::String(sql));
                    }
                    Ok(out)
                }
            },
        )
        .with_parameters_schema::<RunKqlArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 5: sample_rows
// ---------------------------------------------------------------------------

fn default_n() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SampleArgs {
    database: String,
    table: String,
    #[serde(default = "default_n")]
    n: usize,
}

const SAMPLE_ROWS_DESC: &str = "Fetch N representative rows from a table. \
Use when describe_table's column types aren't enough to understand the data \
shape (e.g. JSON/dynamic columns, text formats).";

pub fn tool_sample_rows(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "sample_rows",
            SAMPLE_ROWS_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: SampleArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    if !is_safe_ident(&parsed.database) || !is_safe_ident(&parsed.table) {
                        return Ok(json!({
                            "error": "database and table must be ascii-alphanumeric \
                                / underscore only",
                        }));
                    }
                    let n = parsed.n.max(1).min(1000);
                    let sql = format!(
                        "SELECT * FROM {}.{} LIMIT {}",
                        parsed.database, parsed.table, n,
                    );
                    Ok(execute_sql(&shared, &parsed.database, &sql, n).await)
                }
            },
        )
        .with_parameters_schema::<SampleArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn is_read_only_sql(sql: &str) -> bool {
    let t = sql.trim_start().to_lowercase();
    t.starts_with("select")
        || t.starts_with("show")
        || t.starts_with("explain")
        || t.starts_with("with ")
}

fn is_safe_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Compile a KQL query to SQL, using the catalog-backed schema of `database`
/// so that `union` and other schema-aware operators work correctly.
///
/// Returns `Err(json)` on catalog errors or KQL parse errors so the caller
/// can return the error JSON directly to the model.
async fn kql_to_sql_for_database(
    shared: &SharedToolCtx,
    database: &str,
    kql: &str,
) -> Result<String, Value> {
    let tables = shared
        .catalog
        .list_tables_in_database(database)
        .await
        .map_err(|e| json!({"error": format!("list_tables_in_database({database}): {e}")}))?;
    let schemas: kyma_kql::SchemaMap = tables
        .iter()
        .map(|t| {
            let cols = t.schema.fields().iter().map(|f| f.name().clone()).collect();
            (t.name.clone(), cols)
        })
        .collect();
    kyma_kql::kql_to_sql_with_schemas(kql, &schemas)
        .map_err(|e| json!({"error": format!("kql_parse: {e}"), "hint": "Check pipe syntax; operators are '|'-separated, strings are double-quoted, comparisons use '=='."}))
}

/// Build a fresh [`SessionContext`], register every table in `database`,
/// execute `sql`, and return a JSON envelope `{columns, rows, truncated}`
/// (or `{error}` on failure — never an `Err`, because tool failures should
/// be surfaced to the model as data rather than abort the run).
pub async fn execute_sql(
    shared: &SharedToolCtx,
    database: &str,
    sql: &str,
    max_rows: usize,
) -> Value {
    // Emit a telemetry child span only when the caller is already inside a
    // kyma_telemetry span (request handler, tool.call, memory.search.*).
    // Background workers (memory queue, dreaming, cc-curation) call this too,
    // and the OTel layer — which only sees kyma_telemetry spans — would export
    // theirs as parentless single-span `sql.execute` roots: pure noise on the
    // Traces page.
    let inside_telemetry = tracing::Span::current()
        .metadata()
        .is_some_and(|m| m.target() == "kyma_telemetry");
    if !inside_telemetry {
        return execute_sql_inner(shared, database, sql, max_rows).await;
    }
    let span = tracing::info_span!(
        target: "kyma_telemetry",
        "sql.execute",
        sql.database = %database,
    );
    tracing::Instrument::instrument(execute_sql_inner(shared, database, sql, max_rows), span).await
}

async fn execute_sql_inner(
    shared: &SharedToolCtx,
    database: &str,
    sql: &str,
    max_rows: usize,
) -> Value {
    let tables = match shared.catalog.list_tables_in_database(database).await {
        Ok(t) => t,
        Err(e) => {
            return json!({"error": format!("list_tables_in_database({database}): {e}")});
        }
    };
    if tables.is_empty() {
        return json!({"error": format!("database `{database}` has no tables or does not exist")});
    }
    let runtime = match RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(TOOL_MEMORY_POOL_BYTES)))
        .build()
    {
        Ok(r) => Arc::new(r),
        Err(e) => return json!({"error": format!("runtime_env: {e}")}),
    };
    let (federated, local): (Vec<_>, Vec<_>) = tables
        .into_iter()
        .partition(|t| t.config.federated.is_some());
    let ctx = if federated.is_empty() {
        SessionContext::new_with_config_rt(SessionConfig::new(), runtime)
    } else {
        kyma_federation::federated_session_context(SessionConfig::new(), runtime)
    };
    kyma_exec::register_vector_udfs(&ctx);
    if !federated.is_empty() {
        let Some(fed_rt) = shared.federation.as_ref() else {
            return json!({"error": format!(
                "database `{database}` contains federated (live-proxied) tables but this server has no federation runtime"
            )});
        };
        // The agent path is tenant-scoped at the transport layer; tools run
        // under the default tenant like the rest of SharedToolCtx.
        let providers = match fed_rt
            .federated_providers(kyma_core::tenant::DEFAULT_TENANT, &federated)
            .await
        {
            Ok(p) => p,
            Err(e) => return json!({"error": format!("federated_providers: {e}")}),
        };
        for (name, provider) in providers {
            if let Err(e) = ctx.register_table(&name, provider) {
                return json!({"error": format!("register_table({name}): {e}")});
            }
        }
    }
    for t in local {
        let name = t.name.clone();
        let table = Arc::new(KymaTable::new(
            t,
            shared.catalog.clone(),
            shared.format.clone(),
        ));
        if let Err(e) = ctx.register_table(&name, table) {
            return json!({"error": format!("register_table({name}): {e}")});
        }
    }

    let df = match ctx.sql(sql).await {
        Ok(df) => df,
        Err(e) => return json!({"error": format!("sql_plan: {e}")}),
    };
    // Schema first so we can surface column order even on empty results.
    let schema = df.schema().clone();
    let batches = match df.collect().await {
        Ok(b) => b,
        Err(e) => return json!({"error": format!("sql_exec: {e}")}),
    };

    let columns: Vec<Value> = schema
        .fields()
        .iter()
        .map(|f| {
            json!({
                "name": f.name(),
                "type": format!("{:?}", f.data_type()),
            })
        })
        .collect();

    // NDJSON-assembly pattern copied from kyma_server::lib's query_handler.
    // We serialize each batch into a JSON array via arrow::json::ArrayWriter,
    // then re-parse and flatten into individual row objects. Cap at max_rows.
    let mut rows: Vec<Value> = Vec::new();
    let mut truncated = false;
    'outer: for batch in &batches {
        let mut buf: Vec<u8> = Vec::with_capacity(batch.num_rows() * 128);
        {
            let mut writer = ArrayWriter::new(&mut buf);
            if let Err(e) = writer.write(batch) {
                return json!({"error": format!("serialize: {e}")});
            }
            if let Err(e) = writer.finish() {
                return json!({"error": format!("serialize_finish: {e}")});
            }
        }
        let parsed: serde_json::Result<Value> = serde_json::from_slice(&buf);
        match parsed {
            Ok(Value::Array(arr)) => {
                for row in arr {
                    if rows.len() >= max_rows {
                        truncated = true;
                        break 'outer;
                    }
                    rows.push(row);
                }
            }
            Ok(other) => rows.push(other),
            Err(e) => return json!({"error": format!("reparse: {e}")}),
        }
    }

    json!({
        "columns": columns,
        "rows": rows,
        "row_count": rows.len(),
        "truncated": truncated,
    })
}

// ---------------------------------------------------------------------------
// Tool 6: explore_schema — one-shot "context graph" view of a database
// ---------------------------------------------------------------------------
//
// Returns every table in `database` with columns + types + a handful of
// sample values per column. Lets the agent reason about cross-table
// relationships in a SINGLE tool call instead of N round-trips to
// describe_table. The agent prompt recommends this as the first step
// for any question that touches multiple entities.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ExploreSchemaArgs {
    /// Database whose schema graph to surface.
    database: String,
    /// Number of sample values per column (cap 10; default 3).
    #[serde(default = "default_samples_per_column")]
    samples_per_column: usize,
}

fn default_samples_per_column() -> usize {
    3
}

const EXPLORE_SCHEMA_DESC: &str = "Return the full schema of a database in \
one call: every table, every column, types, and a few sample values per \
column. Use this FIRST for any question that spans multiple tables or \
when you don't yet know how entities relate. Much cheaper than calling \
list_databases + describe_table per table.";

pub fn tool_explore_schema(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "explore_schema",
            EXPLORE_SCHEMA_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: ExploreSchemaArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let n_samples = parsed.samples_per_column.min(10).max(0);
                    let tables = match shared
                        .catalog
                        .list_tables_in_database(&parsed.database)
                        .await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            return Ok(json!({
                                "error": format!("list_tables_in_database: {e}"),
                            }));
                        }
                    };
                    let mut out_tables: Vec<Value> = Vec::with_capacity(tables.len());
                    for t in &tables {
                        let cols: Vec<Value> = t
                            .schema
                            .fields()
                            .iter()
                            .map(|f| {
                                json!({
                                    "name": f.name(),
                                    "type": format!("{:?}", f.data_type()),
                                    "nullable": f.is_nullable(),
                                })
                            })
                            .collect();
                        // Sample values — single SELECT * LIMIT n per table.
                        let mut samples_by_col: serde_json::Map<String, Value> =
                            serde_json::Map::new();
                        if n_samples > 0 && !is_safe_ident(&t.name) {
                            // Defensive — should never happen for catalog-origin names,
                            // but cheap to check.
                            samples_by_col.insert(
                                "__error".into(),
                                json!(format!("unsafe table name: {}", t.name)),
                            );
                        } else if n_samples > 0 {
                            let sql = format!(
                                "SELECT * FROM {}.{} LIMIT {}",
                                parsed.database, t.name, n_samples
                            );
                            let sampled = execute_sql(&shared, &parsed.database, &sql, n_samples)
                                .await;
                            if let Some(rows) = sampled.get("rows").and_then(|v| v.as_array()) {
                                for f in t.schema.fields() {
                                    let col = f.name();
                                    let vals: Vec<Value> = rows
                                        .iter()
                                        .filter_map(|r| r.get(col).cloned())
                                        .collect();
                                    samples_by_col.insert(col.clone(), Value::Array(vals));
                                }
                            }
                        }
                        out_tables.push(json!({
                            "name": t.name,
                            "columns": cols,
                            "sample_values": samples_by_col,
                        }));
                    }
                    Ok(json!({
                        "database": parsed.database,
                        "tables": out_tables,
                        "table_count": tables.len(),
                        "hint": "Columns whose sample values look like ids ('abc-123', uuid-shapes, etc.) \
                                are likely foreign-key candidates — try find_references_to or \
                                cross-table joins.",
                    }))
                }
            },
        )
        .with_parameters_schema::<ExploreSchemaArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 7: find_references_to — live relationship traversal across the catalog
// ---------------------------------------------------------------------------
//
// Given a value (e.g. a user id, a request id, a URL), scan the catalog's
// extent-level column_stats JSONB index and return every (database, table,
// column) where that value appears in at least one extent's distinct set.
//
// This is the "what else knows about X" primitive. It uses indexes already
// in place (the equality-index work), so it's fast — a single SQL query
// against `extents.column_stats`. No full-scan.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct FindReferencesArgs {
    /// Database to search within. Pass an empty string or omit for cluster-wide.
    #[serde(default)]
    database: Option<String>,
    /// Value to locate across all columns.
    value: String,
}

/// Locate every `(database, table, column)` where `value` appears in the
/// catalog's extent-level distinct-value index (`extents.column_stats`).
///
/// Fast: a single indexed Postgres query against the metadata, no columnar
/// scan. Shared by the `find_references_to` tool and the memory entity
/// resolver ([`super::memory_resolve`]) so both stay consistent. `database`
/// `None` searches cluster-wide.
pub(crate) async fn find_references(
    pool: &PgPool,
    database: Option<&str>,
    value: &str,
) -> std::result::Result<Vec<(String, String, String)>, String> {
    // `column_stats` is shaped per-column as
    //     { "<col>": { "distinct": [values...], "tokens": [...] } }
    // so the containment test targets `kv.value -> 'distinct'`. For
    // numeric-looking values we also try a JSON-number cast so ids ingested
    // as integers still match.
    let target_str = serde_json::to_string(&vec![value]).unwrap();
    let target_num = value.parse::<f64>().ok().map(|n| format!("[{n}]"));
    let sql = r#"
        SELECT DISTINCT
            db.name  AS database_name,
            t.name   AS table_name,
            kv.key   AS column_name
        FROM extents e
        JOIN tables t      ON e.table_id    = t.id
        JOIN databases db  ON t.database_id = db.id
        CROSS JOIN LATERAL jsonb_each(e.column_stats) kv
        WHERE ($1::text IS NULL OR db.name = $1)
          AND (
                (kv.value -> 'distinct') @> $2::jsonb
                OR ($3::text IS NOT NULL
                    AND (kv.value -> 'distinct') @> $3::jsonb)
              )
        ORDER BY db.name, t.name, kv.key
        LIMIT 200
    "#;
    sqlx::query_as(sql)
        .bind(database)
        .bind(&target_str)
        .bind(target_num.as_deref())
        .fetch_all(pool)
        .await
        .map_err(|e| format!("pg_query: {e}"))
}

const FIND_REFERENCES_DESC: &str = "Find every (database, table, column) \
where a given value appears in the catalog's distinct-value index. The \
relationship-traversal primitive — use when the user asks 'what else \
references X?' or 'where does X show up?'. Returns a compact list of \
matches suitable for follow-up queries.";

pub fn tool_find_references_to(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "find_references_to",
            FIND_REFERENCES_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: FindReferencesArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    // This tool reads the catalog's column_stats index directly
                    // over Postgres; in local mode there is no pool. Recall +
                    // graph_traverse cover the same intent over the engine.
                    let Some(pool) = shared.pool.as_ref() else {
                        return Ok(json!({
                            "error": "find_references_to is unavailable in local mode; \
                                      use memory_search or graph_traverse instead",
                        }));
                    };
                    let rows = match find_references(
                        pool,
                        parsed.database.as_deref(),
                        &parsed.value,
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(e) => return Ok(json!({"error": e})),
                    };
                    let matches: Vec<Value> = rows
                        .into_iter()
                        .map(|(db, tbl, col)| {
                            json!({
                                "database": db, "table": tbl, "column": col,
                            })
                        })
                        .collect();
                    Ok(json!({
                        "value": parsed.value,
                        "matches": matches,
                        "match_count": matches.len(),
                        "hint": "For each match, you can call run_kql to fetch the rows: \
                                `<table> | where <column> == \"<value>\"`",
                    }))
                }
            },
        )
        .with_parameters_schema::<FindReferencesArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool 8: graph_traverse — wrap the existing KQL graph-traverse operator
// ---------------------------------------------------------------------------
//
// Thin wrapper over the KQL `graph-traverse` operator for tables that store
// graph edges as rows. Lets the agent explore connectivity without knowing
// KQL syntax for the operator.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct GraphTraverseArgs {
    database: String,
    /// Table whose rows represent edges (each row has a source and target).
    edges_table: String,
    /// Starting node value.
    source: String,
    /// Column in `edges_table` that holds the source node id.
    from_column: String,
    /// Column in `edges_table` that holds the target node id.
    to_column: String,
    /// Maximum hops. Default 5, cap 20.
    #[serde(default = "default_max_hops")]
    max_hops: usize,
    /// "forward" (default), "backward", or "both".
    #[serde(default = "default_direction")]
    direction: String,
}

fn default_max_hops() -> usize {
    5
}
fn default_direction() -> String {
    "forward".to_string()
}

const GRAPH_TRAVERSE_DESC: &str = "Traverse a graph stored as edges in a \
kyma table. Wraps the KQL `graph-traverse` operator. Returns reachable \
nodes as (node, depth) pairs. Use for connectivity questions: 'what \
services depend on X?', 'which users trigger Y?'.";

pub fn tool_graph_traverse(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "graph_traverse",
            GRAPH_TRAVERSE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: GraphTraverseArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    if !is_safe_ident(&parsed.edges_table)
                        || !is_safe_ident(&parsed.from_column)
                        || !is_safe_ident(&parsed.to_column)
                    {
                        return Ok(json!({
                            "error": "edges_table / from_column / to_column must be \
                                ascii-alphanumeric / underscore only",
                        }));
                    }
                    let hops = parsed.max_hops.clamp(1, 20);
                    let dir = match parsed.direction.as_str() {
                        "forward" | "backward" | "both" => parsed.direction.as_str(),
                        _ => {
                            return Ok(json!({
                                "error": "direction must be forward | backward | both",
                            }));
                        }
                    };
                    let kql = format!(
                        "{} | graph-traverse source \"{}\" from {} to {} \
                         max-hops {} direction {}",
                        parsed.edges_table,
                        parsed.source.replace('"', "\\\""),
                        parsed.from_column,
                        parsed.to_column,
                        hops,
                        dir,
                    );
                    let sql = match kql_to_sql_for_database(&shared, &parsed.database, &kql).await {
                        Ok(s) => s,
                        Err(e) => {
                            let mut err = e;
                            if let Value::Object(ref mut m) = err {
                                m.insert("kql".into(), Value::String(kql));
                            }
                            return Ok(err);
                        }
                    };
                    let mut out = execute_sql(&shared, &parsed.database, &sql, 1000).await;
                    if let Value::Object(ref mut m) = out {
                        m.insert("compiled_sql".into(), Value::String(sql));
                        m.insert("compiled_kql".into(), Value::String(kql));
                    }
                    Ok(out)
                }
            },
        )
        .with_parameters_schema::<GraphTraverseArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool: graph_analytics — PageRank centrality / communities / components
// ---------------------------------------------------------------------------

const GRAPH_ANALYTICS_DESC: &str =
    "Whole-graph analytics over a registered graph's topology: PageRank centrality, \
     community detection (modularity), or weakly-connected components. Read-only. Use to \
     find the most central memories/entities, cluster related context, or spot disconnected \
     sub-graphs. Returns {kind, count, results:[{id, value}]}.";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct GraphAnalyticsArgs {
    /// Database the graph is registered in.
    database: String,
    /// Registered graph name.
    graph: String,
    /// "pagerank" (centrality), "communities", or "components".
    kind: String,
    /// PageRank restart seeds (node ids). Empty ⇒ global PageRank (every node a
    /// restart source). Ignored by communities / components.
    #[serde(default)]
    seeds: Vec<String>,
    /// Top-N results for pagerank (descending score). Default 50, cap 100000.
    #[serde(default = "default_analytics_limit")]
    limit: usize,
}

fn default_analytics_limit() -> usize {
    50
}

pub fn tool_graph_analytics(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "graph_analytics",
            GRAPH_ANALYTICS_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: GraphAnalyticsArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    // Resolve the registered graph by (database, name).
                    let regs = shared
                        .catalog
                        .list_graphs(&parsed.database)
                        .await
                        .unwrap_or_default();
                    let Some(reg) = regs.into_iter().find(|r| r.name == parsed.graph) else {
                        return Ok(json!({
                            "error": format!("graph not found: {}/{}", parsed.database, parsed.graph),
                        }));
                    };
                    let provider = crate::graph_handler::stored_provider(
                        &shared.catalog,
                        &shared.format,
                        reg,
                    );
                    let limit = parsed.limit.clamp(1, 100_000);
                    let pairs: anyhow::Result<Vec<(String, Value)>> = match parsed.kind.as_str() {
                        "components" => provider
                            .components()
                            .await
                            .map(|v| v.into_iter().map(|(id, c)| (id, json!(c))).collect()),
                        "communities" => provider
                            .communities()
                            .await
                            .map(|v| v.into_iter().map(|(id, c)| (id, json!(c))).collect()),
                        "pagerank" => provider
                            .pagerank(&parsed.seeds, limit)
                            .await
                            .map(|v| v.into_iter().map(|(id, s)| (id, json!(s))).collect()),
                        other => {
                            return Ok(json!({
                                "error": format!(
                                    "unknown kind `{other}` (pagerank|communities|components)"
                                ),
                            }));
                        }
                    };
                    match pairs {
                        Ok(items) => {
                            let results: Vec<Value> = items
                                .into_iter()
                                .map(|(id, value)| json!({"id": id, "value": value}))
                                .collect();
                            Ok(json!({
                                "kind": parsed.kind, "count": results.len(), "results": results,
                            }))
                        }
                        Err(e) => Ok(json!({"error": format!("analytics: {e}")})),
                    }
                }
            },
        )
        .with_parameters_schema::<GraphAnalyticsArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// Tool: retrieve_artifact — fetch a byte window of a stored log/file blob
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RetrieveArtifactArgs {
    /// Object-store path of the artifact, e.g. the `object_path` from a
    /// `github_job_logs` row or a `LogFile` graph node. Must start with
    /// `artifacts/`.
    object_path: String,
    /// Byte offset to start reading from. Default 0.
    #[serde(default)]
    offset: u64,
    /// Max bytes to return (server-capped at 4 MiB). Default 65536. Page large
    /// logs by advancing `offset` until `eof` is true.
    limit: Option<u64>,
}

const RETRIEVE_ARTIFACT_DESC: &str = "Fetch a byte window of a stored artifact \
(a full CI job log, contributed file, or fs-watch snapshot) from the object \
store by its object_path. Discover the path first via run_kql/run_sql over \
github_job_logs (column object_path) or a LogFile graph node. Returns a slice \
of `content` plus `eof`; page large logs by advancing `offset`. Bytes are \
already secret-redacted at ingest.";

const ARTIFACT_DEFAULT_LIMIT: u64 = 64 * 1024;
const ARTIFACT_MAX_LIMIT: u64 = 4 * 1024 * 1024;

pub fn tool_retrieve_artifact(store: Arc<dyn object_store::ObjectStore>) -> Arc<dyn Tool> {
    Arc::new(
        FunctionTool::new(
            "retrieve_artifact",
            RETRIEVE_ARTIFACT_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let store = store.clone();
                async move {
                    let parsed: RetrieveArtifactArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    // Defense-in-depth: only the artifact namespace is readable —
                    // never extent segments or arbitrary object keys.
                    if !parsed.object_path.starts_with("artifacts/") {
                        return Ok(json!({"error": "object_path must start with 'artifacts/'"}));
                    }
                    let limit = parsed.limit.unwrap_or(ARTIFACT_DEFAULT_LIMIT).min(ARTIFACT_MAX_LIMIT) as usize;
                    let offset = parsed.offset as usize;
                    let path = object_store::path::Path::from(parsed.object_path.as_str());
                    let size = match store.head(&path).await {
                        Ok(meta) => meta.size,
                        Err(object_store::Error::NotFound { .. }) => {
                            return Ok(json!({"error": "artifact not found", "object_path": parsed.object_path}));
                        }
                        Err(e) => return Ok(json!({"error": format!("head: {e}")})),
                    };
                    // raw bytes read (end - offset), NOT content.len(): from_utf8_lossy
                    // expands invalid/boundary-split bytes to U+FFFD (3 bytes), which
                    // would corrupt the agent's offset paging.
                    let (nbytes, content) = if offset >= size {
                        (0usize, String::new())
                    } else {
                        let end = offset.saturating_add(limit).min(size);
                        match store.get_range(&path, offset..end).await {
                            Ok(bytes) => (bytes.len(), String::from_utf8_lossy(&bytes).into_owned()),
                            Err(e) => return Ok(json!({"error": format!("read: {e}")})),
                        }
                    };
                    Ok(json!({
                        "object_path": parsed.object_path,
                        "size_bytes": size,
                        "offset": offset,
                        "returned_bytes": nbytes,
                        "eof": offset.saturating_add(nbytes) >= size,
                        "content": content,
                    }))
                }
            },
        )
        .with_parameters_schema::<RetrieveArtifactArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

#[cfg(test)]
mod retrieve_artifact_tests {
    use super::*;
    use adk_rust::tool::SimpleToolContext;
    use object_store::memory::InMemory;
    use object_store::path::Path as ObjPath;

    #[tokio::test]
    async fn reads_window_guards_prefix_and_handles_missing() {
        let store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
        store
            .put(
                &ObjPath::from("artifacts/t/x.log"),
                b"0123456789".to_vec().into(),
            )
            .await
            .unwrap();
        let tool = tool_retrieve_artifact(store);
        let ctx = Arc::new(SimpleToolContext::new("test"));

        // Byte-window read.
        let out = tool
            .execute(
                ctx.clone(),
                json!({"object_path": "artifacts/t/x.log", "offset": 2, "limit": 3}),
            )
            .await
            .unwrap();
        assert_eq!(out["content"], "234");
        assert_eq!(out["eof"], false);
        assert_eq!(out["size_bytes"], 10);

        // Reading to the end reports eof.
        let tail = tool
            .execute(
                ctx.clone(),
                json!({"object_path": "artifacts/t/x.log", "offset": 8}),
            )
            .await
            .unwrap();
        assert_eq!(tail["content"], "89");
        assert_eq!(tail["eof"], true);

        // Prefix guard: only the artifact namespace is readable.
        let bad = tool
            .execute(
                ctx.clone(),
                json!({"object_path": "extents/secret-segment"}),
            )
            .await
            .unwrap();
        assert!(bad["error"].as_str().unwrap().contains("artifacts/"));

        // Missing artifact.
        let missing = tool
            .execute(ctx, json!({"object_path": "artifacts/t/nope.log"}))
            .await
            .unwrap();
        assert!(missing["error"].as_str().unwrap().contains("not found"));
    }
}
