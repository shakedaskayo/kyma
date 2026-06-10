//! KQL parser — phase E.1.
//!
//! Translates a KQL subset to SQL that DataFusion executes. The MVP covers
//! the most common ADX patterns:
//!
//! ```kql
//! nginx_logs
//! | where timestamp > ago(1h) and status >= 500
//! | project timestamp, path, status
//! | sort by timestamp desc
//! | take 10
//! ```
//!
//! ```kql
//! nginx_logs
//! | where body contains "OutOfMemory"
//! | summarize count(), avg(latency_ms) by bin(timestamp, 5m), status
//! ```
//!
//! # What's supported
//!
//! - Operators: `where`, `project`, `project-away`, `extend`, `summarize ...
//!   by ...`, `take`, `limit`, `sort by`, `order by`, `top N by`, `count`,
//!   `distinct`.
//! - `union` (leading-only): `union [withsource=Col] A, (B | …), … | ops`
//!   with ADX-default outer-by-name semantics (column superset, null-fill).
//!   Needs schema context — use [`kql_to_sql_with_schemas`].
//! - Expressions: literals (int, float, string, bool, duration, datetime),
//!   column refs, arithmetic `+ - * / %`, comparison `== != < > <= >=`,
//!   logical `and or not`, string `contains`/`startswith`/`endswith`/`has`.
//! - Functions: `now()`, `ago(d)`, `bin(col, d)`, `startofhour/day(col)`,
//!   `strcat(a,b)`, `tolower(s)`, `toupper(s)`, aggregates `count()`,
//!   `sum(x)`, `avg(x)`, `min(x)`, `max(x)`, `dcount(x)`.
//! - Duration literals: `30s`, `5m`, `2h`, `7d`.
//! - Datetime literals: `datetime(2026-04-19T10:00:00Z)`.
//!
//! # What's deferred
//!
//! `join`, `make-series`, `mv-expand`, regex, `parse_json`, lookup tables,
//! scalar-valued subqueries, materialized views.
//!
//! # Not building an AST
//!
//! The MVP lowers KQL directly to SQL as it parses, via a `QueryState`
//! accumulator. This is enough for KQL→SQL→DataFusion correctness; richer
//! semantics (e.g., `make-series` auto-fill) need a proper IR and land
//! with the unified-plan work in Phase E.2.

#![forbid(unsafe_code)]

mod cypher;
mod lexer;
mod parser;
mod state;

pub use cypher::{cypher_to_kql, GraphBinding};
pub use parser::{kql_to_sql, kql_to_sql_with_schemas, ParseError, SchemaMap};
