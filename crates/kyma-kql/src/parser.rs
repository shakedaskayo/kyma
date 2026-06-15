//! KQL parser + KQL→SQL translator.
//!
//! Recursive-descent. No AST — we stream-parse operators and directly
//! populate a [`QueryState`], then ask it to emit SQL. Trades a tiny loss
//! of compositionality for dramatically less code.

use std::collections::HashMap;

use crate::lexer::{tokenize, Token};
use crate::state::QueryState;

/// Table → ordered column names. Used to compute the outer-by-name column
/// superset for `union`. Insertion order of the `Vec<String>` is the table's
/// schema order and is preserved in the union projection.
pub type SchemaMap = HashMap<String, Vec<String>>;

#[derive(Debug, Clone)]
pub struct ParseError(pub String);
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ParseError {}

impl From<crate::lexer::LexError> for ParseError {
    fn from(e: crate::lexer::LexError) -> Self {
        ParseError(e.0)
    }
}

/// Public entry point. Compiles a KQL pipeline to SQL with no schema context.
///
/// Sufficient for every non-`union` query. The `union` operator needs the
/// column names of each operand table to compute the outer-by-name superset,
/// so a schema-less `union` errors clearly (mentioning "schema"); use
/// [`kql_to_sql_with_schemas`] for that.
pub fn kql_to_sql(src: &str) -> Result<String, ParseError> {
    kql_to_sql_with_schemas(src, &HashMap::new())
}

/// Schema-aware entry point. Identical to [`kql_to_sql`] for every query
/// except `union`, where `schemas` supplies the per-table column lists used
/// to compute the outer-by-name column superset and null-fill the branches.
///
/// Non-`union` queries never touch `schemas`, so they produce byte-identical
/// SQL through either entry point.
pub fn kql_to_sql_with_schemas(src: &str, schemas: &SchemaMap) -> Result<String, ParseError> {
    let toks = tokenize(src)?;
    let mut p = Parser::new(toks, schemas);
    p.parse_query()?;
    Ok(p.state.to_sql())
}

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

struct Parser<'s> {
    toks: Vec<Token>,
    pos: usize,
    state: QueryState,
    /// Borrowed table → column schema map for `union` superset computation.
    /// Empty for the schema-less [`kql_to_sql`] path.
    schemas: &'s SchemaMap,
}

impl<'s> Parser<'s> {
    fn new(toks: Vec<Token>, schemas: &'s SchemaMap) -> Self {
        Self {
            toks,
            pos: 0,
            state: QueryState::default(),
            schemas,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    fn bump(&mut self) -> Result<Token, ParseError> {
        let idx = self.pos;
        let t = self
            .toks
            .get(idx)
            .cloned()
            .ok_or_else(|| ParseError("unexpected end of query".into()))?;
        self.pos += 1;
        Ok(t)
    }
    fn eat(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, expected: &Token, what: &str) -> Result<(), ParseError> {
        if self.eat(expected) {
            Ok(())
        } else {
            Err(ParseError(format!(
                "expected {what}, got {:?}",
                self.peek()
            )))
        }
    }
    fn expect_ident_eq(&mut self, lit: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(Token::Ident(s)) if s == lit => {
                self.pos += 1;
                Ok(())
            }
            other => Err(ParseError(format!("expected `{lit}`, got {other:?}"))),
        }
    }

    /// Consume the next token and require it to be an identifier. Returns the
    /// raw identifier string (not `quote_ident`-d — quote at the use-site).
    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.bump()? {
            Token::Ident(s) => Ok(s),
            other => Err(ParseError(format!("expected identifier, got {other:?}"))),
        }
    }

    /// If the next token is `Ident(kw)`, consume it and return `true`;
    /// otherwise leave position unchanged and return `false`.
    fn eat_ident_eq(&mut self, kw: &str) -> bool {
        match self.peek() {
            Some(Token::Ident(s)) if s == kw => {
                self.pos += 1;
                true
            }
            _ => false,
        }
    }

    // -----------------------------------------------------------------
    // Top-level
    // -----------------------------------------------------------------
    fn parse_query(&mut self) -> Result<(), ParseError> {
        // `union` is only valid as the leading construct. It builds the
        // initial QueryState itself (a `_u` CTE over the per-branch SELECTs)
        // and leaves the active table set to `_u` so trailing pipe ops below
        // scope the union result.
        if matches!(self.peek(), Some(Token::Ident(s)) if s == "union") {
            self.pos += 1;
            self.parse_union()?;
        } else {
            // Table name is the first identifier.
            let table = match self.bump()? {
                Token::Ident(s) => s,
                other => return Err(ParseError(format!("expected table name, got {other:?}"))),
            };
            self.state = QueryState::new(table);
        }

        while self.eat(&Token::Pipe) {
            self.parse_operator()?;
        }
        if self.pos < self.toks.len() {
            return Err(ParseError(format!(
                "unexpected trailing tokens: {:?}",
                &self.toks[self.pos..]
            )));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // union — leading-only, outer-by-name (ADX default) + withsource
    // -----------------------------------------------------------------
    //
    // Syntax: `union [withsource=Ident] operand (, operand)* [| ops...]`
    // where `operand` is a bare table ident or a parenthesized sub-pipeline
    // `( <full kql pipeline> )`.
    //
    // Lowering (outer-by-name): compute the column superset across operand
    // ROOT tables (first-seen order); each branch SELECTs `col` when it owns
    // that column else `NULL AS col`, plus `'table' AS <srcCol>` when
    // `withsource` is given (the source column is projected FIRST so it is
    // part of the result schema and trailing ops can reference it). The
    // branches are combined with `UNION ALL` inside a `_u` CTE and trailing
    // pipe ops scope `_u` via the normal QueryState machinery.
    //
    // v1 limitations (documented, accepted): positional union is rejected
    // (DataFusion 44 only supports positional UNION ALL, so we emit explicit
    // per-branch projections); a branch's output schema is taken to be its
    // ROOT table's schema, so a `project` inside a parenthesized branch does
    // NOT reshape the null-fill superset (the engine will surface a column
    // mismatch if the branch truly diverges); no `kind=`, no wildcards.
    fn parse_union(&mut self) -> Result<(), ParseError> {
        // Optional `withsource=Ident`.
        let with_source: Option<String> = if self.eat_ident_eq("withsource") {
            self.expect(&Token::Assign, "`=` after withsource")?;
            Some(self.expect_ident()?)
        } else {
            None
        };

        // Parse comma-separated operands until a pipe or EOF.
        let mut operands: Vec<UnionOperand> = Vec::new();
        loop {
            operands.push(self.parse_union_operand()?);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        if operands.is_empty() {
            return Err(ParseError("union requires at least one operand".into()));
        }

        // Outer-by-name column superset: each operand's ROOT-table columns,
        // in first-seen order across operands, deduped.
        let mut superset: Vec<String> = Vec::new();
        for op in &operands {
            let cols = self.schemas.get(&op.root_table).ok_or_else(|| {
                if self.schemas.is_empty() {
                    ParseError(format!(
                        "union requires schema context: no schema available for table `{}`",
                        op.root_table
                    ))
                } else {
                    ParseError(format!(
                        "union: unknown table `{}` (not in schema)",
                        op.root_table
                    ))
                }
            })?;
            for c in cols {
                if !superset.contains(c) {
                    superset.push(c.clone());
                }
            }
        }

        // I1: withsource column must not collide with any superset column.
        if let Some(src_col) = &with_source {
            if superset.contains(src_col) {
                return Err(ParseError(format!(
                    "withsource column `{src_col}` collides with a column of the union operands"
                )));
            }
        }

        // Build one branch SELECT per operand projecting the superset with
        // null-fill, plus the optional source column FIRST.
        let mut branches: Vec<String> = Vec::with_capacity(operands.len());
        for (i, op) in operands.iter().enumerate() {
            let branch_cols: std::collections::HashSet<&String> = self
                .schemas
                .get(&op.root_table)
                .map(|v| v.iter().collect())
                .unwrap_or_default();

            let mut proj: Vec<String> = Vec::new();
            if let Some(src_col) = &with_source {
                // SQL string literal for the table name, then `AS srcCol`.
                proj.push(format!(
                    "'{}' AS {}",
                    op.root_table.replace('\'', "''"),
                    quote_ident(src_col)
                ));
            }
            for col in &superset {
                if branch_cols.contains(col) {
                    proj.push(quote_ident(col));
                } else {
                    proj.push(format!("NULL AS {}", quote_ident(col)));
                }
            }
            let projection = proj.join(", ");

            // A bare table reads directly; a parenthesized sub-pipeline wraps
            // its compiled SQL so the superset projection runs over the
            // branch's output. The branch output schema is its root table's
            // schema (v1 rule documented above).
            let branch = match &op.from {
                UnionFrom::Table(t) => format!("SELECT {projection} FROM {t}"),
                UnionFrom::SubQuery(sub_sql) => {
                    format!("SELECT {projection} FROM ({sub_sql}) _b{i}")
                }
            };
            branches.push(branch);
        }

        // Assemble: `WITH _u AS (b0 UNION ALL b1 ...) SELECT ... FROM _u`.
        // Trailing pipe ops accumulate onto this QueryState (table = `_u`).
        let union_body = branches.join(" UNION ALL ");
        self.state = QueryState::new("_u");
        self.state
            .ctes
            .push(("_u".to_string(), union_body, false));
        Ok(())
    }

    /// Parse one union operand: a bare table ident or a parenthesized
    /// sub-pipeline. Returns the operand's compiled FROM-source and the root
    /// table that determines its schema for the superset computation.
    fn parse_union_operand(&mut self) -> Result<UnionOperand, ParseError> {
        if self.eat(&Token::LParen) {
            // Slice out the sub-pipeline tokens up to the matching `)`,
            // respecting nesting, then compile them with a fresh sub-parser
            // that shares the same schema map (recursion via parse_query).
            let start = self.pos;
            let mut depth = 1usize;
            while depth > 0 {
                match self.peek() {
                    Some(Token::LParen) => depth += 1,
                    Some(Token::RParen) => depth -= 1,
                    None => {
                        return Err(ParseError(
                            "unterminated `(` in union operand".into(),
                        ))
                    }
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                self.pos += 1;
            }
            let inner_toks = self.toks[start..self.pos].to_vec();
            // Consume the closing `)`.
            self.expect(&Token::RParen, "`)` to close union operand")?;
            if inner_toks.is_empty() {
                return Err(ParseError("empty `()` union operand".into()));
            }

            // C1: Detect nested union before recursing — the sub-pipeline
            // starts with the `union` keyword if its first token is that ident.
            if matches!(inner_toks.first(), Some(Token::Ident(s)) if s == "union") {
                return Err(ParseError("nested union is not supported".into()));
            }

            // Fix 4: Reject schema-reshaping operators in branch pipelines at
            // parse time. Inspect token pairs: any Pipe immediately followed
            // by a reshaping operator keyword is rejected.
            const RESHAPING_OPS: &[&str] = &[
                "project",
                "project-away",
                "summarize",
                "count",
                "distinct",
                "extend",
                "graph-traverse",
                "graph-shortest-path",
                "make-graph",
                "graph-match",
            ];
            for window in inner_toks.windows(2) {
                if let [Token::Pipe, Token::Ident(op)] = window {
                    if RESHAPING_OPS.contains(&op.as_str()) {
                        return Err(ParseError(format!(
                            "union branch reshapes its schema (`{op}`); v1 union branches may only filter/sort/take"
                        )));
                    }
                }
            }

            let mut sub = Parser::new(inner_toks, self.schemas);
            sub.parse_query()?;
            let root_table = sub.state.root_table().to_string();
            Ok(UnionOperand {
                root_table,
                from: UnionFrom::SubQuery(sub.state.to_sql()),
            })
        } else {
            let table = self.expect_ident()?;
            Ok(UnionOperand {
                root_table: table.clone(),
                from: UnionFrom::Table(table),
            })
        }
    }

    fn parse_operator(&mut self) -> Result<(), ParseError> {
        let op = match self.bump()? {
            Token::Ident(s) => s,
            other => return Err(ParseError(format!("expected operator, got {other:?}"))),
        };
        match op.as_str() {
            "where" => self.op_where(),
            "project" => self.op_project(false),
            "project-away" => self.op_project(true),
            "extend" => self.op_extend(),
            "summarize" => self.op_summarize(),
            "take" | "limit" => self.op_take(),
            "sort" | "order" => self.op_sort(),
            "top" => self.op_top(),
            "count" => self.op_count(),
            "distinct" => self.op_distinct(),
            "graph-traverse" => self.op_graph_traverse(),
            "graph-var-length" => self.op_graph_var_length(),
            "graph-var-match" => self.op_graph_var_match(),
            "graph-shortest-path" => self.op_graph_shortest_path(),
            "make-graph" => self.op_make_graph(),
            "graph-match" => self.op_graph_match(),
            other => Err(ParseError(format!("unsupported operator: `{other}`"))),
        }
    }

    // -----------------------------------------------------------------
    // Operator handlers
    // -----------------------------------------------------------------
    fn op_where(&mut self) -> Result<(), ParseError> {
        let e = self.parse_expr()?;
        self.state.where_clauses.push(e);
        Ok(())
    }

    fn op_project(&mut self, away: bool) -> Result<(), ParseError> {
        let mut cols = Vec::new();
        loop {
            match self.bump()? {
                Token::Ident(s) => cols.push(quote_ident(&s)),
                other => {
                    return Err(ParseError(format!(
                        "expected column name after project, got {other:?}"
                    )))
                }
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        if away {
            self.state.exclude.extend(cols);
        } else {
            self.state.select = cols;
        }
        Ok(())
    }

    fn op_extend(&mut self) -> Result<(), ParseError> {
        loop {
            let name = match self.bump()? {
                Token::Ident(s) => s,
                other => {
                    return Err(ParseError(format!(
                        "expected name in extend, got {other:?}"
                    )))
                }
            };
            self.expect(&Token::Assign, "`=`")?;
            let expr = self.parse_expr()?;
            self.state.extend.push((quote_ident(&name), expr));
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        Ok(())
    }

    fn op_summarize(&mut self) -> Result<(), ParseError> {
        // summarize AGG_LIST [by GROUP_LIST]
        loop {
            let alias_opt = self.try_alias()?;
            let agg_expr = self.parse_expr()?;
            let item = match alias_opt {
                Some(a) => format!("({agg_expr}) AS {a}"),
                None => format!("({agg_expr}) AS {}", implicit_agg_alias(&agg_expr)),
            };
            self.state.aggregates.push(item);
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        // Optional `by col1, col2, ...`
        if let Some(Token::Ident(s)) = self.peek() {
            if s == "by" {
                self.pos += 1;
                loop {
                    let key = self.parse_expr()?;
                    self.state.group_by.push(key);
                    if !self.eat(&Token::Comma) {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn op_take(&mut self) -> Result<(), ParseError> {
        match self.bump()? {
            Token::Int(n) if n >= 0 => {
                self.state.limit = Some(n as u64);
                Ok(())
            }
            other => Err(ParseError(format!("`take` expects integer, got {other:?}"))),
        }
    }

    fn op_sort(&mut self) -> Result<(), ParseError> {
        // sort by col1 [asc|desc], col2 [asc|desc], ...
        self.expect_ident_eq("by")?;
        loop {
            let col_expr = self.parse_expr()?;
            // optional asc/desc
            let desc = if let Some(Token::Ident(s)) = self.peek() {
                match s.as_str() {
                    "asc" => {
                        self.pos += 1;
                        false
                    }
                    "desc" => {
                        self.pos += 1;
                        true
                    }
                    _ => true, // KQL defaults to desc
                }
            } else {
                true
            };
            self.state.order_by.push((col_expr, desc));
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        Ok(())
    }

    fn op_top(&mut self) -> Result<(), ParseError> {
        let n = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => return Err(ParseError(format!("`top` expects integer, got {other:?}"))),
        };
        self.expect_ident_eq("by")?;
        let col_expr = self.parse_expr()?;
        let desc = if let Some(Token::Ident(s)) = self.peek() {
            match s.as_str() {
                "asc" => {
                    self.pos += 1;
                    false
                }
                "desc" => {
                    self.pos += 1;
                    true
                }
                _ => true,
            }
        } else {
            true
        };
        self.state.order_by.push((col_expr, desc));
        self.state.limit = Some(n);
        Ok(())
    }

    fn op_count(&mut self) -> Result<(), ParseError> {
        // Quote to preserve the ADX-style `Count` capitalization in the
        // response — DataFusion lowercases unquoted aliases otherwise.
        self.state
            .aggregates
            .push(r#"count(*) AS "Count""#.to_string());
        Ok(())
    }

    fn op_distinct(&mut self) -> Result<(), ParseError> {
        self.state.distinct = true;
        let mut cols = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Ident(s)) => {
                    cols.push(quote_ident(s));
                    self.pos += 1;
                }
                _ => break,
            }
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        if !cols.is_empty() {
            self.state.select = cols;
        }
        Ok(())
    }

    /// Parse the shared graph-op preamble:
    ///   from <src-col> to <dst-col> max-hops <N> [direction forward|backward|both]
    /// Returns (src_col, dst_col, max_hops, direction).
    fn parse_graph_preamble(
        &mut self,
    ) -> Result<(String, String, u64, GraphDirection), ParseError> {
        self.expect_ident_eq("from")?;
        let src_col = match self.bump()? {
            Token::Ident(s) => quote_ident(&s),
            other => {
                return Err(ParseError(format!(
                    "expected `from` column name, got {other:?}"
                )))
            }
        };
        self.expect_ident_eq("to")?;
        let dst_col = match self.bump()? {
            Token::Ident(s) => quote_ident(&s),
            other => {
                return Err(ParseError(format!(
                    "expected `to` column name, got {other:?}"
                )))
            }
        };
        self.expect_ident_eq("max-hops")?;
        let max_hops = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => {
                return Err(ParseError(format!(
                    "expected positive integer after `max-hops`, got {other:?}"
                )))
            }
        };
        let direction = if matches!(self.peek(), Some(Token::Ident(s)) if s == "direction") {
            self.pos += 1;
            match self.bump()? {
                Token::Ident(s) => match s.as_str() {
                    "forward" => GraphDirection::Forward,
                    "backward" => GraphDirection::Backward,
                    "both" => GraphDirection::Both,
                    other => return Err(ParseError(format!("unknown direction: {other}"))),
                },
                other => {
                    return Err(ParseError(format!(
                        "expected direction keyword, got {other:?}"
                    )))
                }
            }
        } else {
            GraphDirection::Forward
        };
        Ok((src_col, dst_col, max_hops, direction))
    }

    /// `graph-traverse source <value> from <src> to <dst> max-hops N [direction ...]`
    ///
    /// Emits three CTEs:
    ///  - `_gt(node, depth, path_src)` — recursive frontier with the
    ///    immediate predecessor node (`path_src`) for the join-back.
    ///  - `_gt_min` — per-node minimum depth (deduplication).
    ///  - `_gt_result` — joins `_gt_min` back to the edge table to project
    ///    the full originating edge row alongside `dst` and `depth`.
    ///    Strictly additive: callers reading only `dst`/`depth` still work.
    fn op_graph_traverse(&mut self) -> Result<(), ParseError> {
        self.expect_ident_eq("source")?;
        let seeds = self.parse_source_seeds()?;
        let (src_col, dst_col, max_hops, direction) = self.parse_graph_preamble()?;

        // Optional edge-type filter: `edge-type "<value>"` — prunes per hop.
        let edge_type: Option<String> =
            if matches!(self.peek(), Some(Token::Ident(s)) if s == "edge-type") {
                self.pos += 1; // consume 'edge-type'
                Some(self.parse_scalar_literal()?)
            } else {
                None
            };
        let type_filter = match &edge_type {
            Some(v) => format!(r#" AND e."type" = {v}"#),
            None => String::new(),
        };

        let edge_table = self.state.table.clone();

        // Recursive step: next-hop node + depth+1 + the edge's src (path_src)
        // so we can join back to the edge table in the result CTE.
        let step_sql = build_recursive_step_with_src(&edge_table, &src_col, &dst_col, direction);

        // Anchor: the seed node(s) have depth=0 and no incoming edge, so path_src
        // is the node itself (a self-reference; won't match any real edge in
        // the result join, which is correct — the seed has no inbound edge).
        let anchor = if seeds.len() == 1 {
            let s = &seeds[0];
            format!(
                "SELECT CAST({s} AS VARCHAR) AS node, 0 AS depth, \
                        CAST({s} AS VARCHAR) AS path_src"
            )
        } else {
            let values = seeds
                .iter()
                .map(|s| format!("(CAST({s} AS VARCHAR))"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SELECT node, 0 AS depth, node AS path_src \
                 FROM (VALUES {values}) AS _seeds(node)"
            )
        };
        let body = format!(
            "{anchor} \
             UNION ALL \
             SELECT {step_sql} \
             FROM _gt t \
             JOIN {edge_table} e ON {join_cond} \
             WHERE t.depth < {max_hops}{type_filter}",
            join_cond = match direction {
                GraphDirection::Forward => format!("e.{src_col} = t.node"),
                GraphDirection::Backward => format!("e.{dst_col} = t.node"),
                GraphDirection::Both => format!("(e.{src_col} = t.node OR e.{dst_col} = t.node)"),
            },
        );
        self.state
            .ctes
            .push(("_gt(node, depth, path_src)".to_string(), body, true));

        // Deduplication CTE: keep only the min-depth row per node.
        // DataFusion requires grouping before joining back to a non-recursive CTE.
        let min_body =
            "SELECT node, path_src, MIN(depth) AS depth FROM _gt GROUP BY node, path_src";
        self.state
            .ctes
            .push(("_gt_min".to_string(), min_body.to_string(), false));

        // Result CTE: join back to the edges table to project full edge columns
        // alongside the dst and depth. Seed row (depth=0, path_src=node) is
        // excluded from the join because the seed has no inbound edge.
        // Strictly additive: the columns {dst_col, depth} that callers relied
        // on before are still present — edge columns are additional.
        let result_body = format!(
            "SELECT e.*, m.depth \
             FROM _gt_min m \
             JOIN {edge_table} e ON e.{src_col} = m.path_src AND e.{dst_col} = m.node"
        );
        self.state
            .ctes
            .push(("_gt_result".to_string(), result_body, false));
        self.state.table = "_gt_result".to_string();
        Ok(())
    }

    /// `graph-var-length source <seed[s]> from <src> to <dst> min-hops M max-hops N [direction forward|backward|both] [edge-type "T"]`
    ///
    /// Variable-length reachability that PRESERVES THE ORIGIN endpoint. Unlike
    /// `graph-traverse` (whose result is keyed only by the reached node), this
    /// carries the originating seed through the recursion, so the result is the
    /// set of `(src, dst)` endpoint PAIRS where `dst` is reachable from `src` at
    /// a shortest distance `d` with `min-hops ≤ d ≤ max-hops`. That pair binding
    /// is exactly what a Cypher `MATCH (a)-[*M..N]->(b) RETURN a.…, b.…`
    /// lowering needs (graph-traverse can't supply it — it drops the origin).
    ///
    /// Semantics match the CSR `var_length` primitive + the petgraph oracle:
    /// each reached node is reported once at its SHORTEST depth, then filtered
    /// to `[min,max]`. So the lowering mirrors `graph-traverse`'s three CTEs:
    ///  - `_vl(origin, node, depth)` — recursive walk enumeration bounded by
    ///    `depth < max-hops` (terminates on cycles: depth strictly increases).
    ///  - `_vl_min` — `MIN(depth)` per `(origin, node)` = the shortest distance.
    ///  - `_vl_result(src, dst, depth)` — shortest-distance pairs filtered to
    ///    `min-hops ≤ depth ≤ max-hops`. The active table.
    ///
    /// Taking `MIN` over ALL walks up to `max` and filtering by `min` AFTER —
    /// rather than filtering walks by `[min,max]` first — is what makes it agree
    /// with the oracle: a node whose shortest path is below `min` is excluded
    /// even if a longer walk reaches it inside the window.
    fn op_graph_var_length(&mut self) -> Result<(), ParseError> {
        self.expect_ident_eq("source")?;
        let seeds = self.parse_source_seeds()?;
        self.expect_ident_eq("from")?;
        let src_col = match self.bump()? {
            Token::Ident(s) => quote_ident(&s),
            other => {
                return Err(ParseError(format!(
                    "expected `from` column name, got {other:?}"
                )))
            }
        };
        self.expect_ident_eq("to")?;
        let dst_col = match self.bump()? {
            Token::Ident(s) => quote_ident(&s),
            other => {
                return Err(ParseError(format!("expected `to` column name, got {other:?}")))
            }
        };
        self.expect_ident_eq("min-hops")?;
        let min_hops = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => {
                return Err(ParseError(format!(
                    "expected non-negative integer after `min-hops`, got {other:?}"
                )))
            }
        };
        self.expect_ident_eq("max-hops")?;
        let max_hops = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => {
                return Err(ParseError(format!(
                    "expected non-negative integer after `max-hops`, got {other:?}"
                )))
            }
        };
        if max_hops == 0 {
            return Err(ParseError("graph-var-length: max-hops must be ≥ 1".into()));
        }
        if min_hops > max_hops {
            return Err(ParseError(format!(
                "graph-var-length: min-hops ({min_hops}) must be ≤ max-hops ({max_hops})"
            )));
        }
        let direction = if matches!(self.peek(), Some(Token::Ident(s)) if s == "direction") {
            self.pos += 1;
            match self.bump()? {
                Token::Ident(s) => match s.as_str() {
                    "forward" => GraphDirection::Forward,
                    "backward" => GraphDirection::Backward,
                    "both" => GraphDirection::Both,
                    other => return Err(ParseError(format!("unknown direction: {other}"))),
                },
                other => {
                    return Err(ParseError(format!(
                        "expected direction keyword, got {other:?}"
                    )))
                }
            }
        } else {
            GraphDirection::Forward
        };
        let edge_type: Option<String> =
            if matches!(self.peek(), Some(Token::Ident(s)) if s == "edge-type") {
                self.pos += 1; // consume 'edge-type'
                Some(self.parse_scalar_literal()?)
            } else {
                None
            };
        let type_filter = match &edge_type {
            Some(v) => format!(r#" AND e."type" = {v}"#),
            None => String::new(),
        };

        let edge_table = self.state.table.clone();

        // Anchor: each seed is its own origin at depth 0.
        let anchor = if seeds.len() == 1 {
            let s = &seeds[0];
            format!(
                "SELECT CAST({s} AS VARCHAR) AS origin, CAST({s} AS VARCHAR) AS node, 0 AS depth"
            )
        } else {
            let values = seeds
                .iter()
                .map(|s| format!("(CAST({s} AS VARCHAR))"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("SELECT node AS origin, node, 0 AS depth FROM (VALUES {values}) AS _seeds(node)")
        };
        // Next-hop node + the join condition, both direction-aware.
        let next_node = match direction {
            GraphDirection::Forward => format!("e.{dst_col}"),
            GraphDirection::Backward => format!("e.{src_col}"),
            GraphDirection::Both => {
                format!("CASE WHEN e.{src_col} = v.node THEN e.{dst_col} ELSE e.{src_col} END")
            }
        };
        let join_cond = match direction {
            GraphDirection::Forward => format!("e.{src_col} = v.node"),
            GraphDirection::Backward => format!("e.{dst_col} = v.node"),
            GraphDirection::Both => format!("(e.{src_col} = v.node OR e.{dst_col} = v.node)"),
        };
        let body = format!(
            "{anchor} \
             UNION ALL \
             SELECT v.origin, {next_node}, v.depth + 1 \
             FROM _vl v \
             JOIN {edge_table} e ON {join_cond} \
             WHERE v.depth < {max_hops}{type_filter}"
        );
        self.state
            .ctes
            .push(("_vl(origin, node, depth)".to_string(), body, true));

        // Shortest depth per (origin, node) — BFS-dedup over the enumerated
        // walks. DataFusion needs the aggregation in its own non-recursive CTE
        // before the range filter (same split as graph-traverse's _gt_min).
        let min_body = "SELECT origin, node, MIN(depth) AS depth FROM _vl GROUP BY origin, node";
        self.state
            .ctes
            .push(("_vl_min".to_string(), min_body.to_string(), false));

        let result_body = format!(
            "SELECT origin AS src, node AS dst, depth \
             FROM _vl_min \
             WHERE depth >= {min_hops} AND depth <= {max_hops}"
        );
        self.state
            .ctes
            .push(("_vl_result".to_string(), result_body, false));
        self.state.table = "_vl_result".to_string();
        Ok(())
    }

    /// `graph-var-match (a)-[r[:T]]->(b) min-hops M max-hops N [direction forward|backward|both] project <var>.<col> [as <alias>], …`
    ///
    /// The variable-length analogue of `graph-match`: binds the `(a, b)`
    /// endpoint pair of a single variable-length relationship and projects
    /// `a.*` / `b.*` node columns. Requires a preceding `make-graph`. This is
    /// what Cypher `MATCH (a)-[*M..N]->(b) RETURN a.…, b.…` lowers onto — the
    /// pair binding `graph-var-length` produces, joined back to the node table
    /// on both endpoints.
    ///
    /// Unlike `graph-var-length` (seeded), `a` ranges over EVERY node (the
    /// anchor scans the node table), matching open Cypher semantics. The
    /// recursion + shortest-depth dedup + `[min,max]` window are identical to
    /// `graph-var-length`; the result CTE additionally joins the node table on
    /// `origin` (alias `a`) and the reached node (alias `b`). The relationship
    /// variable has no per-edge columns (it denotes a whole path), so projecting
    /// `r.*` is rejected.
    fn op_graph_var_match(&mut self) -> Result<(), ParseError> {
        let def = self
            .state
            .graph_def
            .clone()
            .ok_or_else(|| ParseError("graph-var-match requires a preceding make-graph".into()))?;

        // Pattern: ( a ) -[ r [:T] ]-> ( b ) — a single forward var-length hop.
        self.expect(&Token::LParen, "(")?;
        let a_var = self.expect_ident()?;
        self.expect(&Token::RParen, ")")?;
        self.expect(&Token::Minus, "-")?;
        self.expect(&Token::LBracket, "[")?;
        let rel_var = self.expect_ident()?;
        let edge_type = if self.eat(&Token::Colon) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        self.expect(&Token::RBracket, "]")?;
        self.expect(&Token::RightArrow, "->")?;
        self.expect(&Token::LParen, "(")?;
        let b_var = self.expect_ident()?;
        self.expect(&Token::RParen, ")")?;

        self.expect_ident_eq("min-hops")?;
        let min_hops = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => {
                return Err(ParseError(format!(
                    "expected non-negative integer after `min-hops`, got {other:?}"
                )))
            }
        };
        self.expect_ident_eq("max-hops")?;
        let max_hops = match self.bump()? {
            Token::Int(n) if n >= 0 => n as u64,
            other => {
                return Err(ParseError(format!(
                    "expected non-negative integer after `max-hops`, got {other:?}"
                )))
            }
        };
        if max_hops == 0 {
            return Err(ParseError("graph-var-match: max-hops must be ≥ 1".into()));
        }
        if min_hops > max_hops {
            return Err(ParseError(format!(
                "graph-var-match: min-hops ({min_hops}) must be ≤ max-hops ({max_hops})"
            )));
        }
        let direction = if matches!(self.peek(), Some(Token::Ident(s)) if s == "direction") {
            self.pos += 1;
            match self.bump()? {
                Token::Ident(s) => match s.as_str() {
                    "forward" => GraphDirection::Forward,
                    "backward" => GraphDirection::Backward,
                    "both" => GraphDirection::Both,
                    other => return Err(ParseError(format!("unknown direction: {other}"))),
                },
                other => {
                    return Err(ParseError(format!(
                        "expected direction keyword, got {other:?}"
                    )))
                }
            }
        } else {
            GraphDirection::Forward
        };

        // project <var>.<col> [as <alias>], … — only a / b node vars.
        self.expect_ident_eq("project")?;
        let mut select_items: Vec<String> = Vec::new();
        loop {
            let var = self.expect_ident()?;
            self.expect(&Token::Dot, ".")?;
            let col = self.expect_ident()?;
            let alias_tbl = if var == a_var {
                "a"
            } else if var == b_var {
                "b"
            } else if var == rel_var {
                return Err(ParseError(format!(
                    "graph-var-match project: relationship `{rel_var}` has no columns in a variable-length match"
                )));
            } else {
                return Err(ParseError(format!(
                    "graph-var-match project: unknown variable `{var}`"
                )));
            };
            let out_alias = if self.eat_ident_eq("as") {
                self.expect_ident()?
            } else {
                format!("{var}_{col}")
            };
            select_items.push(format!(
                "{alias_tbl}.{} AS {}",
                quote_ident(&col),
                quote_ident(&out_alias)
            ));
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        if select_items.is_empty() {
            return Err(ParseError(
                "graph-var-match: project must list at least one column".into(),
            ));
        }

        let edges_tbl = &def.edge_table;
        let nodes_tbl = &def.node_table;
        let src = quote_ident(&def.src_col);
        let dst = quote_ident(&def.dst_col);
        let id = quote_ident(&def.id_col);
        let type_filter = match &edge_type {
            Some(t) => format!(r#" AND e."type" = '{}'"#, t.replace('\'', "''")),
            None => String::new(),
        };

        // Anchor: every node is a candidate `a` (open Cypher: `a` is unbound).
        let anchor = format!(
            "SELECT CAST({id} AS VARCHAR) AS origin, CAST({id} AS VARCHAR) AS node, 0 AS depth \
             FROM {nodes_tbl}"
        );
        let next_node = match direction {
            GraphDirection::Forward => format!("e.{dst}"),
            GraphDirection::Backward => format!("e.{src}"),
            GraphDirection::Both => {
                format!("CASE WHEN e.{src} = v.node THEN e.{dst} ELSE e.{src} END")
            }
        };
        let join_cond = match direction {
            GraphDirection::Forward => format!("e.{src} = v.node"),
            GraphDirection::Backward => format!("e.{dst} = v.node"),
            GraphDirection::Both => format!("(e.{src} = v.node OR e.{dst} = v.node)"),
        };
        let body = format!(
            "{anchor} \
             UNION ALL \
             SELECT v.origin, {next_node}, v.depth + 1 \
             FROM _vm v \
             JOIN {edges_tbl} e ON {join_cond} \
             WHERE v.depth < {max_hops}{type_filter}"
        );
        self.state
            .ctes
            .push(("_vm(origin, node, depth)".to_string(), body, true));

        let min_body = "SELECT origin, node, MIN(depth) AS depth FROM _vm GROUP BY origin, node";
        self.state
            .ctes
            .push(("_vm_min".to_string(), min_body.to_string(), false));

        // Bind both endpoints back to the node table, then project + window.
        let sel = select_items.join(", ");
        let result_body = format!(
            "SELECT {sel} FROM _vm_min m \
             JOIN {nodes_tbl} a ON m.origin = a.{id} \
             JOIN {nodes_tbl} b ON m.node = b.{id} \
             WHERE m.depth >= {min_hops} AND m.depth <= {max_hops}"
        );
        self.state
            .ctes
            .push(("_vm_result".to_string(), result_body, false));
        self.state.table = "_vm_result".to_string();
        Ok(())
    }

    /// `graph-shortest-path source <start> target <end> from <src> to <dst> max-hops N`
    ///
    /// Returns a single-row result with `depth` (min hops) and `found`.
    fn op_graph_shortest_path(&mut self) -> Result<(), ParseError> {
        self.expect_ident_eq("source")?;
        let source_sql = self.parse_scalar_literal()?;
        self.expect_ident_eq("target")?;
        let target_sql = self.parse_scalar_literal()?;
        let (src_col, dst_col, max_hops, direction) = self.parse_graph_preamble()?;

        let edge_table = self.state.table.clone();

        let body = format!(
            "SELECT CAST({source_sql} AS VARCHAR) AS node, 0 AS depth \
             UNION ALL \
             SELECT {step_sql} \
             FROM _sp t \
             JOIN {edge_table} e ON {join_cond} \
             WHERE t.depth < {max_hops}",
            step_sql = build_recursive_step(&edge_table, &src_col, &dst_col, direction),
            join_cond = match direction {
                GraphDirection::Forward => format!("e.{src_col} = t.node"),
                GraphDirection::Backward => format!("e.{dst_col} = t.node"),
                GraphDirection::Both => format!("(e.{src_col} = t.node OR e.{dst_col} = t.node)"),
            },
        );
        self.state
            .ctes
            .push(("_sp(node, depth)".to_string(), body, true));

        // DataFusion rejects scalar-subquery references to a recursive CTE,
        // so split the result into two non-recursive CTEs: `_sp_hits` filters,
        // `_sp_result` aggregates. MIN over an empty set yields NULL depth
        // (correct) and COUNT(*)>0 yields false for `found`.
        let hits_body = format!("SELECT depth FROM _sp WHERE node = CAST({target_sql} AS VARCHAR)");
        self.state
            .ctes
            .push(("_sp_hits".to_string(), hits_body, false));
        let result_body =
            "SELECT MIN(depth) AS depth, COUNT(*) > 0 AS found FROM _sp_hits".to_string();
        self.state
            .ctes
            .push(("_sp_result".to_string(), result_body, false));
        self.state.table = "_sp_result".to_string();
        Ok(())
    }

    /// `make-graph <src> --> <dst> with <NodeTable> on <id>`
    ///
    /// Records a [`crate::state::GraphDef`] on the state for the following
    /// `graph-match` operator. Emits no SQL itself — the active table is
    /// unchanged.
    fn op_make_graph(&mut self) -> Result<(), ParseError> {
        let src_col = self.expect_ident()?;
        if !self.eat(&Token::Arrow) {
            return Err(ParseError(
                "make-graph: expected `-->` between src and dst".into(),
            ));
        }
        let dst_col = self.expect_ident()?;
        self.expect_ident_eq("with")?;
        let node_table = self.expect_ident()?;
        self.expect_ident_eq("on")?;
        let id_col = self.expect_ident()?;
        self.state.graph_def = Some(crate::state::GraphDef {
            edge_table: self.state.table.clone(),
            src_col,
            dst_col,
            node_table,
            id_col,
        });
        Ok(())
    }

    /// `graph-match (a)-[e[:TYPE]]->(b)[-[e2[:T2]]->(c)…] project <var>.<col> [as <alias>], …`
    ///
    /// Requires a preceding `make-graph` (errors otherwise). Parses a forward
    /// chain of one or more hops and a required `project` list. Pushes a
    /// non-recursive CTE `_gm_result`.
    ///
    /// - **Single hop (N=1):** the historical 3-way join — edge table aliased
    ///   `e`, src node `a`, dst node `b` — with an optional `WHERE e."type"=…`.
    ///   Emitted byte-for-byte as before.
    /// - **Multi-hop (N≥2):** a (2N+1)-table join with positional aliases —
    ///   nodes `n0..nN`, edges `ed0..ed{N-1}` — chained
    ///   `ed_i.src = n_i.id` / `ed_i.dst = n_{i+1}.id`, with one
    ///   `ed_i."type"=…` per typed edge.
    fn op_graph_match(&mut self) -> Result<(), ParseError> {
        let def = self
            .state
            .graph_def
            .clone()
            .ok_or_else(|| ParseError("graph-match requires a preceding make-graph".into()))?;

        // Pattern: ( n0 ) -[ e0 [:T0] ]-> ( n1 ) [ -[ e1 [:T1] ]-> ( n2 ) ]…
        self.expect(&Token::LParen, "(")?;
        let mut nodes: Vec<String> = vec![self.expect_ident()?];
        self.expect(&Token::RParen, ")")?;
        let mut edges: Vec<(String, Option<String>)> = Vec::new();
        // First hop's `-`; subsequent hops are detected by eating the next `-`.
        self.expect(&Token::Minus, "-")?;
        loop {
            self.expect(&Token::LBracket, "[")?;
            let e = self.expect_ident()?;
            let edge_type = if self.eat(&Token::Colon) {
                Some(self.expect_ident()?)
            } else {
                None
            };
            self.expect(&Token::RBracket, "]")?;
            self.expect(&Token::RightArrow, "->")?;
            self.expect(&Token::LParen, "(")?;
            nodes.push(self.expect_ident()?);
            self.expect(&Token::RParen, ")")?;
            edges.push((e, edge_type));
            // Another hop iff the next token is `-` (consume it and continue).
            if !self.eat(&Token::Minus) {
                break;
            }
        }
        let single = edges.len() == 1;

        // Required: project <var>.<col> [as <alias>], …
        self.expect_ident_eq("project")?;
        let mut select_items: Vec<String> = Vec::new();
        loop {
            let var = self.expect_ident()?;
            self.expect(&Token::Dot, ".")?;
            let col = self.expect_ident()?;
            // Map the pattern variable to its SQL table alias. Single-hop keeps
            // the historical `a`/`e`/`b`; multi-hop uses positional `n*`/`ed*`.
            let alias_tbl: String = if single {
                if var == nodes[0] {
                    "a".to_string()
                } else if var == edges[0].0 {
                    "e".to_string()
                } else if var == nodes[1] {
                    "b".to_string()
                } else {
                    return Err(ParseError(format!(
                        "graph-match project: unknown variable `{var}`"
                    )));
                }
            } else if let Some(i) = nodes.iter().position(|n| *n == var) {
                format!("n{i}")
            } else if let Some(i) = edges.iter().position(|(e, _)| *e == var) {
                format!("ed{i}")
            } else {
                return Err(ParseError(format!(
                    "graph-match project: unknown variable `{var}`"
                )));
            };
            let out_alias = if self.eat_ident_eq("as") {
                self.expect_ident()?
            } else {
                format!("{var}_{col}")
            };
            select_items.push(format!(
                "{alias_tbl}.{} AS {}",
                quote_ident(&col),
                quote_ident(&out_alias)
            ));
            if !self.eat(&Token::Comma) {
                break;
            }
        }
        if select_items.is_empty() {
            return Err(ParseError(
                "graph-match: project must list at least one column".into(),
            ));
        }

        let sel = select_items.join(", ");
        let edges_tbl = &def.edge_table;
        let nodes_tbl = &def.node_table;
        let src = quote_ident(&def.src_col);
        let dst = quote_ident(&def.dst_col);
        let id = quote_ident(&def.id_col);

        let body = if single {
            // Byte-identical to the historical single-hop emission.
            let where_clause = match &edges[0].1 {
                Some(t) => format!(r#" WHERE e."type" = '{}'"#, t.replace('\'', "''")),
                None => String::new(),
            };
            format!(
                "SELECT {sel} FROM {edges_tbl} e \
                 JOIN {nodes_tbl} a ON e.{src} = a.{id} \
                 JOIN {nodes_tbl} b ON e.{dst} = b.{id}{where_clause}",
            )
        } else {
            // Multi-hop chain: ed0 joins n0 (src) and n1 (dst); ed1 joins n1
            // (src) and n2 (dst); … and so on.
            let mut from = format!(
                "{edges_tbl} ed0 \
                 JOIN {nodes_tbl} n0 ON ed0.{src} = n0.{id} \
                 JOIN {nodes_tbl} n1 ON ed0.{dst} = n1.{id}"
            );
            for i in 1..edges.len() {
                let j = i + 1;
                from.push_str(&format!(
                    " JOIN {edges_tbl} ed{i} ON ed{i}.{src} = n{i}.{id} \
                      JOIN {nodes_tbl} n{j} ON ed{i}.{dst} = n{j}.{id}"
                ));
            }
            let mut wheres: Vec<String> = Vec::new();
            for (i, (_, t)) in edges.iter().enumerate() {
                if let Some(t) = t {
                    wheres.push(format!(r#"ed{i}."type" = '{}'"#, t.replace('\'', "''")));
                }
            }
            let where_clause = if wheres.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", wheres.join(" AND "))
            };
            format!("SELECT {sel} FROM {from}{where_clause}")
        };
        self.state
            .ctes
            .push(("_gm_result".to_string(), body, false));
        self.state.table = "_gm_result".to_string();
        Ok(())
    }

    /// Parse the `source` value: either a single scalar literal, or a
    /// parenthesized comma list `( <lit>, <lit>, … )`. Returns the SQL scalar
    /// literal strings (already SQL-escaped by `parse_scalar_literal`).
    fn parse_source_seeds(&mut self) -> Result<Vec<String>, ParseError> {
        if self.eat(&Token::LParen) {
            let mut seeds = Vec::new();
            loop {
                seeds.push(self.parse_scalar_literal()?);
                if self.eat(&Token::Comma) {
                    continue;
                }
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

    /// Parse a single scalar literal (string, int, or bare identifier wrapped
    /// as a SQL literal). Used for graph operator arguments.
    fn parse_scalar_literal(&mut self) -> Result<String, ParseError> {
        match self.bump()? {
            Token::Str(s) => Ok(format!("'{}'", s.replace('\'', "''"))),
            Token::Int(n) => Ok(n.to_string()),
            Token::Float(f) => Ok(f.to_string()),
            other => Err(ParseError(format!("expected literal, got {other:?}"))),
        }
    }

    // -----------------------------------------------------------------
    // Expression parser (precedence climbing)
    // -----------------------------------------------------------------
    fn parse_expr(&mut self) -> Result<String, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<String, ParseError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Ident(s)) if s == "or") {
            self.pos += 1;
            let rhs = self.parse_and()?;
            lhs = format!("({lhs} OR {rhs})");
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<String, ParseError> {
        let mut lhs = self.parse_not()?;
        while matches!(self.peek(), Some(Token::Ident(s)) if s == "and") {
            self.pos += 1;
            let rhs = self.parse_not()?;
            lhs = format!("({lhs} AND {rhs})");
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<String, ParseError> {
        if matches!(self.peek(), Some(Token::Ident(s)) if s == "not") {
            self.pos += 1;
            let inner = self.parse_not()?;
            return Ok(format!("(NOT {inner})"));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<String, ParseError> {
        let lhs = self.parse_additive()?;

        // `between (low .. high)` — inclusive on both ends, lowers to AND of two comparisons.
        if matches!(self.peek(), Some(Token::Ident(s)) if s == "between") {
            self.pos += 1;
            self.expect(&Token::LParen, "`(`")?;
            let low = self.parse_additive()?;
            self.expect(&Token::DotDot, "`..`")?;
            let high = self.parse_additive()?;
            self.expect(&Token::RParen, "`)`")?;
            return Ok(format!("(({lhs} >= {low}) AND ({lhs} <= {high}))"));
        }

        // `in (v1, v2, ...)` — lower to SQL IN (...).
        if matches!(self.peek(), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("in")) {
            self.pos += 1;
            self.expect(&Token::LParen, "`(` after `in`")?;
            let mut values: Vec<String> = Vec::new();
            loop {
                values.push(self.parse_additive()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RParen, "`)`")?;
            return Ok(format!("({lhs} IN ({}))", values.join(", ")));
        }

        let op = match self.peek() {
            Some(Token::Eq) => Some("="),
            Some(Token::Ne) => Some("<>"),
            Some(Token::Lt) => Some("<"),
            Some(Token::Le) => Some("<="),
            Some(Token::Gt) => Some(">"),
            Some(Token::Ge) => Some(">="),
            Some(Token::Ident(s)) => match s.as_str() {
                "contains" => Some("KQL_CONTAINS"),
                "startswith" => Some("KQL_STARTSWITH"),
                "endswith" => Some("KQL_ENDSWITH"),
                "has" => Some("KQL_HAS"),
                _ => None,
            },
            _ => None,
        };
        let Some(op) = op else {
            return Ok(lhs);
        };
        self.pos += 1;
        let rhs = self.parse_additive()?;
        let result = match op {
            // `contains` → LIKE '%needle%'. Using LIKE (rather than
            // position) so `kyma-exec` can pattern-match and push down
            // extent-pruning to the catalog token index.
            "KQL_CONTAINS" => format!("({lhs} LIKE {})", add_like_both(&rhs)),
            "KQL_STARTSWITH" => format!("({lhs} LIKE {})", add_like_right(&rhs)),
            "KQL_ENDSWITH" => format!("({lhs} LIKE {})", add_like_left(&rhs)),
            // `has` — whole-word match. LIKE '%needle%' is close enough for
            // pruning; correctness comes from DataFusion applying a stricter
            // filter above the scan. (Future: emit regex for real.)
            "KQL_HAS" => format!("({lhs} LIKE {})", add_like_both(&rhs)),
            sql_op => format!("({lhs} {sql_op} {rhs})"),
        };
        Ok(result)
    }

    fn parse_additive(&mut self) -> Result<String, ParseError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => "+",
                Some(Token::Minus) => "-",
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_multiplicative()?;
            lhs = format!("({lhs} {op} {rhs})");
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<String, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => "*",
                Some(Token::Slash) => "/",
                Some(Token::Percent) => "%",
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = format!("({lhs} {op} {rhs})");
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<String, ParseError> {
        if self.eat(&Token::Minus) {
            let inner = self.parse_unary()?;
            return Ok(format!("(-{inner})"));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<String, ParseError> {
        let t = self.bump()?;
        match t {
            Token::Int(n) => Ok(n.to_string()),
            Token::Float(f) => Ok(f.to_string()),
            Token::Str(s) => Ok(format!("'{}'", s.replace('\'', "''"))),
            Token::Duration(d) => Ok(duration_to_sql_interval(&d)?),
            Token::LParen => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen, "`)`")?;
                Ok(format!("({inner})"))
            }
            Token::Star => Ok("*".to_string()),
            Token::Ident(name) => {
                // Function call?
                if self.eat(&Token::LParen) {
                    self.parse_func_call(name)
                } else if matches!(self.peek(), Some(Token::Dot)) {
                    // Dotted path (e.g. data.user.id). For MVP we translate
                    // to Postgres-style: data->>'user' etc. For our typed
                    // schema only, this is unlikely; flag unsupported.
                    Err(ParseError(format!(
                        "dotted path `{name}...` not yet supported (dynamic columns land with format-v1)"
                    )))
                } else {
                    // Plain column reference (handle `true`/`false` and datetime() below).
                    match name.as_str() {
                        "true" => Ok("TRUE".into()),
                        "false" => Ok("FALSE".into()),
                        "null" => Ok("NULL".into()),
                        _ => Ok(quote_ident(&name)),
                    }
                }
            }
            other => Err(ParseError(format!(
                "unexpected token in expression: {other:?}"
            ))),
        }
    }

    fn parse_func_call(&mut self, name: String) -> Result<String, ParseError> {
        let mut args = Vec::new();
        if !self.eat(&Token::RParen) {
            loop {
                args.push(self.parse_expr()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RParen, "`)`")?;
        }
        match (name.as_str(), args.as_slice()) {
            ("now", []) => Ok("now()".into()),
            ("ago", [d]) => Ok(format!("(now() - {d})")),
            // bin(col, 5m) → floor-to-bucket via date_bin when duration,
            // or plain integer division when a numeric bucket.
            ("bin", [col, bucket]) => Ok(format!(
                "date_bin({bucket}, {col}, TIMESTAMP '1970-01-01 00:00:00')"
            )),
            ("startofhour", [col]) => Ok(format!("date_trunc('hour', {col})")),
            ("startofday", [col]) => Ok(format!("date_trunc('day', {col})")),
            ("startofmonth", [col]) => Ok(format!("date_trunc('month', {col})")),
            ("datetime", [x]) => Ok(format!("CAST({x} AS TIMESTAMP)")),
            ("strcat", _) => Ok(format!("concat({})", args.join(", "))),
            ("tolower", [s]) => Ok(format!("lower({s})")),
            ("toupper", [s]) => Ok(format!("upper({s})")),
            ("strlen", [s]) => Ok(format!("char_length({s})")),
            ("isnull", [x]) => Ok(format!("({x} IS NULL)")),
            ("isnotnull", [x]) => Ok(format!("({x} IS NOT NULL)")),
            ("iff", [c, a, b]) => Ok(format!("(CASE WHEN {c} THEN {a} ELSE {b} END)")),
            // Aggregates pass-through (SQL has these verbatim).
            ("count", []) => Ok("count(*)".into()),
            ("count", [x]) => Ok(format!("count({x})")),
            ("sum", [x]) => Ok(format!("sum({x})")),
            ("avg", [x]) => Ok(format!("avg({x})")),
            ("min", [x]) => Ok(format!("min({x})")),
            ("max", [x]) => Ok(format!("max({x})")),
            ("dcount", [x]) => Ok(format!("count(DISTINCT {x})")),
            ("dcountif", [x, c]) => Ok(format!(
                "count(DISTINCT CASE WHEN {c} THEN {x} ELSE NULL END)"
            )),
            (other, _) => Err(ParseError(format!(
                "unsupported function: {other}/{}",
                args.len()
            ))),
        }
    }

    fn try_alias(&mut self) -> Result<Option<String>, ParseError> {
        // `name = expr` — lookahead two tokens.
        if let (Some(Token::Ident(a)), Some(Token::Assign)) =
            (self.toks.get(self.pos), self.toks.get(self.pos + 1))
        {
            let alias = a.clone();
            self.pos += 2;
            return Ok(Some(quote_ident(&alias)));
        }
        Ok(None)
    }
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum GraphDirection {
    Forward,
    Backward,
    Both,
}

/// A single `union` operand: where the branch reads from plus the root table
/// that determines its column schema for the outer-by-name superset.
struct UnionOperand {
    /// Table whose columns define this branch's contribution to the superset.
    root_table: String,
    from: UnionFrom,
}

/// The FROM-source of a union branch.
enum UnionFrom {
    /// Bare table reference; the branch reads it directly.
    Table(String),
    /// A parenthesized sub-pipeline already compiled to SQL; the superset
    /// projection wraps it as `SELECT … FROM (<sql>) _bN`.
    SubQuery(String),
}

/// The recursive-step SELECT list: which edge-column to pick up as the
/// next-hop node, plus depth+1. Forward traversal follows src→dst, backward
/// follows dst→src, both produces the endpoint that isn't the current node.
fn build_recursive_step(_table: &str, src_col: &str, dst_col: &str, dir: GraphDirection) -> String {
    match dir {
        GraphDirection::Forward => format!("e.{dst_col}, t.depth + 1"),
        GraphDirection::Backward => format!("e.{src_col}, t.depth + 1"),
        GraphDirection::Both => format!(
            "CASE WHEN e.{src_col} = t.node THEN e.{dst_col} ELSE e.{src_col} END, t.depth + 1"
        ),
    }
}

/// Like [`build_recursive_step`] but also projects `path_src` (the node we
/// hopped from) so the result CTE can join back to the edge table to retrieve
/// the full edge row for each reached node.
fn build_recursive_step_with_src(
    _table: &str,
    src_col: &str,
    dst_col: &str,
    dir: GraphDirection,
) -> String {
    match dir {
        GraphDirection::Forward => format!("e.{dst_col}, t.depth + 1, t.node"),
        GraphDirection::Backward => format!("e.{src_col}, t.depth + 1, t.node"),
        GraphDirection::Both => format!(
            "CASE WHEN e.{src_col} = t.node THEN e.{dst_col} ELSE e.{src_col} END, t.depth + 1, t.node"
        ),
    }
}

/// SQL reserved words that must be double-quoted even when they look like
/// plain identifiers (all alphanumeric). Subset covering what schema column
/// names are most likely to collide with.
const SQL_RESERVED: &[&str] = &[
    "all", "and", "any", "array", "as", "asc", "asymmetric", "authorization",
    "between", "both", "by", "case", "cast", "check", "collate", "column",
    "constraint", "create", "cross", "current", "current_date", "current_role",
    "current_time", "current_timestamp", "current_user", "default",
    "deferrable", "deferred", "delete", "desc", "distinct", "do", "else",
    "end", "except", "exists", "false", "fetch", "for", "foreign", "from",
    "full", "grant", "group", "having", "in", "initially", "inner", "intersect",
    "into", "is", "join", "leading", "left", "like", "limit", "localtime",
    "localtimestamp", "natural", "not", "null", "offset", "on", "only", "or",
    "order", "outer", "overlaps", "placing", "primary", "references", "right",
    "select", "session_user", "similar", "some", "symmetric", "table", "then",
    "to", "trailing", "true", "union", "unique", "user", "using", "variadic",
    "verbose", "when", "where", "window", "with",
];

/// Quote an identifier for SQL safely. We use double quotes (ANSI SQL).
/// Identifiers that are syntactically simple (all alphanumeric / underscore,
/// not starting with a digit) are still quoted when they collide with SQL
/// reserved words so that column names like `from` or `select` produce
/// valid SQL.
fn quote_ident(name: &str) -> String {
    // A name that needs quoting for structural reasons (spaces, specials, etc.)
    let structurally_simple = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name
            .chars()
            .next()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(false);

    if structurally_simple && !SQL_RESERVED.contains(&name.to_lowercase().as_str()) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

fn duration_to_sql_interval(d: &str) -> Result<String, ParseError> {
    let (n, unit) = d.split_at(d.len() - 1);
    let n: i64 = n
        .parse()
        .map_err(|_| ParseError(format!("bad duration literal: {d}")))?;
    let (secs, unit_name) = match unit {
        "s" => (n, "second"),
        "m" => (n * 60, "minute"),
        "h" => (n * 3600, "hour"),
        "d" => (n * 86400, "day"),
        other => return Err(ParseError(format!("unknown duration unit: {other}"))),
    };
    let _ = secs;
    Ok(format!("INTERVAL '{n} {unit_name}'"))
}

/// For `startswith "foo"` → `LIKE 'foo%'`. Input already SQL-quoted.
fn add_like_right(sql_string_literal: &str) -> String {
    let unquoted = strip_outer_quotes(sql_string_literal);
    format!("'{}%'", unquoted.replace('\'', "''"))
}
/// For `endswith "foo"` → `LIKE '%foo'`.
fn add_like_left(sql_string_literal: &str) -> String {
    let unquoted = strip_outer_quotes(sql_string_literal);
    format!("'%{}'", unquoted.replace('\'', "''"))
}
/// For `contains "foo"` / `has "foo"` → `LIKE '%foo%'`.
fn add_like_both(sql_string_literal: &str) -> String {
    let unquoted = strip_outer_quotes(sql_string_literal);
    format!("'%{}%'", unquoted.replace('\'', "''"))
}

/// Reverses the `'x'` SQL-string quoting to get the raw content; if the
/// input wasn't a quoted literal (e.g. a column ref), returns it wrapped in
/// a LIKE-escaping cast.
fn strip_outer_quotes(s: &str) -> String {
    if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
        s[1..s.len() - 1].replace("''", "'")
    } else {
        // Non-literal — wrap via concat (`col LIKE concat(x,'%')`).
        s.to_string()
    }
}

fn implicit_agg_alias(expr: &str) -> String {
    // Make up a safe alias so DataFusion doesn't assign "count(UInt8(1))".
    let id: String = expr
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let id = id.trim_matches('_').to_string();
    if id.is_empty() {
        "expr".into()
    } else {
        id.chars().take(40).collect()
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn must(src: &str) -> String {
        kql_to_sql(src).expect("parse")
    }

    fn schemas(
        pairs: &[(&str, &[&str])],
    ) -> std::collections::HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(t, cols)| {
                (
                    t.to_string(),
                    cols.iter().map(|c| c.to_string()).collect(),
                )
            })
            .collect()
    }
    fn must_with(
        src: &str,
        s: &std::collections::HashMap<String, Vec<String>>,
    ) -> String {
        kql_to_sql_with_schemas(src, s).expect("parse")
    }

    #[test]
    fn union_two_tables_outer_by_name() {
        let s = schemas(&[("a", &["ts", "msg"]), ("b", &["ts", "code"])]);
        let sql = must_with("union a, b", &s);
        // Column superset in first-seen order: ts, msg, code.
        assert!(sql.contains(r#"SELECT ts, msg, NULL AS code FROM a"#), "{sql}");
        assert!(sql.contains(r#"SELECT ts, NULL AS msg, code FROM b"#), "{sql}");
        assert!(sql.to_uppercase().contains("UNION ALL"), "{sql}");
    }

    #[test]
    fn union_with_source_column() {
        let s = schemas(&[("a", &["x"]), ("b", &["x"])]);
        let sql = must_with(r#"union withsource=src a, b"#, &s);
        assert!(sql.contains(r#"'a' AS src"#), "{sql}");
        assert!(sql.contains(r#"'b' AS src"#), "{sql}");
    }

    #[test]
    fn union_parenthesized_pipeline_operand() {
        let s = schemas(&[("a", &["ts", "msg"]), ("b", &["ts", "msg"])]);
        let sql = must_with(r#"union a, (b | where msg contains "x") | take 10"#, &s);
        assert!(sql.contains("LIKE"), "{sql}");
        assert!(sql.to_uppercase().contains("LIMIT 10"), "{sql}");
    }

    #[test]
    fn union_trailing_ops_apply_to_union() {
        let s = schemas(&[("a", &["ts"]), ("b", &["ts"])]);
        let sql = must_with(
            r#"union a, b | where ts > datetime("2026-01-01T00:00:00Z") | take 5"#,
            &s,
        );
        // WHERE must scope the union result, not a single branch.
        let upper = sql.to_uppercase();
        let union_pos = upper.find("UNION ALL").unwrap();
        let where_pos = upper.rfind("WHERE").unwrap();
        assert!(
            where_pos > union_pos,
            "trailing where must be outside the union: {sql}"
        );
    }

    #[test]
    fn union_unknown_table_errors() {
        let s = schemas(&[("a", &["x"])]);
        let e = kql_to_sql_with_schemas("union a, nope", &s).unwrap_err();
        assert!(e.to_string().contains("nope"), "{e}");
    }

    #[test]
    fn union_without_schemas_errors_clearly() {
        let e = kql_to_sql("union a, b").unwrap_err();
        assert!(e.to_string().contains("schema"), "{e}");
    }

    #[test]
    fn non_union_queries_unaffected_by_schema_api() {
        let s = schemas(&[("t", &["x"])]);
        assert_eq!(must_with("t | take 5", &s), kql_to_sql("t | take 5").unwrap());
    }

    #[test]
    fn both_apis_byte_identical_for_non_union() {
        // The binding constraint: every non-union query compiles byte-for-byte
        // identically through kql_to_sql and kql_to_sql_with_schemas.
        let s = std::collections::HashMap::new();
        for q in [
            "t",
            "t | where status >= 500 | project timestamp, status",
            "t | summarize count(), avg(latency) by status | sort by status desc | take 10",
            r#"e | graph-traverse source "a" from src to dst max-hops 2"#,
            r#"edges | make-graph caller --> callee with services on id | graph-match (a)-[e:CALLS]->(b) project a.name"#,
        ] {
            assert_eq!(
                kql_to_sql(q).unwrap(),
                kql_to_sql_with_schemas(q, &s).unwrap(),
                "mismatch for: {q}"
            );
        }
    }

    // --- Additional contract-sharpening tests ---

    #[test]
    fn union_single_operand() {
        // A single-operand union is degenerate but must still compile: one
        // branch, no UNION ALL keyword needed, superset = that table's cols.
        let s = schemas(&[("a", &["ts", "msg"])]);
        let sql = must_with("union a", &s);
        assert!(sql.contains("SELECT ts, msg FROM a"), "{sql}");
        assert!(sql.to_uppercase().contains(" FROM _U"), "{sql}");
    }

    #[test]
    fn union_trailing_sort_and_take() {
        let s = schemas(&[("a", &["ts"]), ("b", &["ts"])]);
        let sql = must_with("union a, b | sort by ts desc | take 5", &s);
        let upper = sql.to_uppercase();
        let union_pos = upper.find("UNION ALL").unwrap();
        let order_pos = upper.rfind("ORDER BY").unwrap();
        assert!(order_pos > union_pos, "sort must scope the union: {sql}");
        assert!(upper.contains("ORDER BY TS DESC"), "{sql}");
        assert!(upper.contains("LIMIT 5"), "{sql}");
    }

    #[test]
    fn union_withsource_trailing_where_on_source() {
        // The withsource column participates in the result schema so a
        // trailing where over it scopes the union output.
        let s = schemas(&[("a", &["x"]), ("b", &["x"])]);
        let sql = must_with(r#"union withsource=src a, b | where src == "a""#, &s);
        let upper = sql.to_uppercase();
        let union_pos = upper.find("UNION ALL").unwrap();
        let where_pos = upper.rfind("WHERE").unwrap();
        assert!(where_pos > union_pos, "where on src must be outside union: {sql}");
        assert!(sql.contains("(src = 'a')"), "{sql}");
    }

    #[test]
    fn union_withsource_column_is_first_in_projection() {
        // withsource column FIRST so it's part of the result schema.
        let s = schemas(&[("a", &["x"]), ("b", &["y"])]);
        let sql = must_with("union withsource=src a, b", &s);
        // Branch for `a`: src first, then superset x, NULL AS y.
        assert!(sql.contains("SELECT 'a' AS src, x, NULL AS y FROM a"), "{sql}");
        assert!(sql.contains("SELECT 'b' AS src, NULL AS x, y FROM b"), "{sql}");
    }

    #[test]
    fn union_parenthesized_operand_wraps_subquery() {
        // A parenthesized operand compiles its sub-pipeline and the superset
        // projection wraps it: SELECT <proj> FROM (<branch sql>) _bN.
        let s = schemas(&[("a", &["ts", "msg"]), ("b", &["ts", "msg"])]);
        let sql = must_with(r#"union (a | where msg contains "x"), b"#, &s);
        assert!(sql.contains(") _b0"), "expected subquery wrapper alias: {sql}");
        assert!(sql.contains("LIKE '%x%'"), "{sql}");
    }

    #[test]
    fn simple_scan() {
        assert_eq!(must("t"), "SELECT * FROM t");
    }

    #[test]
    fn where_and_project() {
        let sql = must("t | where status >= 500 | project timestamp, status");
        assert!(sql.contains("SELECT timestamp, status FROM t"));
        assert!(sql.contains("(status >= 500)"));
    }

    #[test]
    fn take_n() {
        assert_eq!(must("t | take 5"), "SELECT * FROM t LIMIT 5");
    }

    #[test]
    fn project_away_excludes_columns() {
        // `project-away` must actually drop the columns — DataFusion supports
        // the `* EXCEPT (...)` wildcard option. A bare `SELECT *` here leaks
        // excluded columns (e.g. 384-float embedding vectors) into search
        // previews and breaks the unified-search RRF row dedup.
        assert_eq!(
            must("t | project-away embedding | take 5"),
            "SELECT * EXCEPT (embedding) FROM t LIMIT 5"
        );
        assert_eq!(
            must("t | project-away a, b"),
            "SELECT * EXCEPT (a, b) FROM t"
        );
    }

    #[test]
    fn count_aggregate() {
        let sql = must("t | count");
        assert!(
            sql.starts_with("SELECT count(*) AS \"Count\" FROM t"),
            "got: {sql}"
        );
    }

    #[test]
    fn summarize_by() {
        let sql = must("t | summarize count() by status");
        assert!(sql.contains("GROUP BY status"));
    }

    #[test]
    fn sort_desc() {
        let sql = must("t | sort by timestamp desc");
        assert!(sql.contains("ORDER BY timestamp DESC"));
    }

    #[test]
    fn extend_arith() {
        let sql = must("t | extend class = status / 100");
        assert!(sql.contains("AS class"));
    }

    #[test]
    fn ago_literal() {
        let sql = must("t | where timestamp > ago(1h)");
        assert!(sql.contains("now() - INTERVAL '1 hour'"));
    }

    #[test]
    fn contains_op() {
        let sql = must(r#"t | where body contains "oom""#);
        assert!(sql.contains("position") || sql.contains("LIKE"));
    }

    #[test]
    fn startswith_op() {
        let sql = must(r#"t | where path startswith "/api""#);
        assert!(sql.contains("LIKE '/api%'"));
    }

    #[test]
    fn top_operator() {
        let sql = must("t | top 5 by latency desc");
        assert!(sql.contains("ORDER BY latency DESC"));
        assert!(sql.contains("LIMIT 5"));
    }

    #[test]
    fn between_with_ago_and_now() {
        let sql = must("t | where timestamp between (ago(1h) .. now())");
        // Should lower to two comparisons joined by AND.
        assert!(sql.contains(">= (now() - INTERVAL '1 hour')"));
        assert!(sql.contains("<= now()"));
        assert!(sql.contains("AND"));
    }

    #[test]
    fn between_with_datetime_literals() {
        let sql =
            must("t | where timestamp between (datetime(2026-01-01) .. datetime(2026-02-01))");
        assert!(sql.contains("CAST("));
        assert!(sql.contains("AND"));
    }

    #[test]
    fn between_combined_with_and() {
        let sql = must(r#"t | where n between (1 .. 10) and label == "x""#);
        // between part
        assert!(sql.contains("(n >= 1)"));
        assert!(sql.contains("(n <= 10)"));
        // label filter
        assert!(sql.contains("(label = 'x')"));
        // outer AND joining between and label
        assert!(sql.contains("AND"));
    }

    #[test]
    fn between_lowering_shape() {
        // Verify exact lowered form: ((expr >= low) AND (expr <= high))
        let sql = must("t | where n between (1 .. 10)");
        assert!(
            sql.contains("((n >= 1) AND (n <= 10))"),
            "unexpected shape: {sql}"
        );
    }

    #[test]
    fn parses_in_with_string_list() {
        let kql = r#"context_nodes | where label in ("a", "b", "c") | take 10"#;
        let sql = kql_to_sql(kql).expect("parse");
        assert!(sql.contains("IN"), "expected SQL to contain 'IN', got: {sql}");
        assert!(sql.contains("'a'"), "expected 'a' in SQL, got: {sql}");
        assert!(sql.contains("'c'"), "expected 'c' in SQL, got: {sql}");
    }

    #[test]
    fn parses_in_with_int_list() {
        let kql = "logs | where status in (200, 201, 204)";
        let sql = kql_to_sql(kql).expect("parse");
        assert!(sql.contains("IN"), "expected SQL IN, got: {sql}");
        assert!(sql.contains("200"), "got: {sql}");
        assert!(sql.contains("204"), "got: {sql}");
    }

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
    #[test]
    fn graph_traverse_multi_source_and_edge_type_compose() {
        let sql = kql_to_sql(r#"e | graph-traverse source ("a","b") from src to dst max-hops 2 edge-type "X""#).expect("parse");
        assert!(sql.to_uppercase().contains("VALUES") && sql.contains(r#"e."type" = 'X'"#), "got: {sql}");
    }

    // ---- graph-var-length (endpoint-preserving reachability) -----------
    #[test]
    fn graph_var_length_emits_origin_preserving_three_cte_shape() {
        let sql = kql_to_sql(
            r#"e | graph-var-length source "a" from src to dst min-hops 2 max-hops 4"#,
        )
        .expect("parse");
        // Origin carried through the recursion (the pair binding graph-traverse lacks).
        assert!(sql.contains("AS origin"), "origin column expected, got: {sql}");
        assert!(sql.contains("v.origin"), "origin carried in recursive step, got: {sql}");
        // Shortest-depth dedup, THEN the [min,max] window (oracle semantics).
        assert!(
            sql.contains("MIN(depth)") && sql.contains("GROUP BY origin, node"),
            "shortest-depth dedup expected, got: {sql}"
        );
        assert!(
            sql.contains("depth >= 2 AND depth <= 4"),
            "range filter applied after MIN, got: {sql}"
        );
        // Forward by default: step forward along src→dst, recursion bounded by max.
        assert!(sql.contains("v.depth < 4"), "recursion bounded by max-hops, got: {sql}");
        assert!(sql.contains("AS src") && sql.contains("AS dst"), "endpoint pair output, got: {sql}");
    }

    #[test]
    fn graph_var_length_backward_and_both_directions() {
        // Backward steps along dst→src and joins on the dst endpoint.
        let back = kql_to_sql(
            r#"e | graph-var-length source "a" from src to dst min-hops 1 max-hops 3 direction backward"#,
        )
        .expect("parse");
        assert!(back.contains("e.dst = v.node"), "backward join on dst, got: {back}");
        // Both expands either endpoint via CASE.
        let both = kql_to_sql(
            r#"e | graph-var-length source "a" from src to dst min-hops 1 max-hops 3 direction both"#,
        )
        .expect("parse");
        assert!(
            both.contains("CASE WHEN e.src = v.node") && both.contains("OR e.dst = v.node"),
            "both-direction CASE + OR join, got: {both}"
        );
    }

    #[test]
    fn graph_var_length_multi_seed_and_edge_type_compose() {
        let sql = kql_to_sql(
            r#"e | graph-var-length source ("a","b") from src to dst min-hops 1 max-hops 2 edge-type "CALLS""#,
        )
        .expect("parse");
        assert!(sql.to_uppercase().contains("VALUES"), "multi-seed VALUES anchor, got: {sql}");
        assert!(sql.contains(r#"e."type" = 'CALLS'"#), "per-hop edge-type filter, got: {sql}");
    }

    #[test]
    fn graph_var_length_rejects_bad_bounds() {
        assert!(
            kql_to_sql(r#"e | graph-var-length source "a" from src to dst min-hops 3 max-hops 1"#)
                .is_err(),
            "min > max must be rejected"
        );
        assert!(
            kql_to_sql(r#"e | graph-var-length source "a" from src to dst min-hops 0 max-hops 0"#)
                .is_err(),
            "max-hops 0 must be rejected"
        );
    }

    #[test]
    fn make_graph_then_match_projects_join() {
        let sql = kql_to_sql(
            r#"edges | make-graph caller --> callee with services on id | graph-match (a)-[e:CALLS]->(b) project a.name, e.latency, b.name"#
        ).expect("parse");
        let u = sql.to_uppercase();
        // Node table joined twice (a = src side, b = dst side).
        assert!(
            u.contains("JOIN SERVICES A ON"),
            "expected node join for a, got: {sql}"
        );
        assert!(
            u.contains("JOIN SERVICES B ON"),
            "expected node join for b, got: {sql}"
        );
        // Join predicates: quote_ident("caller")="caller", quote_ident("id")="id" — simple idents, no quotes.
        assert!(
            sql.contains("e.caller = a.id"),
            "a joins on src=id, got: {sql}"
        );
        assert!(
            sql.contains("e.callee = b.id"),
            "b joins on dst=id, got: {sql}"
        );
        // Edge-type filter uses the conventional quoted "type" column.
        assert!(
            sql.contains(r#"e."type" = 'CALLS'"#),
            "edge-type filter, got: {sql}"
        );
        // Projected columns: quote_ident("name")="name" (simple), "latency"="latency" (simple).
        // Aliases: a_name, e_latency, b_name (auto from var_col).
        assert!(
            sql.contains("a.name AS a_name"),
            "expected a.name AS a_name, got: {sql}"
        );
        assert!(
            sql.contains("e.latency AS e_latency"),
            "expected e.latency AS e_latency, got: {sql}"
        );
        assert!(
            sql.contains("b.name AS b_name"),
            "expected b.name AS b_name, got: {sql}"
        );
    }

    #[test]
    fn graph_match_without_make_graph_errors() {
        let err = kql_to_sql(r#"edges | graph-match (a)-[e]->(b) project a.x"#);
        assert!(
            err.is_err(),
            "graph-match requires a preceding make-graph"
        );
    }

    #[test]
    fn make_graph_then_multi_hop_match_chains_joins() {
        let sql = kql_to_sql(
            r#"edges | make-graph caller --> callee with services on id | graph-match (a)-[e1:CALLS]->(b)-[e2:DEPENDS]->(c) project a.name, c.name"#
        ).expect("2-hop parse");
        // Positional aliases: ed0/ed1 edges, n0/n1/n2 nodes.
        assert!(sql.contains("ed0.caller = n0.id"), "ed0 src→n0: {sql}");
        assert!(sql.contains("ed0.callee = n1.id"), "ed0 dst→n1: {sql}");
        assert!(sql.contains("ed1.caller = n1.id"), "ed1 src→n1 (chain): {sql}");
        assert!(sql.contains("ed1.callee = n2.id"), "ed1 dst→n2: {sql}");
        // Per-edge type filters AND-ed.
        assert!(
            sql.contains(r#"ed0."type" = 'CALLS'"#) && sql.contains(r#"ed1."type" = 'DEPENDS'"#),
            "per-edge type filters: {sql}"
        );
        // Endpoint projections map to the right positional aliases.
        assert!(sql.contains("n0.name AS a_name"), "a→n0: {sql}");
        assert!(sql.contains("n2.name AS c_name"), "c→n2: {sql}");
    }

    #[test]
    fn make_graph_then_match_single_hop_unchanged() {
        // The N=1 path must stay byte-identical to the historical emission.
        let sql = kql_to_sql(
            r#"edges | make-graph caller --> callee with services on id | graph-match (a)-[e:CALLS]->(b) project a.name"#
        ).expect("1-hop parse");
        assert!(sql.contains("e.caller = a.id") && sql.contains("e.callee = b.id"), "single-hop aliases e/a/b: {sql}");
        assert!(!sql.contains("ed0") && !sql.contains("n0"), "single-hop must NOT use positional aliases: {sql}");
    }

    #[test]
    fn graph_traverse_projects_full_edge_columns() {
        let kql = r#"context_edges | graph-traverse source "a" from src to dst max-hops 2"#;
        let sql = kql_to_sql(kql).expect("parse");
        // Result query should select original edge columns alongside depth
        assert!(
            sql.contains("e.*"),
            "expected edge-row projection (e.*), got: {sql}"
        );
        assert!(sql.contains("depth"), "expected depth column, got: {sql}");
        // dst column should still be present via edge join
        assert!(sql.contains("dst"), "expected dst column, got: {sql}");
        // Recursive CTE should now have path_src for the join-back
        assert!(
            sql.contains("path_src"),
            "expected path_src in CTE, got: {sql}"
        );
    }

    // ------------------------------------------------------------------
    // C1 — nested union rejected cleanly
    // ------------------------------------------------------------------

    #[test]
    fn union_nested_errors_with_clear_message() {
        // `union a, (union b, c)` must error with "nested union", not leak
        // internal names like `_u`.
        let s = schemas(&[("a", &["ts"]), ("b", &["ts"]), ("c", &["ts"])]);
        let err = kql_to_sql_with_schemas("union a, (union b, c)", &s).unwrap_err();
        assert!(
            err.to_string().contains("nested union"),
            "expected 'nested union' in error, got: {err}"
        );
        assert!(
            !err.to_string().contains("_u"),
            "internal CTE name `_u` must not leak into error: {err}"
        );
    }

    // ------------------------------------------------------------------
    // I1 — withsource collision rejected at parse time
    // ------------------------------------------------------------------

    #[test]
    fn union_withsource_collision_errors() {
        // `ts` exists in both operand schemas → collision with withsource=ts.
        let s = schemas(&[("a", &["ts", "msg"]), ("b", &["ts"])]);
        let err = kql_to_sql_with_schemas("union withsource=ts a, b", &s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("withsource") && msg.contains("ts") && msg.contains("collides"),
            "expected withsource collision error, got: {msg}"
        );
    }

    // ------------------------------------------------------------------
    // I2 — superset columns routed through quote_ident
    // ------------------------------------------------------------------

    #[test]
    fn union_superset_reserved_word_column_quoted() {
        // A column named `from` (SQL reserved word) must appear double-quoted
        // in the emitted SQL via quote_ident.
        let s = schemas(&[("a", &["from", "ts"]), ("b", &["ts"])]);
        let sql = kql_to_sql_with_schemas("union a, b", &s).expect("parse");
        // quote_ident("from") → `"from"` because `from` is SQL-reserved.
        assert!(
            sql.contains(r#""from""#),
            "expected `\"from\"` (quoted reserved word) in SQL, got: {sql}"
        );
        // The null-fill branch for b must also be quoted.
        assert!(
            sql.contains(r#"NULL AS "from""#),
            "expected `NULL AS \"from\"` in SQL, got: {sql}"
        );
    }

    // ------------------------------------------------------------------
    // Fix 4 — schema-reshaping branch ops rejected at parse time
    // ------------------------------------------------------------------

    #[test]
    fn union_branch_with_project_errors() {
        let s = schemas(&[("a", &["x"]), ("b", &["x"])]);
        let err = kql_to_sql_with_schemas("union a, (b | project x)", &s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("project"),
            "error must name the offending operator, got: {msg}"
        );
        assert!(
            msg.contains("reshapes"),
            "error must mention schema reshaping, got: {msg}"
        );
    }

    #[test]
    fn union_branch_with_extend_errors() {
        let s = schemas(&[("a", &["x"]), ("b", &["x"])]);
        let err = kql_to_sql_with_schemas("union a, (b | extend y=1)", &s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("extend"),
            "error must name `extend`, got: {msg}"
        );
        assert!(
            msg.contains("reshapes"),
            "expected reshape message, got: {msg}"
        );
    }

    #[test]
    fn union_branch_filter_and_take_still_allowed() {
        // `where` + `take` are not schema-reshaping → must compile fine.
        let s = schemas(&[("a", &["x"]), ("b", &["x"])]);
        let sql = must_with("union a, (b | where x > 1 | take 3)", &s);
        assert!(
            sql.to_uppercase().contains("WHERE"),
            "filter branch should compile, got: {sql}"
        );
        assert!(
            sql.to_uppercase().contains("LIMIT 3"),
            "take inside branch should compile, got: {sql}"
        );
    }
}

