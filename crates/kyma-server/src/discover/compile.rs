//! Compiles a `Vec<Clause>` against a concrete source schema to a KQL string.
//!
//! Tolerance rules (see spec section 5):
//! - Clauses on fields the source does not expose are silently dropped.
//! - Numeric comparisons against non-numeric columns are silently dropped.
//! - Substrings expand to a disjunction over all string columns of the source.
//! - A source with zero remaining clauses still gets executed (returns recent rows).
//!
//! Returns the KQL string and the list of clauses that were dropped (for the
//! `dropped_clauses` field on `source_done`).

use arrow_schema::DataType;
use kyma_core::catalog::TableRef;
use serde::Serialize;

use super::grammar::{Clause, CmpOp};

#[derive(Debug, Clone, Serialize)]
pub struct CompiledSource {
    pub kql: String,
    pub dropped_clauses: Vec<DroppedClause>,
    pub has_timestamp: bool,
    pub timestamp_column: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DroppedClause {
    pub reason: DropReason,
    pub clause: Clause,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    UnknownField,
    TypeMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub from_ms: i64,
    pub to_ms: i64,
}

pub fn compile_for_source(
    source: &TableRef,
    clauses: &[Clause],
    time_range: Option<&TimeRange>,
    per_source_limit: usize,
) -> CompiledSource {
    let ts_col = find_timestamp_column(source);
    let has_timestamp = ts_col.is_some();
    let ts_col_name = ts_col.as_ref().map(|t| t.name.clone());

    let mut where_parts: Vec<String> = Vec::new();
    let mut dropped: Vec<DroppedClause> = Vec::new();

    for c in clauses {
        match compile_clause(source, c) {
            Ok(Some(s)) => where_parts.push(s),
            Ok(None) => {}
            Err(reason) => dropped.push(DroppedClause {
                reason,
                clause: c.clone(),
            }),
        }
    }

    if let Some(tr) = time_range {
        if let Some(ts) = &ts_col {
            let from = ms_to_iso(tr.from_ms);
            let to = ms_to_iso(tr.to_ms);
            match ts.kind {
                TsKind::Timestamp => {
                    // Emit ISO-8601 string literals, NOT raw epoch-millis.
                    // `datetime(x)` compiles to `CAST(x AS TIMESTAMP)`, and a bare
                    // integer is read as epoch *seconds* (×1e9 → nanos), so
                    // millisecond magnitudes overflow i64 and break every
                    // timestamped source in the fanout.
                    where_parts.push(format!(
                        "{col} >= datetime(\"{from}\") and {col} < datetime(\"{to}\")",
                        col = ts.name,
                    ));
                }
                TsKind::IsoString => {
                    // The time column is an ISO-8601 *string* (e.g. a firehose
                    // that writes its own `ts` while the reserved `at` column is
                    // null). ISO-8601 UTC strings sort lexicographically in
                    // chronological order, so a plain string range compare is
                    // both valid SQL and chronologically correct for the
                    // canonical `…Z` format.
                    where_parts.push(format!(
                        "{col} >= \"{from}\" and {col} < \"{to}\"",
                        col = ts.name,
                    ));
                }
            }
        }
    }

    let mut kql = source.name.clone();
    for p in &where_parts {
        kql.push_str(" | where ");
        kql.push_str(p);
    }
    // Drop vector/embedding columns — hundreds of floats per row, useless in a
    // search preview and a big NDJSON bloat (e.g. memory_nodes.embedding).
    let drop_cols = vector_columns(source);
    if !drop_cols.is_empty() {
        kql.push_str(" | project-away ");
        kql.push_str(&drop_cols.join(", "));
    }
    kql.push_str(&format!(" | take {per_source_limit}"));

    CompiledSource {
        kql,
        dropped_clauses: dropped,
        has_timestamp,
        timestamp_column: ts_col_name,
    }
}

/// Render epoch-millis as an RFC3339 UTC string for a KQL `datetime("…")`
/// literal. Passing the bare integer triggers the seconds×1e9 overflow above.
fn ms_to_iso(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string())
}

/// Names of vector/embedding columns (fixed-size or variable lists of floats),
/// which Discover excludes from result rows.
fn vector_columns(source: &TableRef) -> Vec<String> {
    source
        .schema
        .fields()
        .iter()
        .filter(|f| is_vector_type(f.data_type()))
        .map(|f| f.name().clone())
        .collect()
}

fn is_vector_type(dt: &DataType) -> bool {
    match dt {
        DataType::FixedSizeList(field, _)
        | DataType::List(field)
        | DataType::LargeList(field) => matches!(
            field.data_type(),
            DataType::Float16 | DataType::Float32 | DataType::Float64
        ),
        _ => false,
    }
}

fn compile_clause(source: &TableRef, c: &Clause) -> Result<Option<String>, DropReason> {
    match c {
        Clause::Substring { value } => {
            let parts: Vec<String> = source
                .schema
                .fields()
                .iter()
                .filter(|f| is_string_type(f.data_type()))
                .map(|f| format!("{} contains {}", f.name(), escape_str(value)))
                .collect();
            if parts.is_empty() {
                return Err(DropReason::TypeMismatch);
            }
            if parts.len() == 1 {
                Ok(Some(parts.into_iter().next().unwrap()))
            } else {
                Ok(Some(format!("({})", parts.join(" or "))))
            }
        }
        Clause::Eq { field, value } => {
            require_field(source, field)?;
            Ok(Some(format!("{field} == {}", escape_str(value))))
        }
        Clause::Neq { field, value } => {
            require_field(source, field)?;
            Ok(Some(format!("{field} != {}", escape_str(value))))
        }
        Clause::Exists { field } => {
            require_field(source, field)?;
            Ok(Some(format!("isnotnull({field})")))
        }
        Clause::Cmp { field, op, value } => {
            let f = source
                .schema
                .fields()
                .iter()
                .find(|f| f.name() == field)
                .ok_or(DropReason::UnknownField)?;
            if !is_numeric_or_timestamp(f.data_type()) {
                return Err(DropReason::TypeMismatch);
            }
            // Sanity-check the literal parses as a number.
            if value.parse::<f64>().is_err() {
                return Err(DropReason::TypeMismatch);
            }
            let opstr = match op {
                CmpOp::Gt => ">",
                CmpOp::Ge => ">=",
                CmpOp::Lt => "<",
                CmpOp::Le => "<=",
            };
            Ok(Some(format!("{field} {opstr} {value}")))
        }
    }
}

fn require_field(source: &TableRef, name: &str) -> Result<(), DropReason> {
    if source.schema.fields().iter().any(|f| f.name() == name) {
        Ok(())
    } else {
        Err(DropReason::UnknownField)
    }
}

/// How the detected time column must be compared in the time-range predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsKind {
    /// Arrow `Timestamp` column — compare via `datetime("…")`.
    Timestamp,
    /// ISO-8601 UTC string column — compare via plain string range.
    IsoString,
}

/// The time column Discover should filter/bucket on, plus how to compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsColumn {
    pub name: String,
    pub kind: TsKind,
}

/// The reserved auto-time column created by the default ingest schema
/// (`kyma_ingest_core::ensure::default_table_schema`). Ingest only fills it
/// from the aliases `timestamp` / `time_unix_nano` / `observed_time_unix_nano`,
/// so a client that writes its event time under a different field name (e.g. a
/// firehose writing `ts`) leaves `at` NULL for every row — which makes a
/// type-only "first Timestamp column" detector pick a column that filters out
/// all data. We therefore treat `at` as a *last-resort* timestamp column.
const RESERVED_AT_COLUMN: &str = "at";

/// Column names that strongly denote an event-time value when stored as an
/// ISO-8601 string. Kept tight on purpose: loose names like `time`/`date` are
/// excluded because they frequently name non-timestamp fields.
const ISO_STRING_TIME_NAMES: &[&str] =
    &["ts", "timestamp", "event_time", "observed_time", "_time"];

/// Pick the best time column for filtering/bucketing.
///
/// Preference order (schema-only, no row sampling):
///   1. A genuinely-declared `Timestamp` column other than the reserved `at`.
///   2. An ISO-8601 *string* column with a strong time name — this is what
///      rescues firehose-shaped tables whose only `Timestamp` column is the
///      reserved-and-empty `at`.
///   3. The reserved `at` column, if it is `Timestamp`-typed (healthy tables
///      that ingest actually populated `at`).
///   4. Any remaining `Timestamp` column.
///
/// Returns `None` only when the table has no usable time column at all, in
/// which case the caller omits the time filter (rather than emitting a filter
/// on a column that does not exist).
fn find_timestamp_column(source: &TableRef) -> Option<TsColumn> {
    let fields = source.schema.fields();

    // 1. A declared Timestamp column that is not the reserved `at`.
    if let Some(f) = fields.iter().find(|f| {
        f.name() != RESERVED_AT_COLUMN && matches!(f.data_type(), DataType::Timestamp(_, _))
    }) {
        return Some(TsColumn {
            name: f.name().clone(),
            kind: TsKind::Timestamp,
        });
    }

    // 2. A strongly-named ISO-8601 string column.
    if let Some(f) = fields.iter().find(|f| {
        is_string_type(f.data_type())
            && ISO_STRING_TIME_NAMES
                .iter()
                .any(|n| f.name().eq_ignore_ascii_case(n))
    }) {
        return Some(TsColumn {
            name: f.name().clone(),
            kind: TsKind::IsoString,
        });
    }

    // 3 & 4. Any Timestamp column (covers a populated reserved `at`).
    fields
        .iter()
        .find(|f| matches!(f.data_type(), DataType::Timestamp(_, _)))
        .map(|f| TsColumn {
            name: f.name().clone(),
            kind: TsKind::Timestamp,
        })
}

fn is_string_type(ty: &DataType) -> bool {
    matches!(
        ty,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

fn is_numeric_or_timestamp(ty: &DataType) -> bool {
    matches!(
        ty,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
            | DataType::Timestamp(_, _)
    )
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
    use kyma_core::catalog::{TableConfig, TableRef};
    use kyma_core::types::{DatabaseId, SchemaSnapshotId, SnapshotId, TableId};
    use std::sync::Arc;

    fn table(name: &str, fields: &[(&str, DataType)]) -> TableRef {
        let arrow_fields: Vec<Field> = fields
            .iter()
            .map(|(n, ty)| Field::new(*n, ty.clone(), true))
            .collect();
        TableRef {
            id: TableId::new(),
            database_id: DatabaseId::new(),
            name: name.to_string(),
            current_snapshot_id: SnapshotId::new(),
            schema_snapshot_id: SchemaSnapshotId::new(),
            schema: Arc::new(ArrowSchema::new(arrow_fields)),
            config: TableConfig::default(),
        }
    }

    fn ts() -> DataType {
        DataType::Timestamp(TimeUnit::Microsecond, None)
    }

    #[test]
    fn empty_clauses_compile_to_bare_take() {
        let t = table(
            "otel_logs",
            &[("timestamp", ts()), ("message", DataType::Utf8)],
        );
        let c = compile_for_source(&t, &[], None, 500);
        assert_eq!(c.kql, "otel_logs | take 500");
        assert!(c.dropped_clauses.is_empty());
        assert!(c.has_timestamp);
        assert_eq!(c.timestamp_column, Some("timestamp".to_string()));
    }

    #[test]
    fn eq_on_known_field_compiles() {
        let t = table(
            "otel_logs",
            &[
                ("service_name", DataType::Utf8),
                ("message", DataType::Utf8),
            ],
        );
        let c = compile_for_source(
            &t,
            &[Clause::Eq {
                field: "service_name".into(),
                value: "payments".into(),
            }],
            None,
            100,
        );
        assert_eq!(
            c.kql,
            "otel_logs | where service_name == \"payments\" | take 100"
        );
        assert!(c.dropped_clauses.is_empty());
    }

    #[test]
    fn unknown_field_is_dropped() {
        let t = table("http_reqs", &[("status", DataType::Int64)]);
        let c = compile_for_source(
            &t,
            &[Clause::Eq {
                field: "service_name".into(),
                value: "payments".into(),
            }],
            None,
            100,
        );
        assert_eq!(c.kql, "http_reqs | take 100");
        assert_eq!(c.dropped_clauses.len(), 1);
        assert_eq!(c.dropped_clauses[0].reason, DropReason::UnknownField);
    }

    #[test]
    fn numeric_cmp_on_string_column_is_dropped() {
        let t = table("otel_logs", &[("message", DataType::Utf8)]);
        let c = compile_for_source(
            &t,
            &[Clause::Cmp {
                field: "message".into(),
                op: CmpOp::Gt,
                value: "100".into(),
            }],
            None,
            100,
        );
        assert_eq!(c.kql, "otel_logs | take 100");
        assert_eq!(c.dropped_clauses[0].reason, DropReason::TypeMismatch);
    }

    #[test]
    fn numeric_cmp_on_int_column_compiles() {
        let t = table("http_reqs", &[("status", DataType::Int64)]);
        let c = compile_for_source(
            &t,
            &[Clause::Cmp {
                field: "status".into(),
                op: CmpOp::Gt,
                value: "500".into(),
            }],
            None,
            100,
        );
        assert_eq!(c.kql, "http_reqs | where status > 500 | take 100");
    }

    #[test]
    fn substring_expands_to_disjunction_over_string_columns() {
        let t = table(
            "otel_logs",
            &[
                ("timestamp", ts()),
                ("message", DataType::Utf8),
                ("service", DataType::Utf8),
                ("status", DataType::Int64),
            ],
        );
        let c = compile_for_source(
            &t,
            &[Clause::Substring {
                value: "auth".into(),
            }],
            None,
            50,
        );
        // Order of disjuncts follows schema field order.
        assert_eq!(
            c.kql,
            "otel_logs | where (message contains \"auth\" or service contains \"auth\") | take 50"
        );
    }

    #[test]
    fn substring_with_no_string_columns_is_dropped() {
        let t = table("metrics", &[("value", DataType::Float64)]);
        let c = compile_for_source(
            &t,
            &[Clause::Substring {
                value: "auth".into(),
            }],
            None,
            50,
        );
        assert_eq!(c.kql, "metrics | take 50");
        assert_eq!(c.dropped_clauses[0].reason, DropReason::TypeMismatch);
    }

    #[test]
    fn exists_compiles_to_isnotnull() {
        let t = table("otel_logs", &[("trace_id", DataType::Utf8)]);
        let c = compile_for_source(
            &t,
            &[Clause::Exists {
                field: "trace_id".into(),
            }],
            None,
            10,
        );
        assert_eq!(c.kql, "otel_logs | where isnotnull(trace_id) | take 10");
    }

    #[test]
    fn time_range_applied_when_table_has_timestamp() {
        let t = table(
            "otel_logs",
            &[("timestamp", ts()), ("message", DataType::Utf8)],
        );
        let c = compile_for_source(
            &t,
            &[],
            Some(&TimeRange {
                from_ms: 1_700_000_000_000,
                to_ms: 1_700_000_900_000,
            }),
            100,
        );
        assert_eq!(
            c.kql,
            "otel_logs | where timestamp >= datetime(\"2023-11-14T22:13:20.000Z\") and timestamp < datetime(\"2023-11-14T22:28:20.000Z\") | take 100"
        );
    }

    #[test]
    fn time_range_skipped_when_table_has_no_timestamp() {
        let t = table("metrics", &[("value", DataType::Float64)]);
        let c = compile_for_source(
            &t,
            &[],
            Some(&TimeRange {
                from_ms: 0,
                to_ms: 1,
            }),
            100,
        );
        assert_eq!(c.kql, "metrics | take 100");
        assert!(!c.has_timestamp);
        assert_eq!(c.timestamp_column, None);
    }

    #[test]
    fn value_with_double_quote_is_escaped() {
        let t = table("otel_logs", &[("message", DataType::Utf8)]);
        let c = compile_for_source(
            &t,
            &[Clause::Eq {
                field: "message".into(),
                value: "say \"hi\"".into(),
            }],
            None,
            10,
        );
        assert_eq!(
            c.kql,
            "otel_logs | where message == \"say \\\"hi\\\"\" | take 10"
        );
    }

    #[test]
    fn substring_accepts_utf8_view_columns() {
        let t = table(
            "otel_logs",
            &[
                (
                    "timestamp",
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                ),
                ("message", DataType::Utf8View),
            ],
        );
        let c = compile_for_source(
            &t,
            &[Clause::Substring {
                value: "auth".into(),
            }],
            None,
            50,
        );
        assert_eq!(
            c.kql,
            "otel_logs | where message contains \"auth\" | take 50"
        );
    }

    #[test]
    fn cmp_on_decimal_column_compiles() {
        let t = table("billing", &[("amount", DataType::Decimal128(18, 2))]);
        let c = compile_for_source(
            &t,
            &[Clause::Cmp {
                field: "amount".into(),
                op: CmpOp::Gt,
                value: "100.50".into(),
            }],
            None,
            100,
        );
        assert_eq!(c.kql, "billing | where amount > 100.50 | take 100");
    }

    #[test]
    fn time_range_uses_first_timestamp_column_by_name() {
        let t = table(
            "events",
            &[
                (
                    "event_time",
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                ),
                ("payload", DataType::Utf8),
            ],
        );
        let c = compile_for_source(
            &t,
            &[],
            Some(&TimeRange {
                from_ms: 1_700_000_000_000,
                to_ms: 1_700_000_900_000,
            }),
            100,
        );
        assert_eq!(
            c.kql,
            "events | where event_time >= datetime(\"2023-11-14T22:13:20.000Z\") and event_time < datetime(\"2023-11-14T22:28:20.000Z\") | take 100"
        );
        assert!(c.has_timestamp);
        assert_eq!(c.timestamp_column, Some("event_time".to_string()));
    }

    #[test]
    fn firehose_shape_prefers_iso_string_ts_over_reserved_at() {
        // Mirrors `claude_code_events`: reserved `at` (Timestamp, NULL in
        // practice) plus the real event time in an ISO-8601 string `ts`.
        let t = table(
            "claude_code_events",
            &[
                ("at", ts()),
                ("ts", DataType::Utf8),
                ("kind", DataType::Utf8),
            ],
        );
        let c = compile_for_source(
            &t,
            &[],
            Some(&TimeRange {
                from_ms: 1_700_000_000_000,
                to_ms: 1_700_000_900_000,
            }),
            100,
        );
        // Detection picks `ts`, and the predicate is a plain string range
        // (NOT datetime(...)), so it actually matches the stored ISO strings.
        assert_eq!(
            c.kql,
            "claude_code_events | where ts >= \"2023-11-14T22:13:20.000Z\" and ts < \"2023-11-14T22:28:20.000Z\" | take 100"
        );
        assert!(c.has_timestamp);
        assert_eq!(c.timestamp_column, Some("ts".to_string()));
    }

    #[test]
    fn populated_at_is_used_when_no_string_time_column() {
        // A healthy default-schema table whose `at` ingest actually filled.
        let t = table("events", &[("at", ts()), ("label", DataType::Utf8)]);
        let c = compile_for_source(
            &t,
            &[],
            Some(&TimeRange {
                from_ms: 1_700_000_000_000,
                to_ms: 1_700_000_900_000,
            }),
            100,
        );
        assert_eq!(
            c.kql,
            "events | where at >= datetime(\"2023-11-14T22:13:20.000Z\") and at < datetime(\"2023-11-14T22:28:20.000Z\") | take 100"
        );
        assert_eq!(c.timestamp_column, Some("at".to_string()));
    }

    #[test]
    fn declared_timestamp_column_beats_iso_string() {
        // A real Timestamp column (not `at`) wins over a string `ts`.
        let t = table(
            "otel",
            &[("event_time", ts()), ("ts", DataType::Utf8)],
        );
        let c = compile_for_source(&t, &[], None, 10);
        assert_eq!(c.timestamp_column, Some("event_time".to_string()));
    }
}
