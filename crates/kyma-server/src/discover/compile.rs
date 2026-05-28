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
        if let Some(col) = &ts_col {
            where_parts.push(format!(
                "{col} >= datetime({from}) and {col} < datetime({to})",
                col = col,
                from = tr.from_ms,
                to = tr.to_ms,
            ));
        }
    }

    let mut kql = source.name.clone();
    for p in &where_parts {
        kql.push_str(" | where ");
        kql.push_str(p);
    }
    kql.push_str(&format!(" | take {per_source_limit}"));

    CompiledSource {
        kql,
        dropped_clauses: dropped,
        has_timestamp,
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

fn find_timestamp_column(source: &TableRef) -> Option<String> {
    source.schema.fields().iter().find_map(|f| {
        if matches!(f.data_type(), DataType::Timestamp(_, _)) {
            Some(f.name().clone())
        } else {
            None
        }
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
        assert_eq!(
            c.kql,
            "otel_logs | where isnotnull(trace_id) | take 10"
        );
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
            "otel_logs | where timestamp >= datetime(1700000000000) and timestamp < datetime(1700000900000) | take 100"
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
                ("timestamp", DataType::Timestamp(TimeUnit::Microsecond, None)),
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
                ("event_time", DataType::Timestamp(TimeUnit::Microsecond, None)),
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
            "events | where event_time >= datetime(1700000000000) and event_time < datetime(1700000900000) | take 100"
        );
        assert!(c.has_timestamp);
    }
}
