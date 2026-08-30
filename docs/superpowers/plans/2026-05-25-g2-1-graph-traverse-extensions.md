# Graph Layer G2.1 — graph-traverse extensions (multi-source + edge-type) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Extend the KQL `graph-traverse` operator with (1) a **multi-source seed set** — `source (a, b, c)` — and (2) an **edge-type filter** — `edge-type <value>` — that prunes per hop (inside the recursive CTE), not just on the final result.

**Architecture:** Pure KQL-parser work in `crates/pensieve-kql/src/parser.rs`. `graph-traverse` already compiles to a recursive CTE (`_gt`/`_gt_min`/`_gt_result`). Multi-source changes only the anchor row of `_gt`; edge-type adds `AND e."type" = <lit>` to both the anchor-join and the recursive step's WHERE. No lexer changes (both use the existing keyword-arg + `(...)` token patterns). Schema-agnostic, like the rest of KQL.

**Tech Stack:** Rust, `pensieve-kql`. Tests inline in `parser.rs` (`must()` + `sql.contains()` style).

**Reference (read these):** `crates/pensieve-kql/src/parser.rs` — `parse_operator` (~128), `op_graph_traverse` (~377–441), `parse_graph_preamble` (~323–375), `parse_scalar_literal` (grep it), `build_recursive_step_with_src` (~757), `quote_ident`, the test module (~868+). `state.rs` `QueryState.ctes`.

**Working dir:** worktree `…/.claude/worktrees/feature+graph-layer`. Verify: `cargo test -p pensieve-kql`, `cargo build -p pensieve-kql`.

**Backward compatibility (critical):** existing `graph-traverse source "a" from src to dst max-hops N [direction …]` must still compile to the same SQL. Both new clauses are OPTIONAL and additive.

---

## Surface (new optional syntax)

```
Edges | graph-traverse source "a"            from src to dst max-hops 3
Edges | graph-traverse source ("a","b","c")  from src to dst max-hops 3            // multi-source
Edges | graph-traverse source "a"            from src to dst max-hops 3 edge-type "CALLS"   // typed
```
`edge-type` targets the conventional `type` edge column (quoted `e."type"` to dodge reserved-word issues). It appears AFTER the preamble (after `max-hops N [direction …]`). Multi-source `source (...)` is a parenthesized comma list of scalar literals.

---

## Task 1: multi-source seed set

**Files:** `crates/pensieve-kql/src/parser.rs` (+ inline tests).

- [ ] **Step 1: failing tests** — add to the `#[cfg(test)] mod tests`:
```rust
#[test]
fn graph_traverse_multi_source_uses_values_anchor() {
    let sql = kql_to_sql(r#"e | graph-traverse source ("a","b") from src to dst max-hops 2"#).expect("parse");
    assert!(sql.to_uppercase().contains("VALUES"), "expected VALUES anchor, got: {sql}");
    assert!(sql.contains("'a'") && sql.contains("'b'"), "expected both seeds, got: {sql}");
    assert!(sql.contains("path_src"), "still a traverse, got: {sql}");
}
#[test]
fn graph_traverse_single_source_unchanged() {
    // regression: single-literal source keeps the original CAST(... AS VARCHAR) anchor
    let sql = kql_to_sql(r#"e | graph-traverse source "a" from src to dst max-hops 2"#).expect("parse");
    assert!(sql.contains("CAST('a' AS VARCHAR) AS node") || sql.contains("CAST('a' AS VARCHAR) AS NODE")
        || sql.contains("CAST('a' AS VARCHAR)"), "expected single-source CAST anchor, got: {sql}");
}
```

- [ ] **Step 2:** `cargo test -p pensieve-kql graph_traverse_multi_source` → FAIL (parse error on `(`).

- [ ] **Step 3: implement.** Add a seed-list parser and use it in `op_graph_traverse`.
  - Add a method (near `parse_scalar_literal`): 
```rust
/// Parse the `source` value: either a single scalar literal, or a
/// parenthesized comma list `( <lit>, <lit>, … )`. Returns the SQL scalar
/// literal strings (already SQL-escaped by `parse_scalar_literal`).
fn parse_source_seeds(&mut self) -> Result<Vec<String>, ParseError> {
    if self.eat(&Token::LParen) {
        let mut seeds = Vec::new();
        loop {
            seeds.push(self.parse_scalar_literal()?);
            if self.eat(&Token::Comma) { continue; }
            break;
        }
        if !self.eat(&Token::RParen) {
            return Err(ParseError("expected ')' to close source list".into()));
        }
        if seeds.is_empty() {
            return Err(ParseError("source list must have at least one value".into()));
        }
        Ok(seeds)
    } else {
        Ok(vec![self.parse_scalar_literal()?])
    }
}
```
  (Confirm `Token::LParen`/`RParen`/`Comma` and the `eat`/`parse_scalar_literal` APIs by reading the file — adapt names if slightly different.)
  - In `op_graph_traverse`, replace `let source_sql = self.parse_scalar_literal()?;` with `let seeds = self.parse_source_seeds()?;`.
  - Build the anchor SELECT from `seeds`:
```rust
let anchor = if seeds.len() == 1 {
    let s = &seeds[0];
    format!("SELECT CAST({s} AS VARCHAR) AS node, 0 AS depth, CAST({s} AS VARCHAR) AS path_src")
} else {
    let values = seeds.iter().map(|s| format!("(CAST({s} AS VARCHAR))")).collect::<Vec<_>>().join(", ");
    format!("SELECT node, 0 AS depth, node AS path_src FROM (VALUES {values}) AS _seeds(node)")
};
```
  - Use `{anchor}` in the `_gt` CTE body in place of the current inline anchor SELECT (keep the `UNION ALL SELECT {step_sql} FROM _gt t JOIN {edge_table} e ON {join_cond} WHERE t.depth < {max_hops}` part unchanged for now — edge-type adds to that WHERE in Task 2).

- [ ] **Step 4:** `cargo test -p pensieve-kql` → all pass (new + existing graph_traverse tests unchanged). `cargo build -p pensieve-kql` clean.

- [ ] **Step 5: Commit:**
```bash
git add crates/pensieve-kql/src/parser.rs
git commit -m "feat(kql): graph-traverse multi-source seed set"
```

---

## Task 2: edge-type filter (per-hop)

**Files:** `crates/pensieve-kql/src/parser.rs` (+ inline tests).

- [ ] **Step 1: failing tests:**
```rust
#[test]
fn graph_traverse_edge_type_filters_per_hop() {
    let sql = kql_to_sql(r#"e | graph-traverse source "a" from src to dst max-hops 3 edge-type "CALLS""#).expect("parse");
    assert!(sql.contains(r#"e."type" = 'CALLS'"#), "expected per-hop edge-type filter, got: {sql}");
}
#[test]
fn graph_traverse_no_edge_type_has_no_type_filter() {
    let sql = kql_to_sql(r#"e | graph-traverse source "a" from src to dst max-hops 3"#).expect("parse");
    assert!(!sql.contains(r#""type" ="#), "no edge-type clause expected, got: {sql}");
}
```

- [ ] **Step 2:** run → FAIL (parse error on `edge-type` / missing filter).

- [ ] **Step 3: implement.** After `parse_graph_preamble` returns in `op_graph_traverse`, parse an OPTIONAL `edge-type` kwarg:
```rust
let edge_type: Option<String> = if matches!(self.peek(), Some(Token::Ident(s)) if s == "edge-type") {
    self.pos += 1; // consume 'edge-type'
    Some(self.parse_scalar_literal()?)
} else {
    None
};
let type_filter = match &edge_type {
    Some(v) => format!(r#" AND e."type" = {v}"#),
    None => String::new(),
};
```
  Then inject `{type_filter}` into the recursive `_gt` body's WHERE clause, i.e. change `WHERE t.depth < {max_hops}` → `WHERE t.depth < {max_hops}{type_filter}`. (The `e` alias is the edge table in that join, so `e."type"` resolves.)
  - `edge-type` must be parsed AFTER the optional `direction` (i.e. after `parse_graph_preamble`, which already consumed direction). Confirm ordering: preamble consumes `from/to/max-hops/[direction]`; `edge-type` comes next. Good.

- [ ] **Step 4:** `cargo test -p pensieve-kql` → all pass. Also confirm multi-source + edge-type compose: optionally add
```rust
#[test]
fn graph_traverse_multi_source_and_edge_type_compose() {
    let sql = kql_to_sql(r#"e | graph-traverse source ("a","b") from src to dst max-hops 2 edge-type "X""#).expect("parse");
    assert!(sql.to_uppercase().contains("VALUES") && sql.contains(r#"e."type" = 'X'"#), "got: {sql}");
}
```
  `cargo build -p pensieve-kql` + `cargo clippy -p pensieve-kql` clean.

- [ ] **Step 5: Commit:**
```bash
git add crates/pensieve-kql/src/parser.rs
git commit -m "feat(kql): graph-traverse edge-type per-hop filter"
```

---

## Task 3: doc + verify
- [ ] **Step 1:** Update `docs/graphs.md` `graph-traverse` section to document the new optional clauses (multi-source `source (...)`, `edge-type "<t>"`), with a one-line example each. Keep it short; match the existing doc style.
- [ ] **Step 2:** `cargo test -p pensieve-kql` all pass; `cargo build -p pensieve-kql` clean.
- [ ] **Step 3: Commit:**
```bash
git add docs/graphs.md
git commit -m "docs(graphs): document graph-traverse multi-source + edge-type"
```

---

## Self-review notes
- **Backward compatible:** single-source + no-edge-type path emits byte-identical SQL to today (the regression tests pin this). Both clauses are optional and additive.
- **edge-type targets `type`** (the conventional edge-type column, quoted `e."type"`). A differently-named type column is handled today via a post-`| where`, or a future `edge-type <col> = <val>` extension.
- **No lexer changes** — `(`/`)`/`,` and hyphenated `edge-type` already tokenize.
- **Out of scope:** `make-graph` + `graph-match` (G2.2 — needs lexer `:`/`-->`, pattern parsing, join CTE, dotted-path expressions). Wiring `StoredGraphProvider` to use typed traverse (optional later optimization).
- **Schema-agnostic:** the parser doesn't validate that a `type` column exists; an invalid column surfaces as a DataFusion plan error at query time (consistent with the rest of KQL).
