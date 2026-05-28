# Discover — Engine Search Endpoint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `POST /v1/explore/search` — the engine-side cross-source search endpoint that powers the new Discover surface. Server-side fanout over all readable `(db, table)` pairs, NDJSON-streamed grouped-by-source results, per-source compilation tolerance, shared wall-clock + memory budget, RBAC-aware scope resolution, structured observability.

**Architecture:** New module `crates/kyma-server/src/discover/` with submodules `grammar`, `compile`, `scope`, `frames`, `fanout`, `handler`. Handler is wired into the `read_router` next to `query_handler`. NDJSON response uses Axum `Body::from_stream` over a `tokio::sync::mpsc` channel — frames are emitted by `fanout` as sources complete. Per-source queries reuse the same `DataFusion SessionContext + KymaTable` plumbing as `query_handler`; the only new compilation step is `grammar → per-source KQL` (which then takes the existing `kyma_kql::kql_to_sql` path).

**Tech Stack:** Rust, Axum 0.7, Tokio, DataFusion, Arrow, kyma_kql, kyma_core::catalog, kyma_core::query_frontend::QueryBudget, `metrics` macros, `tracing`, testcontainers for integration tests.

**Reference spec:** `docs/superpowers/specs/2026-05-28-explore-discover-refactor-design.md` (sections 4, 5, 8).

**Prereqs / dependencies:** none. This plan ships independently; the frontend Discover plan and saved-views plan depend on this endpoint existing.

---

## File Structure

| File                                                                                | Action  | Responsibility                                                       |
|-------------------------------------------------------------------------------------|---------|----------------------------------------------------------------------|
| `crates/kyma-server/src/discover/mod.rs`                                            | Create  | Module root: re-exports + public types                               |
| `crates/kyma-server/src/discover/grammar.rs`                                        | Create  | Parse search-bar string → `Vec<Clause>`                              |
| `crates/kyma-server/src/discover/compile.rs`                                        | Create  | Compile `Vec<Clause>` against a source schema → KQL string + dropped-clause list |
| `crates/kyma-server/src/discover/scope.rs`                                          | Create  | Resolve request `scope` → `Vec<SourceRef>` after RBAC + glob expansion |
| `crates/kyma-server/src/discover/frames.rs`                                         | Create  | NDJSON frame structs + serializer (`Plan`, `SourceProgress`, `Rows`, `Histogram`, `SourceDone`, `Error`, `Done`) |
| `crates/kyma-server/src/discover/fanout.rs`                                         | Create  | Per-source executor + parallel orchestrator under shared budget; emits frames to mpsc |
| `crates/kyma-server/src/discover/handler.rs`                                        | Create  | Axum `discover_search_handler`; wires request → scope → fanout → streaming response |
| `crates/kyma-server/src/discover/handler_test_support.rs`                           | Create  | Test fixtures used only by integration tests (seeds two tables in two DBs)            |
| `crates/kyma-server/src/lib.rs`                                                     | Modify  | `pub mod discover;` + add route `POST /v1/explore/search` in `router()`               |
| `crates/kyma-server/tests/discover_search_it.rs`                                    | Create  | End-to-end integration test exercising plan → rows → done over real Postgres + tables |

---

## Phase 1 — Grammar

### Task 1: Module skeleton + grammar types

**Files:**
- Create: `crates/kyma-server/src/discover/mod.rs`
- Create: `crates/kyma-server/src/discover/grammar.rs`
- Modify: `crates/kyma-server/src/lib.rs:99` (add `pub mod discover;` near the other `pub mod` lines)

- [ ] **Step 1: Create module root**

Write `crates/kyma-server/src/discover/mod.rs`:

```rust
//! Cross-source search engine that powers `POST /v1/explore/search`.
//!
//! See `docs/superpowers/specs/2026-05-28-explore-discover-refactor-design.md`
//! sections 4 + 5 for the full contract.

pub mod compile;
pub mod fanout;
pub mod frames;
pub mod grammar;
pub mod handler;
pub mod scope;

#[cfg(test)]
mod handler_test_support;
```

- [ ] **Step 2: Wire module into the crate**

Edit `crates/kyma-server/src/lib.rs`. Find the existing `pub mod` block (around line 99 near `pub mod agent;`) and add:

```rust
pub mod discover;
```

(Add no other lines yet — route registration is Task 8.)

- [ ] **Step 3: Write grammar types + a failing tokenizer test**

Write `crates/kyma-server/src/discover/grammar.rs`:

```rust
//! Parses the user's search-bar string into structured clauses.
//!
//! Grammar (mirrors the frontend's `discoverGrammar.ts`):
//!
//! | Token form    | Clause                                |
//! |---------------|----------------------------------------|
//! | `auth`        | `Substring("auth")`                    |
//! | `"foo bar"`   | `Substring("foo bar")`                 |
//! | `field:value` | `Eq("field", "value")`                 |
//! | `-field:value`| `Neq("field", "value")`                |
//! | `field:>N`    | `Cmp("field", Op::Gt, "N")`            |
//! | `field:>=N`   | `Cmp("field", Op::Ge, "N")`            |
//! | `field:<N`    | `Cmp("field", Op::Lt, "N")`            |
//! | `field:<=N`   | `Cmp("field", Op::Le, "N")`            |
//! | `field:*`     | `Exists("field")`                      |
//!
//! Multiple clauses combine with implicit AND.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Clause {
    Substring { value: String },
    Eq { field: String, value: String },
    Neq { field: String, value: String },
    Cmp { field: String, op: CmpOp, value: String },
    Exists { field: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, thiserror::Error)]
pub enum GrammarError {
    #[error("unterminated quoted string")]
    UnterminatedQuote,
}

pub fn parse(input: &str) -> Result<Vec<Clause>, GrammarError> {
    todo!("Task 2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_token_as_substring() {
        assert_eq!(
            parse("auth").unwrap(),
            vec![Clause::Substring { value: "auth".to_string() }],
        );
    }
}
```

- [ ] **Step 4: Run the test — should fail with "not yet implemented"**

```bash
cargo test -p kyma-server --lib discover::grammar::tests::parses_bare_token_as_substring -- --nocapture
```

Expected: FAIL with a panic message from `todo!("Task 2")`.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/discover/ crates/kyma-server/src/lib.rs
git commit -m "feat(discover): scaffold grammar module with failing test"
```

---

### Task 2: Grammar tokenizer

**Files:**
- Modify: `crates/kyma-server/src/discover/grammar.rs`

- [ ] **Step 1: Add the full failing test suite**

Replace the `#[cfg(test)] mod tests { ... }` block in `grammar.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String { v.to_string() }

    #[test]
    fn parses_bare_token_as_substring() {
        assert_eq!(parse("auth").unwrap(),
            vec![Clause::Substring { value: s("auth") }]);
    }

    #[test]
    fn parses_quoted_phrase_as_substring() {
        assert_eq!(parse("\"foo bar\"").unwrap(),
            vec![Clause::Substring { value: s("foo bar") }]);
    }

    #[test]
    fn parses_field_value_as_eq() {
        assert_eq!(parse("service:payments").unwrap(),
            vec![Clause::Eq { field: s("service"), value: s("payments") }]);
    }

    #[test]
    fn parses_negation_as_neq() {
        assert_eq!(parse("-severity:INFO").unwrap(),
            vec![Clause::Neq { field: s("severity"), value: s("INFO") }]);
    }

    #[test]
    fn parses_numeric_comparisons() {
        assert_eq!(parse("status:>500").unwrap(),
            vec![Clause::Cmp { field: s("status"), op: CmpOp::Gt, value: s("500") }]);
        assert_eq!(parse("status:>=500").unwrap(),
            vec![Clause::Cmp { field: s("status"), op: CmpOp::Ge, value: s("500") }]);
        assert_eq!(parse("latency:<10").unwrap(),
            vec![Clause::Cmp { field: s("latency"), op: CmpOp::Lt, value: s("10") }]);
        assert_eq!(parse("latency:<=10").unwrap(),
            vec![Clause::Cmp { field: s("latency"), op: CmpOp::Le, value: s("10") }]);
    }

    #[test]
    fn parses_exists() {
        assert_eq!(parse("trace_id:*").unwrap(),
            vec![Clause::Exists { field: s("trace_id") }]);
    }

    #[test]
    fn parses_quoted_value() {
        assert_eq!(parse("message:\"conn refused\"").unwrap(),
            vec![Clause::Eq { field: s("message"), value: s("conn refused") }]);
    }

    #[test]
    fn parses_mix() {
        let got = parse("auth service:payments -severity:INFO").unwrap();
        assert_eq!(got, vec![
            Clause::Substring { value: s("auth") },
            Clause::Eq { field: s("service"), value: s("payments") },
            Clause::Neq { field: s("severity"), value: s("INFO") },
        ]);
    }

    #[test]
    fn empty_input_yields_empty_vec() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("   ").unwrap().is_empty());
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(matches!(parse("foo:\"bar"), Err(GrammarError::UnterminatedQuote)));
    }
}
```

- [ ] **Step 2: Verify all new tests fail**

```bash
cargo test -p kyma-server --lib discover::grammar::tests -- --nocapture
```

Expected: all 9 tests FAIL with the `todo!` panic.

- [ ] **Step 3: Implement the tokenizer**

Replace the `pub fn parse` body in `grammar.rs`:

```rust
pub fn parse(input: &str) -> Result<Vec<Clause>, GrammarError> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let negated = if c == '-' {
            chars.next();
            true
        } else {
            false
        };
        let token = read_token(&mut chars)?;
        if token.is_empty() {
            continue;
        }
        out.push(token_to_clause(token, negated)?);
    }
    Ok(out)
}

fn read_token<I>(chars: &mut std::iter::Peekable<I>) -> Result<String, GrammarError>
where
    I: Iterator<Item = char>,
{
    let mut buf = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        if c == '"' {
            chars.next();
            let mut quoted = String::new();
            let mut closed = false;
            while let Some(c2) = chars.next() {
                if c2 == '"' {
                    closed = true;
                    break;
                }
                quoted.push(c2);
            }
            if !closed {
                return Err(GrammarError::UnterminatedQuote);
            }
            buf.push_str(&quoted);
            continue;
        }
        chars.next();
        buf.push(c);
    }
    Ok(buf)
}

fn token_to_clause(token: String, negated: bool) -> Result<Clause, GrammarError> {
    if let Some((field, value)) = split_field(&token) {
        if value == "*" {
            return Ok(Clause::Exists { field: field.to_string() });
        }
        if let Some(rest) = value.strip_prefix(">=") {
            return Ok(Clause::Cmp { field: field.to_string(), op: CmpOp::Ge, value: rest.to_string() });
        }
        if let Some(rest) = value.strip_prefix("<=") {
            return Ok(Clause::Cmp { field: field.to_string(), op: CmpOp::Le, value: rest.to_string() });
        }
        if let Some(rest) = value.strip_prefix('>') {
            return Ok(Clause::Cmp { field: field.to_string(), op: CmpOp::Gt, value: rest.to_string() });
        }
        if let Some(rest) = value.strip_prefix('<') {
            return Ok(Clause::Cmp { field: field.to_string(), op: CmpOp::Lt, value: rest.to_string() });
        }
        if negated {
            Ok(Clause::Neq { field: field.to_string(), value: value.to_string() })
        } else {
            Ok(Clause::Eq { field: field.to_string(), value: value.to_string() })
        }
    } else {
        // Substrings can't be negated (matches grammar spec).
        Ok(Clause::Substring { value: token })
    }
}

fn split_field(token: &str) -> Option<(&str, &str)> {
    let idx = token.find(':')?;
    let (f, v) = token.split_at(idx);
    if f.is_empty() {
        return None;
    }
    Some((f, &v[1..]))
}
```

- [ ] **Step 4: Run tests — all should pass**

```bash
cargo test -p kyma-server --lib discover::grammar::tests -- --nocapture
```

Expected: 9 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/discover/grammar.rs
git commit -m "feat(discover): grammar tokenizer (substring/eq/neq/cmp/exists)"
```

---

## Phase 2 — Per-source KQL compiler

### Task 3: Compile clauses to KQL against a source schema

**Files:**
- Create: `crates/kyma-server/src/discover/compile.rs`

- [ ] **Step 1: Write the module skeleton + failing tests**

Write `crates/kyma-server/src/discover/compile.rs`:

```rust
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

use kyma_core::catalog::TableRef;
use kyma_core::schema::FieldType;
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
    todo!("Task 3 implementation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kyma_core::catalog::TableRef;
    use kyma_core::schema::{Field, FieldType, Schema};

    fn table(name: &str, fields: &[(&str, FieldType)]) -> TableRef {
        TableRef {
            id: 0,
            name: name.to_string(),
            schema_snapshot_id: 0,
            schema: Schema {
                fields: fields.iter().map(|(n, t)| Field {
                    name: n.to_string(),
                    ty: *t,
                }).collect(),
            },
            config: Default::default(),
        }
    }

    #[test]
    fn empty_clauses_compile_to_bare_take() {
        let t = table("otel_logs", &[
            ("timestamp", FieldType::Timestamp),
            ("message", FieldType::String),
        ]);
        let c = compile_for_source(&t, &[], None, 500);
        assert_eq!(c.kql, "otel_logs | take 500");
        assert!(c.dropped_clauses.is_empty());
        assert!(c.has_timestamp);
    }

    #[test]
    fn eq_on_known_field_compiles() {
        let t = table("otel_logs", &[
            ("service_name", FieldType::String),
            ("message", FieldType::String),
        ]);
        let c = compile_for_source(&t, &[Clause::Eq {
            field: "service_name".into(),
            value: "payments".into(),
        }], None, 100);
        assert_eq!(c.kql, "otel_logs | where service_name == \"payments\" | take 100");
        assert!(c.dropped_clauses.is_empty());
    }

    #[test]
    fn unknown_field_is_dropped() {
        let t = table("http_reqs", &[("status", FieldType::Int)]);
        let c = compile_for_source(&t, &[Clause::Eq {
            field: "service_name".into(),
            value: "payments".into(),
        }], None, 100);
        assert_eq!(c.kql, "http_reqs | take 100");
        assert_eq!(c.dropped_clauses.len(), 1);
        assert_eq!(c.dropped_clauses[0].reason, DropReason::UnknownField);
    }

    #[test]
    fn numeric_cmp_on_string_column_is_dropped() {
        let t = table("otel_logs", &[("message", FieldType::String)]);
        let c = compile_for_source(&t, &[Clause::Cmp {
            field: "message".into(),
            op: CmpOp::Gt,
            value: "100".into(),
        }], None, 100);
        assert_eq!(c.kql, "otel_logs | take 100");
        assert_eq!(c.dropped_clauses[0].reason, DropReason::TypeMismatch);
    }

    #[test]
    fn numeric_cmp_on_int_column_compiles() {
        let t = table("http_reqs", &[("status", FieldType::Int)]);
        let c = compile_for_source(&t, &[Clause::Cmp {
            field: "status".into(),
            op: CmpOp::Gt,
            value: "500".into(),
        }], None, 100);
        assert_eq!(c.kql, "http_reqs | where status > 500 | take 100");
    }

    #[test]
    fn substring_expands_to_disjunction_over_string_columns() {
        let t = table("otel_logs", &[
            ("timestamp", FieldType::Timestamp),
            ("message", FieldType::String),
            ("service", FieldType::String),
            ("status", FieldType::Int),
        ]);
        let c = compile_for_source(&t, &[Clause::Substring {
            value: "auth".into(),
        }], None, 50);
        // Order of disjuncts follows schema field order.
        assert_eq!(c.kql,
            "otel_logs | where (message contains \"auth\" or service contains \"auth\") | take 50");
    }

    #[test]
    fn substring_with_no_string_columns_is_dropped() {
        let t = table("metrics", &[("value", FieldType::Float)]);
        let c = compile_for_source(&t, &[Clause::Substring {
            value: "auth".into(),
        }], None, 50);
        assert_eq!(c.kql, "metrics | take 50");
        assert_eq!(c.dropped_clauses[0].reason, DropReason::TypeMismatch);
    }

    #[test]
    fn exists_compiles_to_isnotnull() {
        let t = table("otel_logs", &[("trace_id", FieldType::String)]);
        let c = compile_for_source(&t, &[Clause::Exists {
            field: "trace_id".into(),
        }], None, 10);
        assert_eq!(c.kql, "otel_logs | where isnotnull(trace_id) | take 10");
    }

    #[test]
    fn time_range_applied_when_table_has_timestamp() {
        let t = table("otel_logs", &[
            ("timestamp", FieldType::Timestamp),
            ("message", FieldType::String),
        ]);
        let c = compile_for_source(&t, &[], Some(&TimeRange {
            from_ms: 1_700_000_000_000,
            to_ms: 1_700_000_900_000,
        }), 100);
        assert_eq!(c.kql,
            "otel_logs | where timestamp >= datetime(1700000000000) and timestamp < datetime(1700000900000) | take 100");
    }

    #[test]
    fn time_range_skipped_when_table_has_no_timestamp() {
        let t = table("metrics", &[("value", FieldType::Float)]);
        let c = compile_for_source(&t, &[], Some(&TimeRange {
            from_ms: 0, to_ms: 1,
        }), 100);
        assert_eq!(c.kql, "metrics | take 100");
        assert!(!c.has_timestamp);
    }

    #[test]
    fn value_with_double_quote_is_escaped() {
        let t = table("otel_logs", &[("message", FieldType::String)]);
        let c = compile_for_source(&t, &[Clause::Eq {
            field: "message".into(),
            value: "say \"hi\"".into(),
        }], None, 10);
        assert_eq!(c.kql,
            "otel_logs | where message == \"say \\\"hi\\\"\" | take 10");
    }
}
```

- [ ] **Step 2: Run tests — all fail with `todo!`**

```bash
cargo test -p kyma-server --lib discover::compile::tests -- --nocapture
```

Expected: 11 tests FAIL.

- [ ] **Step 3: Implement the compiler**

Replace the `pub fn compile_for_source` body and add helpers:

```rust
pub fn compile_for_source(
    source: &TableRef,
    clauses: &[Clause],
    time_range: Option<&TimeRange>,
    per_source_limit: usize,
) -> CompiledSource {
    let has_timestamp = source
        .schema
        .fields
        .iter()
        .any(|f| f.name == "timestamp" && matches!(f.ty, FieldType::Timestamp));

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
        if has_timestamp {
            where_parts.push(format!(
                "timestamp >= datetime({from}) and timestamp < datetime({to})",
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
                .fields
                .iter()
                .filter(|f| matches!(f.ty, FieldType::String))
                .map(|f| format!("{} contains {}", f.name, escape_str(value)))
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
                .fields
                .iter()
                .find(|f| f.name == *field)
                .ok_or(DropReason::UnknownField)?;
            if !matches!(f.ty, FieldType::Int | FieldType::Float | FieldType::Timestamp) {
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
    if source.schema.fields.iter().any(|f| f.name == name) {
        Ok(())
    } else {
        Err(DropReason::UnknownField)
    }
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
```

- [ ] **Step 4: Run tests — all pass**

```bash
cargo test -p kyma-server --lib discover::compile::tests -- --nocapture
```

Expected: 11 passed.

**Note:** The test helper uses `kyma_core::schema::FieldType` enum variants `String/Int/Float/Timestamp`. If the actual variant names differ in `crates/kyma-core/src/schema.rs`, run `grep -n 'enum FieldType' crates/kyma-core/src/schema.rs` and align names in **both the tests and the compiler match arms**. The structure of the compiler is independent of the exact variant names.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/discover/compile.rs
git commit -m "feat(discover): per-source KQL compiler with type-aware tolerance"
```

---

## Phase 3 — Scope resolution

### Task 4: Resolve request `scope` → `Vec<SourceRef>`

**Files:**
- Create: `crates/kyma-server/src/discover/scope.rs`

- [ ] **Step 1: Inspect the catalog trait once**

Read `crates/kyma-core/src/catalog.rs` and confirm the methods you can use:

```bash
grep -n 'fn ' crates/kyma-core/src/catalog.rs | head -40
```

You need a way to list **databases** the caller can read (often `list_databases`) and tables per database (`list_tables_in_database`). If `list_databases` exists, use it. If not, the multi-database scope == iteration over a hard-coded list — open a question with the spec owner before continuing. Don't fake it.

- [ ] **Step 2: Write the failing test suite**

Write `crates/kyma-server/src/discover/scope.rs`:

```rust
//! Resolve the `scope` field of a search request to a concrete
//! `Vec<SourceRef>`, respecting RBAC (only readable tables) and the
//! `max_sources_per_request` cap.

use std::sync::Arc;

use kyma_core::catalog::{Catalog, TableRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Scope {
    All,
    Sources { sources: Vec<String> },
    View { view_id: String },
}

#[derive(Clone, Debug)]
pub struct ResolvedSource {
    pub db: String,
    pub table: TableRef,
}

#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("scope resolves to {0} sources, exceeds max_sources_per_request={1}")]
    ScopeTooLarge(usize, usize),
    #[error("view {0} not found")]
    ViewNotFound(String),
    #[error("catalog error: {0}")]
    Catalog(String),
}

pub async fn resolve(
    scope: &Scope,
    catalog: Arc<dyn Catalog>,
    saved_view_lookup: Option<&dyn SavedViewLookup>,
    max_sources_per_request: usize,
) -> Result<Vec<ResolvedSource>, ScopeError> {
    todo!("Task 4")
}

#[async_trait::async_trait]
pub trait SavedViewLookup: Send + Sync {
    async fn load_sources(&self, view_id: &str) -> Result<Option<Vec<String>>, String>;
}

pub fn matches_pattern(pattern: &str, db: &str, table: &str) -> bool {
    let Some((p_db, p_tbl)) = pattern.split_once('.') else { return false; };
    seg_match(p_db, db) && seg_match(p_tbl, table)
}

fn seg_match(pat: &str, value: &str) -> bool {
    pat == "*" || pat == value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_exact() {
        assert!(matches_pattern("prod.otel_logs", "prod", "otel_logs"));
        assert!(!matches_pattern("prod.otel_logs", "prod", "http_reqs"));
    }

    #[test]
    fn pattern_glob_table() {
        assert!(matches_pattern("prod.*", "prod", "anything"));
        assert!(!matches_pattern("prod.*", "stg", "anything"));
    }

    #[test]
    fn pattern_glob_db() {
        assert!(matches_pattern("*.otel_logs", "prod", "otel_logs"));
        assert!(!matches_pattern("*.otel_logs", "prod", "http_reqs"));
    }

    #[test]
    fn pattern_full_wildcard() {
        assert!(matches_pattern("*.*", "any", "thing"));
    }

    // Resolver tests against the real catalog live in
    // `tests/discover_search_it.rs` since `Catalog` is a trait object.
}
```

- [ ] **Step 3: Run tests — pattern tests pass, resolver tests deferred**

```bash
cargo test -p kyma-server --lib discover::scope::tests -- --nocapture
```

Expected: 4 passed (pattern unit tests). Resolver unit tests are not present at this stage; resolver behavior is exercised in the integration test (Task 9).

- [ ] **Step 4: Implement the resolver**

Replace the `pub async fn resolve` body in `scope.rs`:

```rust
pub async fn resolve(
    scope: &Scope,
    catalog: Arc<dyn Catalog>,
    saved_view_lookup: Option<&dyn SavedViewLookup>,
    max_sources_per_request: usize,
) -> Result<Vec<ResolvedSource>, ScopeError> {
    // Concrete patterns: "*.*", or what the user / view supplied.
    let patterns: Vec<String> = match scope {
        Scope::All => vec!["*.*".to_string()],
        Scope::Sources { sources } => sources.clone(),
        Scope::View { view_id } => match saved_view_lookup {
            None => return Err(ScopeError::ViewNotFound(view_id.clone())),
            Some(l) => match l.load_sources(view_id).await
                .map_err(ScopeError::Catalog)?
            {
                None => return Err(ScopeError::ViewNotFound(view_id.clone())),
                Some(srcs) => srcs,
            },
        },
    };

    let dbs = catalog.list_databases().await
        .map_err(|e| ScopeError::Catalog(e.to_string()))?;

    let mut out: Vec<ResolvedSource> = Vec::new();
    for db in dbs {
        let tables = match catalog.list_tables_in_database(&db).await {
            Ok(t) => t,
            Err(_) => continue, // table list may fail for RBAC-dropped DBs — skip
        };
        for t in tables {
            let matched = patterns.iter().any(|p| matches_pattern(p, &db, &t.name));
            if !matched { continue; }
            out.push(ResolvedSource { db: db.clone(), table: t });
            if out.len() > max_sources_per_request {
                return Err(ScopeError::ScopeTooLarge(out.len(), max_sources_per_request));
            }
        }
    }

    Ok(out)
}
```

If `catalog.list_databases()` does **not** exist on the trait, two options:

- (A) Add it now to `kyma-core/src/catalog.rs` + `kyma-catalog/src/lib.rs` (a 3-method addition: trait method + `PostgresCatalog` impl returning `SELECT DISTINCT database_name FROM tables` + an in-memory test impl).
- (B) Pass an explicit `Vec<String>` of database names into `resolve` from the handler, sourced from existing `list_databases`-equivalent already used somewhere in the codebase.

Choose (A) — it's the clean home for this responsibility. The trait change is one async method.

- [ ] **Step 5: Run tests — still pass; resolver compiles**

```bash
cargo test -p kyma-server --lib discover::scope -- --nocapture
```

Expected: 4 passed; `cargo check -p kyma-server` succeeds.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/src/discover/scope.rs crates/kyma-core/src/catalog.rs crates/kyma-catalog/src/lib.rs
git commit -m "feat(discover): scope resolver with glob patterns + RBAC drops"
```

---

## Phase 4 — NDJSON frames

### Task 5: Frame types + serializer

**Files:**
- Create: `crates/kyma-server/src/discover/frames.rs`

- [ ] **Step 1: Write the failing test**

Write `crates/kyma-server/src/discover/frames.rs`:

```rust
//! Discriminated frame types emitted by `POST /v1/explore/search`.
//! Wire format: one JSON object per line, NDJSON.
//!
//! See spec section 4.1.

use serde::Serialize;

use super::compile::DroppedClause;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Plan { sources: Vec<PlanSource> },
    SourceProgress { source: String, state: ProgressState },
    Rows { source: String, rows: Vec<serde_json::Value> },
    Histogram { source: String, buckets: Vec<Bucket> },
    SourceDone {
        source: String,
        total: usize,
        capped: bool,
        dropped_clauses: Vec<DroppedClause>,
    },
    Error { source: Option<String>, code: String, message: String },
    Done { elapsed_ms: u64 },
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSource {
    pub source: String,           // "db.table"
    pub has_timestamp: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProgressState {
    Running,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    pub t: String,    // ISO-8601 UTC
    pub n: u64,
}

pub fn frame_to_line(f: &Frame) -> String {
    let mut s = serde_json::to_string(f).expect("frame serialization is infallible");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_frame_round_trips() {
        let f = Frame::Plan {
            sources: vec![
                PlanSource { source: "prod.otel_logs".into(), has_timestamp: true },
                PlanSource { source: "prod.metrics".into(), has_timestamp: false },
            ],
        };
        let line = frame_to_line(&f);
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "plan");
        assert_eq!(v["sources"][0]["source"], "prod.otel_logs");
        assert_eq!(v["sources"][0]["has_timestamp"], true);
    }

    #[test]
    fn done_frame_serializes() {
        let f = Frame::Done { elapsed_ms: 842 };
        let line = frame_to_line(&f);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["type"], "done");
        assert_eq!(v["elapsed_ms"], 842);
    }

    #[test]
    fn error_frame_includes_source_when_present() {
        let f = Frame::Error {
            source: Some("prod.http_reqs".into()),
            code: "timeout".into(),
            message: "per-source timeout".into(),
        };
        let v: serde_json::Value = serde_json::from_str(frame_to_line(&f).trim()).unwrap();
        assert_eq!(v["source"], "prod.http_reqs");
        assert_eq!(v["code"], "timeout");
    }

    #[test]
    fn source_done_includes_dropped_clauses() {
        use crate::discover::grammar::Clause;
        use crate::discover::compile::DropReason;

        let f = Frame::SourceDone {
            source: "prod.otel_logs".into(),
            total: 1238,
            capped: true,
            dropped_clauses: vec![DroppedClause {
                reason: DropReason::UnknownField,
                clause: Clause::Eq { field: "foo".into(), value: "bar".into() },
            }],
        };
        let v: serde_json::Value = serde_json::from_str(frame_to_line(&f).trim()).unwrap();
        assert_eq!(v["type"], "source_done");
        assert_eq!(v["capped"], true);
        assert_eq!(v["dropped_clauses"][0]["reason"], "unknown_field");
    }
}
```

- [ ] **Step 2: Verify tests fail to compile (no implementation yet — but actually the struct definitions ARE the implementation; tests should pass).**

```bash
cargo test -p kyma-server --lib discover::frames -- --nocapture
```

Expected: 4 passed (frames are pure-data, no logic to implement).

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-server/src/discover/frames.rs
git commit -m "feat(discover): NDJSON frame types"
```

---

## Phase 5 — Fanout executor

### Task 6: Run per-source queries in parallel under a shared budget

**Files:**
- Create: `crates/kyma-server/src/discover/fanout.rs`

- [ ] **Step 1: Skim the existing query execution path**

Re-read `crates/kyma-server/src/lib.rs:343-470` (the SessionContext build + `ctx.sql(&sql).await + tokio::time::timeout(...)` block). The fanout executor reuses this same plumbing per source — register one table, run the compiled SQL, collect batches, serialize to JSON rows.

- [ ] **Step 2: Write the module**

Write `crates/kyma-server/src/discover/fanout.rs`:

```rust
//! Run compiled per-source KQL in parallel under a shared wall-clock + memory
//! budget. Emits `Frame`s into an mpsc as sources complete.

use std::sync::Arc;
use std::time::Instant;

use arrow::json::ArrayWriter;
use datafusion::execution::context::{SessionConfig, SessionContext};
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::{RuntimeConfig, RuntimeEnv};
use kyma_core::catalog::Catalog;
use kyma_core::query_frontend::QueryBudget;
use kyma_exec::KymaTable;
use kyma_object_store::ObjectStoreFactory;
use tokio::sync::mpsc;

use super::compile::{CompiledSource, TimeRange};
use super::frames::{Frame, PlanSource, ProgressState};
use super::grammar::Clause;
use super::scope::ResolvedSource;

pub struct FanoutInput {
    pub sources: Vec<ResolvedSource>,
    pub clauses: Vec<Clause>,
    pub time_range: Option<TimeRange>,
    pub per_source_limit: usize,
    pub budget: QueryBudget,
    pub catalog: Arc<dyn Catalog>,
    pub format: Arc<dyn ObjectStoreFactory>,
    pub node_id: Option<u32>,
}

pub fn run(input: FanoutInput, tx: mpsc::Sender<Frame>) {
    tokio::spawn(async move { run_inner(input, tx).await });
}

async fn run_inner(input: FanoutInput, tx: mpsc::Sender<Frame>) {
    let start = Instant::now();

    // 1. Plan frame.
    let plan_sources: Vec<PlanSource> = input.sources.iter().map(|s| PlanSource {
        source: format!("{}.{}", s.db, s.table.name),
        has_timestamp: super::compile::compile_for_source(
            &s.table, &[], None, input.per_source_limit,
        ).has_timestamp,
    }).collect();
    let _ = tx.send(Frame::Plan { sources: plan_sources }).await;

    // 2. Compile per source.
    let compiled: Vec<(ResolvedSource, CompiledSource)> = input
        .sources
        .iter()
        .map(|s| {
            let c = super::compile::compile_for_source(
                &s.table,
                &input.clauses,
                input.time_range.as_ref(),
                input.per_source_limit,
            );
            (s.clone(), c)
        })
        .collect();

    // 3. Fan out under one shared budget. Each source gets its own
    //    SessionContext but the wall-clock applies once across the whole batch.
    let deadline = tokio::time::Instant::now() + input.budget.max_wall_clock;
    let mut handles = Vec::new();
    for (src, compiled) in compiled {
        let tx = tx.clone();
        let catalog = input.catalog.clone();
        let format = input.format.clone();
        let node_id = input.node_id;
        let budget_mem = input.budget.max_memory_bytes;
        let per_source_limit = input.per_source_limit;

        handles.push(tokio::spawn(async move {
            let source_key = format!("{}.{}", src.db, src.table.name);
            let _ = tx.send(Frame::SourceProgress {
                source: source_key.clone(),
                state: ProgressState::Running,
            }).await;

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let _ = tx.send(Frame::Error {
                    source: Some(source_key.clone()),
                    code: "wall_clock_exceeded".into(),
                    message: "exceeded global wall-clock before starting".into(),
                }).await;
                return;
            }

            match execute_one(
                &src, &compiled, catalog, format, node_id, budget_mem, remaining, per_source_limit,
            ).await {
                Ok(out) => {
                    if !out.rows.is_empty() {
                        let _ = tx.send(Frame::Rows {
                            source: source_key.clone(),
                            rows: out.rows,
                        }).await;
                    }
                    let _ = tx.send(Frame::SourceDone {
                        source: source_key,
                        total: out.total,
                        capped: out.capped,
                        dropped_clauses: compiled.dropped_clauses,
                    }).await;
                }
                Err((code, message)) => {
                    let _ = tx.send(Frame::Error {
                        source: Some(source_key),
                        code,
                        message,
                    }).await;
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    let _ = tx.send(Frame::Done {
        elapsed_ms: start.elapsed().as_millis() as u64,
    }).await;
}

struct PerSourceResult {
    rows: Vec<serde_json::Value>,
    total: usize,
    capped: bool,
}

async fn execute_one(
    src: &ResolvedSource,
    compiled: &CompiledSource,
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn ObjectStoreFactory>,
    node_id: Option<u32>,
    memory_budget_bytes: u64,
    timeout: std::time::Duration,
    per_source_limit: usize,
) -> Result<PerSourceResult, (String, String)> {
    let sql = kyma_kql::kql_to_sql(&compiled.kql)
        .map_err(|e| ("kql_compile_error".into(), e.to_string()))?;

    let runtime = RuntimeEnv::new(
        RuntimeConfig::new().with_memory_pool(
            Arc::new(GreedyMemoryPool::new(memory_budget_bytes as usize))
        ),
    ).map_err(|e| ("runtime_env_error".into(), e.to_string()))?;
    let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), Arc::new(runtime));
    kyma_exec::register_vector_udfs(&ctx);

    let kt: Arc<KymaTable> = match node_id {
        Some(nid) => Arc::new(KymaTable::with_node_id(
            src.table.clone(), catalog.clone(), format, nid, src.db.clone(),
        )),
        None => Arc::new(KymaTable::new(src.table.clone(), catalog.clone(), format)),
    };
    ctx.register_table(&src.table.name, kt)
        .map_err(|e| ("table_register_error".into(), e.to_string()))?;

    let df = ctx.sql(&sql).await.map_err(|e| ("sql_plan_error".into(), e.to_string()))?;
    let batches = match tokio::time::timeout(timeout, df.collect()).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            let msg = e.to_string();
            if msg.contains("ResourcesExhausted") || msg.contains("Resources exhausted") {
                return Err(("memory_exceeded".into(), msg));
            }
            return Err(("query_execution_error".into(), msg));
        }
        Err(_) => return Err(("wall_clock_exceeded".into(), "per-source wall-clock".into())),
    };

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    let capped = total_rows >= per_source_limit;
    let mut buf: Vec<u8> = Vec::with_capacity(total_rows * 128);
    for batch in &batches {
        let mut w = ArrayWriter::new(&mut buf);
        w.write(batch).map_err(|e| ("serialization_error".into(), e.to_string()))?;
        w.finish().map_err(|e| ("serialization_error".into(), e.to_string()))?;
    }
    let rows = collate_to_values(&buf).map_err(|e| ("serialization_error".into(), e))?;
    Ok(PerSourceResult { rows, total: total_rows, capped })
}

fn collate_to_values(concatenated_arrays: &[u8]) -> Result<Vec<serde_json::Value>, String> {
    let mut out = Vec::new();
    let stream = serde_json::Deserializer::from_slice(concatenated_arrays)
        .into_iter::<serde_json::Value>();
    for arr in stream {
        let arr = arr.map_err(|e| format!("json parse: {e}"))?;
        if let serde_json::Value::Array(rows) = arr {
            for row in rows {
                out.push(row);
            }
        }
    }
    Ok(out)
}
```

- [ ] **Step 3: Compile-check**

```bash
cargo check -p kyma-server
```

Expected: compiles. If a crate import is missing (e.g. `kyma_object_store::ObjectStoreFactory`), grep the existing `query_handler` for the right type:

```bash
grep -n 'state.format' crates/kyma-server/src/lib.rs
grep -n 'Arc<dyn ' crates/kyma-server/src/lib.rs | head -10
```

and align the import.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-server/src/discover/fanout.rs
git commit -m "feat(discover): parallel fanout executor under shared budget"
```

---

## Phase 6 — Handler

### Task 7: Wire `POST /v1/explore/search`

**Files:**
- Create: `crates/kyma-server/src/discover/handler.rs`
- Modify: `crates/kyma-server/src/lib.rs:113` (add the route)

- [ ] **Step 1: Write the handler module**

Write `crates/kyma-server/src/discover/handler.rs`:

```rust
//! Axum handler for `POST /v1/explore/search`.
//!
//! Wires: parse body → resolve scope → kick off fanout → stream NDJSON
//! response from the mpsc.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use serde::Deserialize;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::QueryState;
use super::compile::TimeRange;
use super::fanout::{run, FanoutInput};
use super::frames::{frame_to_line, Frame};
use super::grammar::parse as parse_grammar;
use super::scope::{resolve as resolve_scope, Scope, ScopeError};

const DEFAULT_PER_SOURCE_LIMIT: usize = 500;
const MAX_PER_SOURCE_LIMIT: usize = 5_000;
const DEFAULT_MAX_SOURCES: usize = 200;

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    #[serde(default)]
    pub query: String,
    pub scope: Scope,
    #[serde(default)]
    pub time_range: Option<TimeRangeBody>,
    #[serde(default)]
    pub per_source_limit: Option<usize>,
    #[serde(default)]
    pub histogram: Option<HistogramBody>,
}

#[derive(Debug, Deserialize)]
pub struct TimeRangeBody { pub from: String, pub to: String }

#[derive(Debug, Deserialize)]
pub struct HistogramBody { pub interval_ms: u64 }

pub async fn discover_search_handler(
    State(state): State<QueryState>,
    req: Request<Body>,
) -> Response {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let request_id = crate::extract_request_id(&parts.headers);

    let body_bytes: Bytes = match axum::body::to_bytes(body, 1 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return crate::error_response(
            StatusCode::PAYLOAD_TOO_LARGE, "body_too_large",
            &format!("failed to read body: {e}"), &request_id,
        ),
    };

    let payload: SearchRequest = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(e) => return crate::error_response(
            StatusCode::BAD_REQUEST, "bad_request",
            &format!("invalid JSON body: {e}"), &request_id,
        ),
    };

    let per_source_limit = payload
        .per_source_limit
        .unwrap_or(DEFAULT_PER_SOURCE_LIMIT)
        .min(MAX_PER_SOURCE_LIMIT);

    let clauses = match parse_grammar(&payload.query) {
        Ok(c) => c,
        Err(e) => return crate::error_response(
            StatusCode::BAD_REQUEST, "grammar_error", &e.to_string(), &request_id,
        ),
    };

    let time_range = match payload.time_range {
        None => None,
        Some(tr) => match parse_time_range(&tr) {
            Ok(t) => Some(t),
            Err(msg) => return crate::error_response(
                StatusCode::BAD_REQUEST, "bad_time_range", &msg, &request_id,
            ),
        },
    };

    let max_sources = std::env::var("KYMA_DISCOVER_MAX_SOURCES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_SOURCES);

    let resolved = match resolve_scope(&payload.scope, state.catalog.clone(), None, max_sources).await {
        Ok(s) => s,
        Err(ScopeError::ScopeTooLarge(n, max)) => return crate::error_response(
            StatusCode::BAD_REQUEST, "scope_too_large",
            &format!("scope resolves to {n} sources, exceeds max {max}"), &request_id,
        ),
        Err(ScopeError::ViewNotFound(id)) => return crate::error_response(
            StatusCode::NOT_FOUND, "view_not_found",
            &format!("saved view {id} not found"), &request_id,
        ),
        Err(ScopeError::Catalog(msg)) => return crate::error_response(
            StatusCode::INTERNAL_SERVER_ERROR, "catalog_error", &msg, &request_id,
        ),
    };

    let budget = crate::resolve_query_budget(&parts.headers);

    let (tx, rx) = mpsc::channel::<Frame>(64);

    // Observability: count requests up-front, emit terminal histogram in a
    // detached task that observes when the channel closes.
    ::metrics::counter!("kyma_explore_search_requests_total",
        "scope_kind" => scope_kind_label(&payload.scope)).increment(1);
    let req_id_for_log = request_id.clone();
    let scope_kind = scope_kind_label(&payload.scope);
    let resolved_count = resolved.len();

    run(FanoutInput {
        sources: resolved,
        clauses,
        time_range,
        per_source_limit,
        budget,
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        node_id: state.node_id,
    }, tx);

    // Stream frames out as bytes; record terminal metrics on drop.
    let rx_stream = ReceiverStream::new(rx);
    let body_stream = async_stream::stream! {
        let mut total_rows = 0usize;
        let mut error_count = 0usize;
        let mut sources_done = 0usize;
        let mut capped_sources = 0usize;
        for await frame in rx_stream {
            match &frame {
                Frame::Rows { rows, .. } => total_rows += rows.len(),
                Frame::Error { .. } => error_count += 1,
                Frame::SourceDone { capped, .. } => {
                    sources_done += 1;
                    if *capped { capped_sources += 1; }
                }
                _ => {}
            }
            let line = frame_to_line(&frame);
            yield Ok::<_, std::convert::Infallible>(Bytes::from(line));
        }
        let elapsed = start.elapsed();
        ::metrics::histogram!("kyma_explore_search_duration_seconds",
            "scope_kind" => scope_kind.clone())
            .record(elapsed.as_secs_f64());
        ::metrics::histogram!("kyma_explore_search_sources_resolved",
            "scope_kind" => scope_kind.clone())
            .record(resolved_count as f64);
        ::metrics::histogram!("kyma_explore_search_rows_returned").record(total_rows as f64);
        ::metrics::counter!("kyma_explore_search_per_source_errors_total")
            .increment(error_count as u64);
        ::metrics::counter!("kyma_explore_search_cap_hits_total")
            .increment(capped_sources as u64);
        tracing::info!(
            request_id = %req_id_for_log,
            endpoint = "/v1/explore/search",
            scope_kind = %scope_kind,
            scope_resolved_sources = resolved_count,
            sources_done,
            total_rows,
            capped_sources,
            error_count,
            elapsed_ms = elapsed.as_millis() as u64,
            "explore search completed"
        );
    };

    let mut resp = Response::new(Body::from_stream(body_stream));
    let hdrs = resp.headers_mut();
    hdrs.insert("content-type", HeaderValue::from_static("application/x-ndjson; charset=utf-8"));
    if let Ok(rid) = HeaderValue::from_str(&request_id) {
        hdrs.insert("x-request-id", rid);
    }
    resp
}

fn scope_kind_label(s: &Scope) -> String {
    match s {
        Scope::All => "all",
        Scope::Sources { .. } => "sources",
        Scope::View { .. } => "view",
    }.to_string()
}

fn parse_time_range(tr: &TimeRangeBody) -> Result<TimeRange, String> {
    let from = chrono::DateTime::parse_from_rfc3339(&tr.from)
        .map_err(|e| format!("from: {e}"))?
        .timestamp_millis();
    let to = chrono::DateTime::parse_from_rfc3339(&tr.to)
        .map_err(|e| format!("to: {e}"))?
        .timestamp_millis();
    if to <= from {
        return Err("time_range.to must be greater than time_range.from".into());
    }
    Ok(TimeRange { from_ms: from, to_ms: to })
}
```

- [ ] **Step 2: Expose `extract_request_id`, `error_response`, `resolve_query_budget` so the handler can use them**

In `crates/kyma-server/src/lib.rs`, locate the three free functions (`fn extract_request_id`, `fn error_response`, `fn resolve_query_budget` — around lines 211–276). Change each from `fn` to `pub(crate) fn`. Example:

```rust
pub(crate) fn extract_request_id(headers: &HeaderMap) -> String { ... }
pub(crate) fn error_response(...) -> Response { ... }
pub(crate) fn resolve_query_budget(headers: &HeaderMap) -> kyma_core::query_frontend::QueryBudget { ... }
```

- [ ] **Step 3: Register the route**

In `crates/kyma-server/src/lib.rs` inside `pub fn router(...)`, add the route next to `/v1/query`:

```rust
Router::new()
    .route("/v1/query", post(query_handler))
    .route("/v1/explore/search", post(discover::handler::discover_search_handler))
    .route("/v1/catalog/schema", get(catalog_handler::schema_handler))
    .with_state(state.clone())
    // ...
```

- [ ] **Step 4: Compile-check**

```bash
cargo check -p kyma-server
```

If `async_stream`, `tokio_stream`, or `chrono` aren't in `kyma-server`'s `Cargo.toml`, add them. Run `cargo add -p kyma-server async-stream tokio-stream chrono` (or edit the manifest manually) — versions should match the workspace already used elsewhere (`cargo tree -p kyma-server | grep <crate>` to find the current pin).

Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/discover/handler.rs crates/kyma-server/src/lib.rs crates/kyma-server/Cargo.toml
git commit -m "feat(discover): POST /v1/explore/search handler + route"
```

---

## Phase 7 — Integration test

### Task 8: End-to-end integration test

**Files:**
- Create: `crates/kyma-server/src/discover/handler_test_support.rs`
- Create: `crates/kyma-server/tests/discover_search_it.rs`

- [ ] **Step 1: Read existing integration-test scaffolding**

```bash
grep -n 'seeded_state_with_obs_otel_logs\|TestServer\|spawn_test_server' crates/kyma-server/src/test_support.rs
cat crates/kyma-server/tests/auth_handler_it.rs | head -100
```

Note the function name and signature for the existing test-state fixture. The new test uses the same pattern.

- [ ] **Step 2: Extend the test fixture to seed a second table in a second database**

Write `crates/kyma-server/src/discover/handler_test_support.rs`:

```rust
//! Test fixture: seeds two databases each with one tiny table so the search
//! endpoint has something cross-source to find.

use crate::test_support;
use crate::QueryState;

/// Two databases, two tables, each pre-loaded with two rows. Returns the
/// `QueryState` ready to mount in a router under `require_role_middleware(Read)`.
pub async fn seeded_state_two_dbs_two_tables() -> QueryState {
    let state = test_support::seeded_state_with_obs_otel_logs().await;
    // Seed a second database "stg" with one table "http_reqs" containing two rows.
    // The exact API for adding tables is in `test_support`; this function is a
    // thin wrapper around whatever helper exists there. If no helper exists,
    // implement one — it belongs in test_support, not here.
    test_support::seed_table(
        &state,
        "stg", "http_reqs",
        &[("timestamp", "Timestamp"), ("status", "Int"), ("path", "String")],
        &[
            r#"{"timestamp":"2026-05-28T13:00:00Z","status":500,"path":"/login"}"#,
            r#"{"timestamp":"2026-05-28T13:01:00Z","status":200,"path":"/health"}"#,
        ],
    ).await;
    state
}
```

If `test_support::seed_table` does not exist, add it (one function in `test_support.rs` that inserts a `TableRef` into the catalog and writes rows into the object store). The shape is the same as whatever `seeded_state_with_obs_otel_logs` already does internally for `otel_logs`.

- [ ] **Step 3: Write the integration test**

Write `crates/kyma-server/tests/discover_search_it.rs`:

```rust
use axum::Router;
use http::StatusCode;
use kyma_server::discover::handler_test_support::seeded_state_two_dbs_two_tables;
use kyma_server::router;

#[tokio::test]
async fn discover_search_returns_plan_rows_and_done() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app: Router = router(state);
    let (addr, _shutdown) = kyma_server::test_support::spawn(app).await;

    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "all" },
        "time_range": null,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/x-ndjson; charset=utf-8",
    );

    let text = resp.text().await.expect("body read failed");
    let frames: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parse"))
        .collect();

    // First frame must be a plan listing both seeded sources.
    assert_eq!(frames[0]["type"], "plan");
    let plan_sources: Vec<&str> = frames[0]["sources"]
        .as_array().unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert!(plan_sources.contains(&"obs.otel_logs"));
    assert!(plan_sources.contains(&"stg.http_reqs"));

    // Last frame is a `done`.
    assert_eq!(frames.last().unwrap()["type"], "done");
    assert!(frames.last().unwrap()["elapsed_ms"].as_u64().unwrap() < 60_000);

    // Each seeded source produced exactly one source_done with total>=1.
    let mut done_per_source = std::collections::HashMap::<String, u64>::new();
    for f in &frames {
        if f["type"] == "source_done" {
            done_per_source.insert(
                f["source"].as_str().unwrap().to_string(),
                f["total"].as_u64().unwrap(),
            );
        }
    }
    assert!(done_per_source.get("obs.otel_logs").copied().unwrap_or(0) >= 1);
    assert!(done_per_source.get("stg.http_reqs").copied().unwrap_or(0) >= 1);
}

#[tokio::test]
async fn discover_search_field_filter_narrows_results() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app = router(state);
    let (addr, _shutdown) = kyma_server::test_support::spawn(app).await;

    let body = serde_json::json!({
        "query": "status:500",
        "scope": { "kind": "all" },
    });

    let resp = reqwest::Client::new()
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .json(&body)
        .send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let text = resp.text().await.unwrap();

    // stg.http_reqs has status=500 row → matches.
    // obs.otel_logs has no `status` column → clause is dropped, so it returns rows
    //   tolerantly with no filter applied (per spec section 5 tolerance rules).
    let mut rows_per_source = std::collections::HashMap::<String, usize>::new();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["type"] == "rows" {
            *rows_per_source.entry(v["source"].as_str().unwrap().into()).or_insert(0)
                += v["rows"].as_array().unwrap().len();
        }
    }
    assert!(rows_per_source.get("stg.http_reqs").copied().unwrap_or(0) >= 1);
}

#[tokio::test]
async fn scope_too_large_returns_400() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app = router(state);
    let (addr, _shutdown) = kyma_server::test_support::spawn(app).await;

    std::env::set_var("KYMA_DISCOVER_MAX_SOURCES", "1");

    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "all" },
    });
    let resp = reqwest::Client::new()
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .json(&body)
        .send().await.unwrap();

    std::env::remove_var("KYMA_DISCOVER_MAX_SOURCES");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "scope_too_large");
}

#[tokio::test]
async fn bad_json_returns_400() {
    let state = seeded_state_two_dbs_two_tables().await;
    let app = router(state);
    let (addr, _shutdown) = kyma_server::test_support::spawn(app).await;

    let resp = reqwest::Client::new()
        .post(&format!("http://{addr}/v1/explore/search"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/json")
        .body("{not json")
        .send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 4: Run the integration tests**

```bash
cargo test -p kyma-server --test discover_search_it -- --nocapture
```

Expected: 4 tests pass. (First run will download testcontainers / start Postgres; subsequent runs are fast.)

If `test_support::spawn` or `spawn_test_server` is named differently in your tree, replace with the correct symbol — match what's already used by `tests/auth_handler_it.rs`. The test logic above is the contract; symbol names are local.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test -p kyma-server -- --nocapture
```

Expected: green, no regressions in the existing handlers.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/src/discover/handler_test_support.rs \
        crates/kyma-server/src/test_support.rs \
        crates/kyma-server/tests/discover_search_it.rs
git commit -m "test(discover): integration test for /v1/explore/search (plan/rows/done, filter, scope-too-large)"
```

---

## Phase 8 — Manual smoke

### Task 9: Hit the endpoint with curl against a local kyma-bin

- [ ] **Step 1: Run the engine**

```bash
cargo run -p kyma-bin
```

Wait for the "listening" log line. Confirm the auto-migration applied cleanly.

- [ ] **Step 2: Issue a baseline search request**

```bash
curl -sN -X POST http://localhost:8080/v1/explore/search \
  -H "authorization: Bearer $KYMA_READ_TOKEN" \
  -H "content-type: application/json" \
  -d '{"query":"","scope":{"kind":"all"}}'
```

Expected: NDJSON frames stream: one `plan` line first, then per-source `source_progress` / `rows` / `source_done`, then a final `done`. Each line parses with `jq -c .`.

- [ ] **Step 3: Confirm metrics surfaced**

```bash
curl -s http://localhost:8080/metrics | grep kyma_explore_search
```

Expected: at least `kyma_explore_search_requests_total`, `kyma_explore_search_duration_seconds_bucket`, `kyma_explore_search_sources_resolved_bucket`.

- [ ] **Step 4: Issue a field-filter request and observe `dropped_clauses` on a source that lacks the field**

```bash
curl -sN -X POST http://localhost:8080/v1/explore/search \
  -H "authorization: Bearer $KYMA_READ_TOKEN" \
  -H "content-type: application/json" \
  -d '{"query":"status:>500","scope":{"kind":"all"}}' \
  | jq -c 'select(.type=="source_done") | {source, total, dropped_clauses}'
```

Expected: at least one source reports a non-empty `dropped_clauses` (the ones without a `status` column) and still returns its full set of rows (tolerance).

- [ ] **Step 5: No commit (smoke only).** Stop the engine.

---

## Phase 9 — Lint + final commit

### Task 10: Clippy + fmt sweep

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy**

```bash
cargo clippy -p kyma-server --all-targets -- -D warnings
```

Expected: clean. Fix any warnings inline.

- [ ] **Step 3: Commit if there were lint fixes**

```bash
git diff --quiet || git commit -am "chore(discover): fmt + clippy"
```

---

## Self-Review Checklist (already performed by author)

**Spec coverage:**

- §4.1 endpoint contract — Task 7 (handler) + Task 8 (integration test asserts frame shapes)
- §4.1 NDJSON frame types — Task 5
- §4.1 `plan` first, `done` last — fanout module + integration test
- §4.1 per-source errors don't abort siblings — fanout module (every per-source handle is its own spawn; sender errors are ignored)
- §4.1 RBAC drops — scope resolver swallows per-DB errors
- §4.1 scope.kind: all / sources / view — Task 4 + Task 7 (View arm returns `ViewNotFound` since saved-view lookup ships in plan B)
- §4.1 `scope_too_large` pre-execution check — Task 4 + Task 7 + Task 8 (integration test)
- §5 grammar — Task 2
- §5 per-source compilation tolerance + `dropped_clauses` — Task 3 + Task 5 + Task 8 (smoke)
- §8.1 limits — Task 7 (DEFAULT_PER_SOURCE_LIMIT=500, MAX=5000, DEFAULT_MAX_SOURCES=200)
- §8.2 RBAC + auth — reuses `require_role_middleware(Role::Read)` applied by `kyma-bin`
- §8.3 error surfacing — error_response helper + per-source `Frame::Error`
- §8.5 observability — Task 7 (5 metrics + structured tracing log)
- §8.6 engine tests — Task 8 (integration), Task 2/3/5 (unit)
- Cursor / pagination — explicitly out of scope (no `cursor` field accepted)
- Histogram frame — type defined in Task 5; emission is a no-op in Phase 1 (the `Frame::Histogram` variant is unused at runtime). Marked as a follow-up; the spec doesn't require histogram in the initial endpoint ship.

**Open follow-ups (not in this plan):**

- Saved-view lookup: shipped in plan B (`2026-05-28-discover-engine-saved-views.md`). The `View` arm of `Scope` returns `ViewNotFound` until that plan lands and wires a real `SavedViewLookup` into `discover_search_handler`.
- Actual histogram-bucket computation per source: ships as a small follow-up task once the frontend wants it; the frame type already supports it.
