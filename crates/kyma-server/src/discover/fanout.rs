//! Parallel fanout executor for Discover.
//!
//! Runs the user's compiled per-source KQL across every `ResolvedSource` in
//! parallel under a shared wall-clock + memory budget, emitting `Frame`s into
//! an mpsc channel as sources complete.
//!
//! Frame ordering invariants:
//!   - `Plan` is emitted first.
//!   - Per-source frames may interleave across sources.
//!   - `Done` is emitted last, after every child task has finished.
//!
//! Per-source errors do NOT abort siblings: a child emits its `Error` frame
//! and exits cleanly. The wall-clock deadline is computed once before
//! fanning out; each child computes its own `remaining` budget against it.

use std::sync::Arc;
use std::time::Instant;

use arrow::json::ArrayWriter;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::{RuntimeConfig, RuntimeEnv};
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_core::catalog::Catalog;
use kyma_core::query_frontend::QueryBudget;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::types::NodeId;
use kyma_exec::KymaTable;
use tokio::sync::mpsc;

use super::compile::{compile_for_source, CompiledSource, TimeRange};
use super::frames::{Frame, PlanSource};
use super::grammar::Clause;
use super::scope::ResolvedSource;

/// Inputs to the fanout executor.
pub struct FanoutInput {
    pub sources: Vec<ResolvedSource>,
    pub clauses: Vec<Clause>,
    pub time_range: Option<TimeRange>,
    pub per_source_limit: usize,
    pub budget: QueryBudget,
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn SegmentFormat>,
    pub node_id: Option<NodeId>,
}

/// Spawn the fanout executor. Returns immediately; the caller drains `tx` for
/// frames. The orchestrator task and all child tasks run on the ambient tokio
/// runtime.
pub fn run(input: FanoutInput, tx: mpsc::Sender<Frame>) {
    tokio::spawn(async move { run_inner(input, tx).await });
}

async fn run_inner(input: FanoutInput, tx: mpsc::Sender<Frame>) {
    let start = Instant::now();
    let deadline = start + input.budget.max_wall_clock;

    // 1. Build the plan frame. has_timestamp is a per-source schema property,
    //    so we compute it via compile_for_source with empty clauses.
    let plan_sources: Vec<PlanSource> = input
        .sources
        .iter()
        .map(|src| {
            let compiled = compile_for_source(&src.table, &[], None, input.per_source_limit);
            PlanSource {
                source: format!("{}.{}", src.db, src.table.name),
                has_timestamp: compiled.has_timestamp,
            }
        })
        .collect();

    // Receiver may have dropped — that's OK, fall through cleanly.
    let _ = tx.send(Frame::Plan { sources: plan_sources }).await;

    // 2. Pre-compile every source.
    let compiled: Vec<(ResolvedSource, CompiledSource)> = input
        .sources
        .into_iter()
        .map(|src| {
            let c = compile_for_source(
                &src.table,
                &input.clauses,
                input.time_range.as_ref(),
                input.per_source_limit,
            );
            (src, c)
        })
        .collect();

    // 3. Spawn a child task per source.
    let mut handles = Vec::with_capacity(compiled.len());
    for (src, compiled) in compiled {
        let tx = tx.clone();
        let catalog = input.catalog.clone();
        let format = input.format.clone();
        let node_id = input.node_id;
        let budget = input.budget.clone();
        let per_source_limit = input.per_source_limit;
        let h = tokio::spawn(async move {
            run_source(
                src,
                compiled,
                catalog,
                format,
                node_id,
                budget,
                deadline,
                per_source_limit,
                tx,
            )
            .await;
        });
        handles.push(h);
    }

    // 4. Wait for all children. We ignore JoinErrors (panics) — they don't
    //    fit the per-source error model cleanly; the orchestrator just moves
    //    on so the receiver still gets the final Done frame.
    for h in handles {
        let _ = h.await;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let _ = tx.send(Frame::Done { elapsed_ms }).await;
}

#[allow(clippy::too_many_arguments)]
async fn run_source(
    src: ResolvedSource,
    compiled: CompiledSource,
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    node_id: Option<NodeId>,
    budget: QueryBudget,
    deadline: Instant,
    per_source_limit: usize,
    tx: mpsc::Sender<Frame>,
) {
    let source_key = format!("{}.{}", src.db, src.table.name);

    // a. Running progress frame.
    let _ = tx
        .send(Frame::SourceProgress {
            source: source_key.clone(),
            state: super::frames::ProgressState::Running,
        })
        .await;

    // b. Compile KQL → SQL.
    let sql = match kyma_kql::kql_to_sql(&compiled.kql) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "kql_compile_error".into(),
                    message: e.0,
                })
                .await;
            return;
        }
    };

    // c. Fresh SessionContext bounded by the per-request memory budget.
    let runtime = match RuntimeEnv::new(RuntimeConfig::new().with_memory_pool(Arc::new(
        GreedyMemoryPool::new(budget.max_memory_bytes as usize),
    ))) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "internal".into(),
                    message: format!("runtime env: {e}"),
                })
                .await;
            return;
        }
    };
    let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);

    // d. Vector UDFs available to every Discover query.
    kyma_exec::register_vector_udfs(&ctx);

    // e. Build + register the KymaTable.
    let table_name = src.table.name.clone();
    let kt: Arc<KymaTable> = match node_id {
        Some(nid) => Arc::new(KymaTable::with_node_id(
            src.table.clone(),
            catalog,
            format,
            nid,
            src.db.clone(),
        )),
        None => Arc::new(KymaTable::new(src.table.clone(), catalog, format)),
    };
    if let Err(e) = ctx.register_table(&table_name, kt) {
        let _ = tx
            .send(Frame::Error {
                source: Some(source_key),
                code: "table_register_error".into(),
                message: e.to_string(),
            })
            .await;
        return;
    }

    // f. Plan + execute under the shared wall-clock deadline.
    let df = match ctx.sql(&sql).await {
        Ok(df) => df,
        Err(e) => {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "query_execution_error".into(),
                    message: format!("SQL plan: {e}"),
                })
                .await;
            return;
        }
    };

    let remaining = deadline.saturating_duration_since(Instant::now());
    let batches = match tokio::time::timeout(remaining, df.collect()).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            // g. Classify the error.
            let msg = e.to_string();
            let (code, message) = if msg.contains("ResourcesExhausted")
                || msg.contains("Resources exhausted")
            {
                ("memory_exceeded", msg)
            } else {
                ("query_execution_error", format!("query execution: {msg}"))
            };
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: code.into(),
                    message,
                })
                .await;
            return;
        }
        Err(_elapsed) => {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "wall_clock_exceeded".into(),
                    message: format!(
                        "source exceeded wall-clock budget of {}ms",
                        budget.max_wall_clock.as_millis()
                    ),
                })
                .await;
            return;
        }
    };

    // h. Serialize rows + emit final frames.
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();

    let mut body_bytes: Vec<u8> = Vec::with_capacity(total_rows * 128);
    for batch in &batches {
        let mut writer = ArrayWriter::new(&mut body_bytes);
        if let Err(e) = writer.write(batch) {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "serialization_error".into(),
                    message: format!("result serialization: {e}"),
                })
                .await;
            return;
        }
        if let Err(e) = writer.finish() {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "serialization_error".into(),
                    message: format!("result serialization finish: {e}"),
                })
                .await;
            return;
        }
    }

    let rows = match collate_to_values(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            let _ = tx
                .send(Frame::Error {
                    source: Some(source_key),
                    code: "serialization_error".into(),
                    message: format!("row collation: {e}"),
                })
                .await;
            return;
        }
    };

    if !rows.is_empty() {
        let _ = tx
            .send(Frame::Rows {
                source: source_key.clone(),
                rows,
            })
            .await;
    }

    let capped = total_rows >= per_source_limit;
    let _ = tx
        .send(Frame::SourceDone {
            source: source_key,
            total: total_rows,
            capped,
            dropped_clauses: compiled.dropped_clauses,
        })
        .await;
}

/// Mirror of `collate_ndjson` from `lib.rs`, but emits row `Value`s rather
/// than a newline-joined string. The input is the concatenation of zero or
/// more JSON arrays as emitted by `arrow_json::ArrayWriter` (one array per
/// `RecordBatch`).
fn collate_to_values(concatenated_arrays: &[u8]) -> Result<Vec<serde_json::Value>, String> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let stream =
        serde_json::Deserializer::from_slice(concatenated_arrays).into_iter::<serde_json::Value>();
    for arr in stream {
        let arr = arr.map_err(|e| format!("json parse: {e}"))?;
        match arr {
            serde_json::Value::Array(rows) => out.extend(rows),
            other => out.push(other),
        }
    }
    Ok(out)
}
