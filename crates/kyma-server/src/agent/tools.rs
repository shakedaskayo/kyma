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
use datafusion::execution::runtime_env::{RuntimeConfig, RuntimeEnv};
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_exec::KymaTable;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
                            Ok(json!({
                                "database": parsed.database,
                                "table": parsed.table,
                                "columns": cols,
                            }))
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
// Tool 4: sample_rows
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

/// Build a fresh [`SessionContext`], register every table in `database`,
/// execute `sql`, and return a JSON envelope `{columns, rows, truncated}`
/// (or `{error}` on failure — never an `Err`, because tool failures should
/// be surfaced to the model as data rather than abort the run).
async fn execute_sql(shared: &SharedToolCtx, database: &str, sql: &str, max_rows: usize) -> Value {
    let tables = match shared.catalog.list_tables_in_database(database).await {
        Ok(t) => t,
        Err(e) => {
            return json!({"error": format!("list_tables_in_database({database}): {e}")});
        }
    };
    if tables.is_empty() {
        return json!({"error": format!("database `{database}` has no tables or does not exist")});
    }
    let runtime = match RuntimeEnv::new(
        RuntimeConfig::new()
            .with_memory_pool(Arc::new(GreedyMemoryPool::new(TOOL_MEMORY_POOL_BYTES))),
    ) {
        Ok(r) => Arc::new(r),
        Err(e) => return json!({"error": format!("runtime_env: {e}")}),
    };
    let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
    kyma_exec::register_vector_udfs(&ctx);
    for t in tables {
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
