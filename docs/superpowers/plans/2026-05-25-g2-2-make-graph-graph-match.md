# Graph Layer G2.2 — make-graph + graph-match Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add KQL `make-graph` + `graph-match` for fixed single-hop pattern matching that projects node/edge properties across the pattern — e.g. `Edges | make-graph caller --> callee with Services on id | graph-match (a)-[e:CALLS]->(b) project a.name, e.latency, b.name`.

**Architecture:** All in `crates/pensieve-kql`. `make-graph` records a `GraphDef` (edge src/dst cols + node table + id col) on `QueryState`; it emits no SQL. `graph-match` consumes the `GraphDef`, parses a `(a)-[e:TYPE]->(b)` pattern + a required `project` list, and pushes a non-recursive CTE that 3-way-joins the edge table to the node table twice (`a` on src, `b` on dst), with an optional `WHERE e."type" = '<TYPE>'`. To avoid touching the **shared** expression parser (which deliberately reserves dotted paths for format-v1 dynamic columns), `graph-match` uses a **local** project parser for `var.col [as alias]` items — no change to `parse_expr`/`parse_atom`.

**Tech Stack:** Rust, `pensieve-kql` (lexer + parser + state). Inline golden tests + a live execution check.

**Reference:** `crates/pensieve-kql/src/lexer.rs` (Token enum ~4-30, `-` scan ~243-258), `parser.rs` (`parse_operator` ~128, `op_graph_traverse` as the CTE-push template, `quote_ident`, tests ~868+), `state.rs` (`QueryState` ~7-34). The G2 exploration report (KQL parser map) has the details.

**Working dir:** worktree `…/.claude/worktrees/feature+graph-layer`. `cargo test -p pensieve-kql`.

---

## Surface

```
Edges
| make-graph <src_col> --> <dst_col> with <NodeTable> on <id_col>
| graph-match ( <a> ) - [ <e> [: <TYPE>] ] -> ( <b> )
    project <var>.<col> [as <alias>] [, …]
```
- v1: fixed single-hop forward pattern; `:TYPE` optional (filters on the conventional `type` edge column); `project` REQUIRED (selects which node/edge columns to return — avoids ambiguous `SELECT *` on the self-join). `var` ∈ {the pattern's a/e/b names}. `where` is deferred.

---

## Task 1: lexer tokens — `:`, `-->`, `->`

**Files:** `crates/pensieve-kql/src/lexer.rs` (+ inline tests).

- [ ] **Step 1: failing test** — add to lexer tests (find the `#[cfg(test)]` module; if none, add one):
```rust
#[test]
fn lexes_graph_pattern_punctuation() {
    let t = tokenize("(a)-[e:T]->(b)").expect("lex");
    use Token::*;
    assert_eq!(t, vec![LParen, Ident("a".into()), RParen, Minus, LBracket,
        Ident("e".into()), Colon, Ident("T".into()), RBracket, RightArrow,
        LParen, Ident("b".into()), RParen]);
}
#[test]
fn lexes_double_arrow() {
    let t = tokenize("src --> dst").expect("lex");
    assert_eq!(t, vec![Token::Ident("src".into()), Token::Arrow, Token::Ident("dst".into())]);
}
#[test]
fn minus_still_lexes_alone() {
    assert_eq!(tokenize("a - b").expect("lex"), vec![Token::Ident("a".into()), Token::Minus, Token::Ident("b".into())]);
}
```
(Confirm `tokenize` is the lexer entry + how `Token` is imported in the test module.)

- [ ] **Step 2:** `cargo test -p pensieve-kql lexes_graph` → FAIL (Colon/Arrow/RightArrow don't exist).

- [ ] **Step 3: implement.** In the `Token` enum add `Colon, Arrow, RightArrow`. In the scanner:
  - `':'` → push `Token::Colon`.
  - In the `'-'` branch (the standalone-dash case, AFTER the ident-with-hyphen logic): peek ahead —
    - if the next two chars are `->` (i.e. `-->`) consume both and push `Token::Arrow`;
    - else if the next char is `>` (i.e. `->`) consume it and push `Token::RightArrow`;
    - else push `Token::Minus` (unchanged).
  READ the existing `-` handling (~lexer.rs:243-258) and integrate so hyphenated idents (`graph-match`, `max-hops`) are UNAFFECTED — those are consumed by the identifier scanner before reaching the standalone-dash branch. Only standalone `-` (after `)`, `]`, whitespace, etc.) reaches this branch.

- [ ] **Step 4:** `cargo test -p pensieve-kql` → all pass (new lexer tests + ALL existing parser/lexer tests unchanged — the existing `graph-traverse`/arithmetic tests must still pass; `-` in `a - b` and negative numbers still lex as `Minus`). `cargo build -p pensieve-kql` clean.

- [ ] **Step 5: Commit:**
```bash
git add crates/pensieve-kql/src/lexer.rs
git commit -m "feat(kql/lexer): Colon, Arrow (-->), RightArrow (->) tokens"
```

---

## Task 2: `QueryState.graph_def` + `make-graph`

**Files:** `crates/pensieve-kql/src/state.rs`, `crates/pensieve-kql/src/parser.rs`.

- [ ] **Step 1:** in `state.rs`, add a `GraphDef` struct + field on `QueryState`:
```rust
#[derive(Debug, Clone)]
pub struct GraphDef {
    pub edge_table: String,
    pub src_col: String,
    pub dst_col: String,
    pub node_table: String,
    pub id_col: String,
}
```
Add `pub graph_def: Option<GraphDef>,` to `QueryState` (it derives `Default`, so `Option` defaults to `None`). `make-graph`/`graph-match` are the only readers/writers; `to_sql` ignores it.

- [ ] **Step 2: failing test** (parser tests):
```rust
#[test]
fn make_graph_then_match_projects_join() {
    let sql = kql_to_sql(
        r#"edges | make-graph caller --> callee with services on id | graph-match (a)-[e:CALLS]->(b) project a.name, e.latency, b.name"#
    ).expect("parse");
    let u = sql.to_uppercase();
    assert!(u.contains("JOIN SERVICES A ON") || sql.contains("join services a on"), "expected node join for a, got: {sql}");
    assert!(sql.contains("e.caller = a.id"), "a joins on src=id, got: {sql}");
    assert!(sql.contains("e.callee = b.id"), "b joins on dst=id, got: {sql}");
    assert!(sql.contains(r#"e."type" = 'CALLS'"#), "edge-type filter, got: {sql}");
    assert!(sql.contains(r#"a."name""#) && sql.contains(r#"b."name""#) && sql.contains(r#"e."latency""#), "projected cols, got: {sql}");
}
#[test]
fn graph_match_without_make_graph_errors() {
    let err = kql_to_sql(r#"edges | graph-match (a)-[e]->(b) project a.x"#);
    assert!(err.is_err(), "graph-match requires a preceding make-graph");
}
```

- [ ] **Step 3:** run → FAIL.

- [ ] **Step 4: implement `op_make_graph`** + register it. In `parse_operator` add arms: `"make-graph" => self.op_make_graph(),` and `"graph-match" => self.op_graph_match(),`.
```rust
/// `make-graph <src> --> <dst> with <NodeTable> on <id>` — records the graph
/// definition for a following `graph-match`. Emits no SQL itself.
fn op_make_graph(&mut self) -> Result<(), ParseError> {
    let src_col = self.expect_ident()?;          // helper: bump and require Ident
    if !self.eat(&Token::Arrow) {
        return Err(ParseError("make-graph: expected `-->` between src and dst".into()));
    }
    let dst_col = self.expect_ident()?;
    self.expect_ident_eq("with")?;
    let node_table = self.expect_ident()?;
    self.expect_ident_eq("on")?;
    let id_col = self.expect_ident()?;
    self.state.graph_def = Some(crate::state::GraphDef {
        edge_table: self.state.table.clone(),
        src_col, dst_col, node_table, id_col,
    });
    Ok(())
}
```
Add an `expect_ident` helper if one doesn't exist (`bump()` + require `Token::Ident`, returning the raw string — do NOT `quote_ident` here; these are column/table names quoted at use-site). Confirm whether a similar helper already exists and reuse it.

- [ ] **Step 5: implement `op_graph_match`:**
```rust
/// `graph-match (a)-[e[:TYPE]]->(b) project <var>.<col> [as <alias>], …`
fn op_graph_match(&mut self) -> Result<(), ParseError> {
    let def = self.state.graph_def.clone()
        .ok_or_else(|| ParseError("graph-match requires a preceding make-graph".into()))?;

    // pattern: ( a ) - [ e [: TYPE] ] -> ( b )
    self.expect(&Token::LParen)?; let a = self.expect_ident()?; self.expect(&Token::RParen)?;
    self.expect(&Token::Minus)?;
    self.expect(&Token::LBracket)?; let e = self.expect_ident()?;
    let edge_type = if self.eat(&Token::Colon) { Some(self.expect_ident()?) } else { None };
    self.expect(&Token::RBracket)?;
    self.expect(&Token::RightArrow)?;
    self.expect(&Token::LParen)?; let b = self.expect_ident()?; self.expect(&Token::RParen)?;

    // required: project <var>.<col> [as <alias>], …
    self.expect_ident_eq("project")?;
    let mut select_items: Vec<String> = Vec::new();
    loop {
        let var = self.expect_ident()?;
        self.expect(&Token::Dot)?;
        let col = self.expect_ident()?;
        // map the pattern variable to its SQL alias
        let alias_tbl = if var == a { "a" } else if var == e { "e" } else if var == b { "b" } else {
            return Err(ParseError(format!("graph-match project: unknown variable `{var}` (expected {a}, {e}, or {b})")));
        };
        let out_alias = if self.eat_ident_eq("as") { self.expect_ident()? }
            else { format!("{var}_{col}") };
        select_items.push(format!(r#"{alias_tbl}.{} AS {}"#, quote_ident(&col), quote_ident(&out_alias)));
        if !self.eat(&Token::Comma) { break; }
    }
    if select_items.is_empty() {
        return Err(ParseError("graph-match: project must list at least one column".into()));
    }

    let mut where_clause = String::new();
    if let Some(t) = &edge_type {
        where_clause = format!(r#" WHERE e."type" = '{}'"#, t.replace('\'', "''"));
    }
    let body = format!(
        "SELECT {sel} FROM {edges} e \
         JOIN {nodes} a ON e.{src} = a.{id} \
         JOIN {nodes} b ON e.{dst} = b.{id}{where_clause}",
        sel = select_items.join(", "),
        edges = def.edge_table, nodes = def.node_table,
        src = quote_ident(&def.src_col), dst = quote_ident(&def.dst_col), id = quote_ident(&def.id_col),
    );
    self.state.ctes.push(("_gm_result".to_string(), body, false));
    self.state.table = "_gm_result".to_string();
    Ok(())
}
```
Add helpers if missing: `expect(&Token)` (bump + assert equals), `expect_ident()` (bump + require Ident → String), `eat_ident_eq(&str)` (peek for a specific ident, consume if matched, return bool). Check the file for existing equivalents (`eat`, `expect_ident_eq`) and reuse/extend.

NOTE on `:TYPE` as an ident: a bare type like `CALLS` lexes as `Ident`. If a type needs quoting, `[e:"some type"]` would lex `:` then `Str` — for v1 support only the `Ident` type token; a `Str` type can be a later enhancement (report if you add it).

- [ ] **Step 6:** `cargo test -p pensieve-kql` → all pass (new + existing). `cargo build` + `cargo clippy -p pensieve-kql` clean.

- [ ] **Step 7: Commit:**
```bash
git add crates/pensieve-kql/src/state.rs crates/pensieve-kql/src/parser.rs
git commit -m "feat(kql): make-graph + graph-match (single-hop pattern + project)"
```

---

## Task 3: docs + live execution verify

- [ ] **Step 1: docs** — add a `make-graph` / `graph-match` subsection to `docs/graphs.md` (after the graph-traverse section) with the surface + a one example, matching the existing doc style.
- [ ] **Step 2: live verify (controller may run this; include the steps).** Rebuild + restart the engine, then run a graph-match against the seeded `kg` graph (`graphdemo`): we have `kg_nodes(id,labels,name)` + `kg_edges(src,dst,type)`. Run via `/v1/query` (KQL, `X-Database: graphdemo`):
```
kg_edges | make-graph src --> dst with kg_nodes on id | graph-match (a)-[e:CALLS]->(b) project a.name as src_name, b.name as dst_name, e.type
```
Expected: one row — `src_name=payments, dst_name=auth, type=CALLS` (the n1→n2 CALLS edge; payments→auth). This proves the join CTE + edge-type filter + property projection execute against DataFusion. Report the actual rows.
- [ ] **Step 3:** `cargo test -p pensieve-kql` all pass; build clean. Commit:
```bash
git add docs/graphs.md
git commit -m "docs(graphs): document make-graph + graph-match"
```

---

## Self-review notes
- **No shared-expr-parser change:** `graph-match` parses its own `project` items (`var.col [as alias]`), so the codebase's reserved dotted-path-for-dynamic-columns behavior is untouched. `where` on matched vars is deferred (would want the shared parser or a richer local one).
- **`project` required** to avoid ambiguous `SELECT *` on the node self-join (a/b both from the node table). Output column collisions are avoided by the `AS` aliasing (auto `var_col` when no explicit alias).
- **`:TYPE` filters the conventional `type` column** (quoted `e."type"`), consistent with G2.1's `edge-type`.
- **make-graph emits no SQL** — it only sets `graph_def`. A `make-graph` with no following `graph-match` renders `SELECT * FROM <edge_table>` (harmless). `graph-match` without `make-graph` errors.
- **v1 limits (deferred):** fixed single-hop forward pattern only (no var-length — use graph-traverse), no `where` clause, `:TYPE` must be a bare identifier. Backward arrows / multi-hop patterns are future work.
- **Backward compat:** lexer change only upgrades standalone `-` to Arrow/RightArrow when immediately followed by `->`/`>`; `Minus` (subtraction, negatives, hyphenated idents) is unaffected — pinned by `minus_still_lexes_alone` + the existing arithmetic tests.
