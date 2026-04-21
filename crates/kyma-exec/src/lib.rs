//! DataFusion integration — phase A.
//!
//! Exposes [`KymaTable`], a DataFusion `TableProvider` that resolves a
//! kyma table against the catalog + format, fetches matching extents, and
//! materializes their rows for SQL execution via `MemoryExec`.
//!
//! Phase-A simplifications vs. the target architecture:
//!
//!  - No filter pushdown yet — all filters evaluated above the scan.
//!  - Loads all matching extents into memory (no streaming).
//!  - No cache-locality-aware planner — every query reads every matching
//!    extent from object storage.
//!
//! These don't affect correctness; they affect latency + memory footprint.
//! Real pushdown + streaming scans land with M2's custom `KymaScanExec`.

#![forbid(unsafe_code)]

use arrow_array::{new_null_array, RecordBatch};
use arrow_schema::{DataType, Field, SchemaRef as ArrowSchemaRef, TimeUnit};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use datafusion::catalog::Session;
use datafusion::datasource::TableProvider;
use datafusion::datasource::TableType;
use datafusion::error::{DataFusionError, Result as DfResult};
use datafusion::logical_expr::{BinaryExpr, Expr, Like, Operator, TableProviderFilterPushDown};
use datafusion::physical_plan::memory::MemoryExec;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::scalar::ScalarValue;
use kyma_core::catalog::{Catalog, ColumnPrune, PrunePredicate, TableRef, TimeRange};
use kyma_core::segment_format::{
    BlockPredicate, ColumnId, ExtentReader, OpenExtentInput, ScalarValue as BlockScalar,
    SegmentFormat,
};
use std::any::Any;
use std::sync::Arc;
use tracing::instrument;

/// A kyma table exposed to DataFusion.
///
/// Construct once per query (or cache cheaply — it holds only shared refs).
pub struct KymaTable {
    table: TableRef,
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
}

impl std::fmt::Debug for KymaTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KymaTable")
            .field("name", &self.table.name)
            .field("table_id", &self.table.id)
            .finish_non_exhaustive()
    }
}

impl KymaTable {
    pub fn new(table: TableRef, catalog: Arc<dyn Catalog>, format: Arc<dyn SegmentFormat>) -> Self {
        Self {
            table,
            catalog,
            format,
        }
    }
}

#[async_trait]
impl TableProvider for KymaTable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> ArrowSchemaRef {
        self.table.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    /// Declare which filters we can evaluate at the scan level.
    ///
    /// `Inexact` for:
    ///   - time-range predicates on a `Timestamp(Nanosecond)` column
    ///   - equality / IN predicates on indexable typed columns (String /
    ///     Int32 / Int64) — these prune extents via the per-extent
    ///     distinct-value set stored in `column_stats`.
    ///
    /// The DataFusion planner still applies these filters above the scan
    /// (hence `Inexact`), so rows within surviving extents are re-checked.
    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DfResult<Vec<TableProviderFilterPushDown>> {
        let ts_col = self
            .table
            .schema
            .fields()
            .iter()
            .find(|f| matches!(f.data_type(), DataType::Timestamp(TimeUnit::Nanosecond, _)))
            .map(|f| f.name().to_string());
        let indexable_cols: std::collections::HashSet<String> = self
            .table
            .schema
            .fields()
            .iter()
            .filter(|f| {
                matches!(
                    f.data_type(),
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Int32 | DataType::Int64
                )
            })
            .map(|f| f.name().clone())
            .collect();

        let string_cols: std::collections::HashSet<String> = self
            .table
            .schema
            .fields()
            .iter()
            .filter(|f| matches!(f.data_type(), DataType::Utf8 | DataType::LargeUtf8))
            .map(|f| f.name().clone())
            .collect();

        let out = filters
            .iter()
            .map(|f| {
                if let Some(name) = &ts_col {
                    if extract_time_bound(f, name).is_some() {
                        return TableProviderFilterPushDown::Inexact;
                    }
                }
                if extract_equality(f, &indexable_cols).is_some() {
                    return TableProviderFilterPushDown::Inexact;
                }
                if extract_text_search(f, &string_cols).is_some() {
                    return TableProviderFilterPushDown::Inexact;
                }
                TableProviderFilterPushDown::Unsupported
            })
            .collect();
        Ok(out)
    }

    #[instrument(skip(self, _state, filters), fields(table = %self.table.name))]
    async fn scan(
        &self,
        _state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        // Build a PrunePredicate from the pushed-down filters (phase D.1:
        // time-range only; equality + range on typed cols land with per-
        // block min/max in phase D.2).
        let prune = build_prune_predicate(&self.table, filters);
        let manifests = self
            .catalog
            .list_extents(self.table.id, self.table.current_snapshot_id, &prune)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;

        tracing::debug!(
            extents = manifests.len(),
            limit = ?limit,
            projection = ?projection,
            time_range = ?prune.time_range,
            "kyma scan"
        );

        ::metrics::counter!("kyma_scan_extents_listed_total").increment(manifests.len() as u64);

        // 2. For every surviving extent, open it, read all blocks, and
        //    promote each batch from its extent-native schema to the
        //    *current* table schema (null-fill new columns from ADD COLUMN).
        //    Projection is applied on the promoted batch.
        let mut batches: Vec<RecordBatch> = Vec::new();
        for m in manifests {
            let reader = self
                .format
                .open_extent(OpenExtentInput {
                    extent_id: m.id,
                    table_id: m.table_id,
                    schema: self.table.schema.clone(),
                    object_path: m.object_path.clone(),
                    byte_size: m.byte_size,
                })
                .await
                .map_err(|e| DataFusionError::External(Box::new(e)))?;

            let total_blocks = reader.metadata().block_count;
            let block_pred = build_block_predicate(&self.table, &prune);
            let block_ids = reader
                .pruned_blocks(&block_pred)
                .await
                .map_err(|e| DataFusionError::External(Box::new(e)))?;

            let scanned = block_ids.len() as u64;
            let pruned = (total_blocks as u64).saturating_sub(scanned);
            ::metrics::counter!("kyma_scan_blocks_scanned_total").increment(scanned);
            ::metrics::counter!("kyma_scan_blocks_pruned_total").increment(pruned);

            for bid in block_ids {
                // Read full columns of the extent (no extent-level projection),
                // so we can promote to the current table schema first.
                let raw = read_one_block(&*reader, bid, &[]).await?;
                let promoted = promote_batch(&raw, &self.table.schema)?;
                let projected = match projection {
                    Some(cols) => promoted
                        .project(cols)
                        .map_err(|e| DataFusionError::ArrowError(e, None))?,
                    None => promoted,
                };
                batches.push(projected);

                // Early stop when the LIMIT is already covered.
                if let Some(l) = limit {
                    let total: usize = batches.iter().map(|b| b.num_rows()).sum();
                    if total >= l {
                        break;
                    }
                }
            }
        }

        // 3. Build a DataFusion MemoryExec over the decoded (promoted, projected) batches.
        let scan_schema: ArrowSchemaRef = match projection {
            Some(cols) => Arc::new(
                self.table
                    .schema
                    .project(cols)
                    .map_err(|e| DataFusionError::ArrowError(e, None))?,
            ),
            None => self.table.schema.clone(),
        };

        let partition = vec![batches];
        let plan = MemoryExec::try_new(&partition, scan_schema, None)?;
        Ok(Arc::new(plan))
    }
}

async fn read_one_block(
    reader: &dyn ExtentReader,
    block: kyma_core::segment_format::BlockId,
    projection: &[ColumnId],
) -> DfResult<RecordBatch> {
    reader
        .read_block(block, projection)
        .await
        .map_err(|e| DataFusionError::External(Box::new(e)))
}

/// Promote a batch from its extent-native schema to the current table schema.
///
/// Rules (phase-C):
///   - For each field in `target_schema`:
///     - If present in `batch` (matched by name + data-type equality) → keep
///       the original array.
///     - If present by name but with a widened compatible type → cast.
///       (Not yet implemented — placeholder returns arrow cast error.)
///     - If missing → null-fill with `new_null_array`.
///   - Fields present in the batch but not in the target schema are dropped.
///
/// This is the mechanism that makes ALTER TABLE ADD COLUMN work without
/// rewriting historical extents.
fn promote_batch(batch: &RecordBatch, target: &ArrowSchemaRef) -> DfResult<RecordBatch> {
    let n = batch.num_rows();
    let batch_schema = batch.schema();
    let mut arrays: Vec<arrow_array::ArrayRef> = Vec::with_capacity(target.fields().len());
    for target_field in target.fields() {
        match batch_schema.index_of(target_field.name()) {
            Ok(idx) => {
                let a = batch.column(idx);
                // Exact type match for now; widen + cast support lands with the
                // real format in a later slice.
                if a.data_type() != target_field.data_type() {
                    return Err(DataFusionError::Plan(format!(
                        "schema promotion: column {:?} type mismatch (extent has {:?}, \
                         table expects {:?}); cast not yet implemented",
                        target_field.name(),
                        a.data_type(),
                        target_field.data_type()
                    )));
                }
                arrays.push(a.clone());
            }
            Err(_) => {
                // Column not in the extent — ADD COLUMN null-fill.
                arrays.push(new_null_array(target_field.data_type(), n));
            }
        }
    }
    RecordBatch::try_new(target.clone(), arrays).map_err(|e| DataFusionError::ArrowError(e, None))
}

#[allow(dead_code)]
fn _unused_field_ref(_: Field) {}

// --------------------------------------------------------------------------
// Filter pushdown helpers (Phase D.1)
// --------------------------------------------------------------------------

/// Half-open bound extracted from one filter expression against the
/// timestamp column.
#[derive(Debug, Clone, Copy)]
enum TimeBound {
    GtOrGe(DateTime<Utc>, /* inclusive = */ bool),
    LtOrLe(DateTime<Utc>, /* inclusive = */ bool),
    Between(DateTime<Utc>, DateTime<Utc>),
}

/// If the filter is a time-range predicate over `ts_col`, return it. Otherwise None.
fn extract_time_bound(expr: &Expr, ts_col: &str) -> Option<TimeBound> {
    match expr {
        Expr::BinaryExpr(BinaryExpr { left, op, right }) => {
            // Support both `col OP literal` and `literal OP col`. Normalise to col-on-left.
            let (col, op, lit) = match (&**left, &**right) {
                (Expr::Column(c), _) if c.name == ts_col => (c, *op, &**right),
                (_, Expr::Column(c)) if c.name == ts_col => (c, flip_op(*op), &**left),
                _ => return None,
            };
            let _ = col;
            let ts = ts_from_expr(lit)?;
            match op {
                Operator::Gt => Some(TimeBound::GtOrGe(ts, false)),
                Operator::GtEq => Some(TimeBound::GtOrGe(ts, true)),
                Operator::Lt => Some(TimeBound::LtOrLe(ts, false)),
                Operator::LtEq => Some(TimeBound::LtOrLe(ts, true)),
                _ => None,
            }
        }
        Expr::Between(b) if !b.negated => {
            let col = match &*b.expr {
                Expr::Column(c) if c.name == ts_col => c,
                _ => return None,
            };
            let _ = col;
            let low = ts_from_expr(&b.low)?;
            let high = ts_from_expr(&b.high)?;
            Some(TimeBound::Between(low, high))
        }
        _ => None,
    }
}

fn flip_op(op: Operator) -> Operator {
    match op {
        Operator::Lt => Operator::Gt,
        Operator::LtEq => Operator::GtEq,
        Operator::Gt => Operator::Lt,
        Operator::GtEq => Operator::LtEq,
        other => other,
    }
}

fn ts_from_expr(e: &Expr) -> Option<DateTime<Utc>> {
    let lit = match e {
        Expr::Literal(s) => s,
        Expr::Cast(c) => match &*c.expr {
            Expr::Literal(s) => s,
            _ => return None,
        },
        _ => return None,
    };
    match lit {
        ScalarValue::TimestampNanosecond(Some(n), _) => {
            let secs = n.div_euclid(1_000_000_000);
            let nanos = n.rem_euclid(1_000_000_000) as u32;
            Utc.timestamp_opt(secs, nanos).single()
        }
        ScalarValue::TimestampMicrosecond(Some(n), _) => Utc
            .timestamp_opt(n / 1_000_000, ((n % 1_000_000) * 1_000) as u32)
            .single(),
        ScalarValue::TimestampMillisecond(Some(n), _) => Utc
            .timestamp_opt(n / 1000, ((n % 1000) * 1_000_000) as u32)
            .single(),
        ScalarValue::TimestampSecond(Some(n), _) => Utc.timestamp_opt(*n, 0).single(),
        ScalarValue::Utf8(Some(s)) | ScalarValue::LargeUtf8(Some(s)) => {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }
        _ => None,
    }
}

/// Combine the time bounds from a list of filters into a single
/// [`PrunePredicate`]. Conservative: only AND conjunction is honored.
/// If *any* filter can't be converted to a bound it's simply skipped —
/// the returned predicate is always a *superset* of the true filter.
fn build_prune_predicate(table: &TableRef, filters: &[Expr]) -> PrunePredicate {
    let ts_col = table
        .schema
        .fields()
        .iter()
        .find(|f| matches!(f.data_type(), DataType::Timestamp(TimeUnit::Nanosecond, _)))
        .map(|f| f.name().to_string());
    let Some(ts_col) = ts_col else {
        return PrunePredicate::default();
    };

    let mut lo: Option<DateTime<Utc>> = None;
    let mut hi: Option<DateTime<Utc>> = None;
    for f in filters {
        match extract_time_bound(f, &ts_col) {
            Some(TimeBound::GtOrGe(t, _inclusive)) => {
                lo = Some(lo.map(|cur| cur.max(t)).unwrap_or(t));
            }
            Some(TimeBound::LtOrLe(t, inclusive)) => {
                let adj = if inclusive {
                    t + chrono::Duration::nanoseconds(1)
                } else {
                    t
                };
                hi = Some(hi.map(|cur| cur.min(adj)).unwrap_or(adj));
            }
            Some(TimeBound::Between(a, b)) => {
                lo = Some(lo.map(|cur| cur.max(a)).unwrap_or(a));
                let b_adj = b + chrono::Duration::nanoseconds(1);
                hi = Some(hi.map(|cur| cur.min(b_adj)).unwrap_or(b_adj));
            }
            None => {}
        }
    }

    let time_range = match (lo, hi) {
        (Some(s), Some(e)) if s < e => Some(TimeRange {
            start_inclusive: s,
            end_exclusive: e,
        }),
        // One-sided bounds: use a very-far sentinel for the missing side so
        // catalog's GIST range query still works without lots of branching.
        (Some(s), None) => Some(TimeRange {
            start_inclusive: s,
            end_exclusive: Utc
                .with_ymd_and_hms(2999, 12, 31, 23, 59, 59)
                .single()
                .unwrap_or_else(Utc::now),
        }),
        (None, Some(e)) => Some(TimeRange {
            start_inclusive: Utc
                .with_ymd_and_hms(1970, 1, 1, 0, 0, 0)
                .single()
                .unwrap_or_else(Utc::now),
            end_exclusive: e,
        }),
        _ => None,
    };

    // Equality-index pushdown for string/int columns.
    let indexable_cols: std::collections::HashSet<String> = table
        .schema
        .fields()
        .iter()
        .filter(|f| {
            matches!(
                f.data_type(),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Int32 | DataType::Int64
            )
        })
        .map(|f| f.name().clone())
        .collect();
    let string_cols: std::collections::HashSet<String> = table
        .schema
        .fields()
        .iter()
        .filter(|f| matches!(f.data_type(), DataType::Utf8 | DataType::LargeUtf8))
        .map(|f| f.name().clone())
        .collect();

    let mut column_predicates: std::collections::HashMap<String, ColumnPrune> =
        std::collections::HashMap::new();
    for f in filters {
        if let Some((col, prune)) = extract_equality(f, &indexable_cols) {
            column_predicates.entry(col).or_insert(prune);
            continue;
        }
        if let Some((col, prune)) = extract_text_search(f, &string_cols) {
            // Text-search pruning — only applies if the equality index
            // didn't already claim this column (which would be more
            // specific).
            column_predicates.entry(col).or_insert(prune);
        }
    }

    PrunePredicate {
        time_range,
        column_predicates,
        ..Default::default()
    }
}

/// Lower the catalog-level [`PrunePredicate`] into a block-level
/// [`BlockPredicate`]. Reuses the same conjunction: any bound the catalog
/// already evaluated, block-level stats can filter further.
///
/// Mappings:
/// - `time_range`                      → `BlockPredicate::TimeRange`
/// - `ColumnPrune::Equals(v)`          → `BlockPredicate::Equals`
/// - `ColumnPrune::InSet([...])`       → `BlockPredicate::InSet`
/// - `ColumnPrune::Between / ContainsTokens` → skipped (conservatively `All`)
fn build_block_predicate(table: &TableRef, prune: &PrunePredicate) -> BlockPredicate {
    let mut parts: Vec<BlockPredicate> = Vec::new();

    let name_to_id = |name: &str| -> Option<ColumnId> {
        table
            .schema
            .fields()
            .iter()
            .position(|f| f.name() == name)
            .map(|i| ColumnId(i as u32))
    };

    if let Some(tr) = &prune.time_range {
        let ts_col =
            table.schema.fields().iter().enumerate().find(|(_, f)| {
                matches!(f.data_type(), DataType::Timestamp(TimeUnit::Nanosecond, _))
            });
        if let Some((i, _)) = ts_col {
            let start = tr.start_inclusive.timestamp_nanos_opt().unwrap_or(i64::MIN);
            let end = tr.end_exclusive.timestamp_nanos_opt().unwrap_or(i64::MAX);
            parts.push(BlockPredicate::TimeRange {
                col: ColumnId(i as u32),
                start_inclusive: start,
                end_exclusive: end,
            });
        }
    }

    for (col_name, pred) in &prune.column_predicates {
        let Some(col_id) = name_to_id(col_name) else {
            continue;
        };
        match pred {
            ColumnPrune::Equals(v) => {
                if let Some(sv) = json_to_block_scalar(v) {
                    parts.push(BlockPredicate::Equals {
                        col: col_id,
                        value: sv,
                    });
                }
            }
            ColumnPrune::InSet(vals) => {
                let values: Vec<BlockScalar> =
                    vals.iter().filter_map(json_to_block_scalar).collect();
                if !values.is_empty() {
                    parts.push(BlockPredicate::InSet {
                        col: col_id,
                        values,
                    });
                }
            }
            ColumnPrune::Between { .. } | ColumnPrune::ContainsTokens(_) => {}
        }
    }

    if parts.is_empty() {
        BlockPredicate::All
    } else if parts.len() == 1 {
        parts.into_iter().next().unwrap()
    } else {
        BlockPredicate::And(parts)
    }
}

fn json_to_block_scalar(v: &serde_json::Value) -> Option<BlockScalar> {
    match v {
        serde_json::Value::String(s) => Some(BlockScalar::String(s.clone())),
        serde_json::Value::Number(n) => n.as_i64().map(BlockScalar::Int64),
        serde_json::Value::Bool(b) => Some(BlockScalar::Bool(*b)),
        serde_json::Value::Null => Some(BlockScalar::Null),
        _ => None,
    }
}

/// Extract a text-search filter and convert to `ColumnPrune::ContainsTokens`.
///
/// Recognized forms (all on a `Utf8` column):
///   - `Expr::Like { expr: Column, pattern: Literal("%needle%"), negated:
///     false, case_insensitive: false | true }` — standard SQL `LIKE`.
///   - `Expr::Like { … pattern: Literal("needle%") }` and `Literal("%needle")`
///     — prefix / suffix match. We still tokenize what's between the
///     wildcards; over-inclusive if the needle is a prefix inside another
///     token, which DataFusion re-checks above the scan.
///
/// Tokenization matches what the writer does: lowercase, alphanumeric-split,
/// tokens ≥ 2 chars.
fn extract_text_search(
    expr: &Expr,
    string_cols: &std::collections::HashSet<String>,
) -> Option<(String, ColumnPrune)> {
    let Expr::Like(Like {
        expr: inner_expr,
        pattern,
        negated,
        escape_char,
        case_insensitive,
    }) = expr
    else {
        return None;
    };
    if *negated {
        return None; // NOT LIKE — would need set difference, skip.
    }
    let _ = (escape_char, case_insensitive); // our tokens are lowercase anyway.
    let col_name = match &**inner_expr {
        Expr::Column(c) if string_cols.contains(&c.name) => c.name.clone(),
        _ => return None,
    };
    let raw = match &**pattern {
        Expr::Literal(ScalarValue::Utf8(Some(s)))
        | Expr::Literal(ScalarValue::LargeUtf8(Some(s))) => s.clone(),
        _ => return None,
    };
    // Strip leading/trailing % wildcards; tokens inside are what we index.
    // Any interior `%` or `_` means DataFusion still filters row-by-row;
    // we're just pruning *extents* here.
    let trimmed = raw.trim_matches('%');
    let mut tokens: std::collections::HashSet<String> = Default::default();
    kyma_format_tlm_tokenize_like(trimmed, &mut tokens);
    if tokens.is_empty() {
        return None;
    }
    // Sort for stable catalog SQL.
    let mut tokens_vec: Vec<String> = tokens.into_iter().collect();
    tokens_vec.sort();
    Some((col_name, ColumnPrune::ContainsTokens(tokens_vec)))
}

/// Minimal mirror of the writer's tokenizer. Kept local so kyma-exec
/// doesn't need a direct runtime dep on kyma-format-tlm's internals.
fn kyma_format_tlm_tokenize_like(s: &str, out: &mut std::collections::HashSet<String>) {
    let mut cur = String::with_capacity(16);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if !cur.is_empty() {
            if cur.len() >= 2 {
                out.insert(cur.clone());
            }
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.insert(cur);
    }
}

/// Extract a pushable equality / IN filter. Returns `(column_name, prune)`
/// if the filter is of the form `col == literal`, `col IN (literal, …)`,
/// or `col == a OR col == b OR …` (the form DataFusion's simplifier
/// produces for small `IN` lists) — for a column we can index.
fn extract_equality(
    expr: &Expr,
    indexable_cols: &std::collections::HashSet<String>,
) -> Option<(String, ColumnPrune)> {
    match expr {
        Expr::BinaryExpr(BinaryExpr { left, op, right }) if *op == Operator::Eq => {
            let (col_name, lit) = match (&**left, &**right) {
                (Expr::Column(c), Expr::Literal(l)) if indexable_cols.contains(&c.name) => {
                    (c.name.clone(), l)
                }
                (Expr::Literal(l), Expr::Column(c)) if indexable_cols.contains(&c.name) => {
                    (c.name.clone(), l)
                }
                _ => return None,
            };
            scalar_to_json(lit).map(|j| (col_name, ColumnPrune::Equals(j)))
        }
        Expr::BinaryExpr(BinaryExpr { left, op, right }) if *op == Operator::Or => {
            // Collect all equality values if every leaf is `col == literal`
            // on the same column. DataFusion's logical-simplifier emits this
            // form for small SQL `IN (a, b, c)` lists.
            let mut col_name: Option<String> = None;
            let mut values: Vec<serde_json::Value> = Vec::new();
            fn walk(
                e: &Expr,
                indexable: &std::collections::HashSet<String>,
                col_name: &mut Option<String>,
                values: &mut Vec<serde_json::Value>,
            ) -> bool {
                match e {
                    Expr::BinaryExpr(BinaryExpr { left, op, right }) if *op == Operator::Or => {
                        walk(left, indexable, col_name, values)
                            && walk(right, indexable, col_name, values)
                    }
                    Expr::BinaryExpr(BinaryExpr { left, op, right }) if *op == Operator::Eq => {
                        let (n, lit) = match (&**left, &**right) {
                            (Expr::Column(c), Expr::Literal(l)) if indexable.contains(&c.name) => {
                                (c.name.clone(), l)
                            }
                            (Expr::Literal(l), Expr::Column(c)) if indexable.contains(&c.name) => {
                                (c.name.clone(), l)
                            }
                            _ => return false,
                        };
                        if let Some(existing) = col_name {
                            if *existing != n {
                                return false; // different columns in OR — can't push down
                            }
                        } else {
                            *col_name = Some(n);
                        }
                        if let Some(j) = scalar_to_json(lit) {
                            values.push(j);
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            if walk(left, indexable_cols, &mut col_name, &mut values)
                && walk(right, indexable_cols, &mut col_name, &mut values)
            {
                return col_name.map(|n| (n, ColumnPrune::InSet(values)));
            }
            None
        }
        Expr::InList(in_list) if !in_list.negated => {
            let col_name = match &*in_list.expr {
                Expr::Column(c) if indexable_cols.contains(&c.name) => c.name.clone(),
                _ => return None,
            };
            let mut vals = Vec::with_capacity(in_list.list.len());
            for e in &in_list.list {
                if let Expr::Literal(l) = e {
                    if let Some(v) = scalar_to_json(l) {
                        vals.push(v);
                        continue;
                    }
                }
                return None;
            }
            Some((col_name, ColumnPrune::InSet(vals)))
        }
        _ => None,
    }
}

fn scalar_to_json(s: &ScalarValue) -> Option<serde_json::Value> {
    match s {
        ScalarValue::Utf8(Some(v)) | ScalarValue::LargeUtf8(Some(v)) => {
            Some(serde_json::Value::String(v.clone()))
        }
        ScalarValue::Int32(Some(v)) => Some(serde_json::json!(i64::from(*v))),
        ScalarValue::Int64(Some(v)) => Some(serde_json::json!(*v)),
        _ => None,
    }
}
