//! Cypher → KQL translator (v1 single-hop `MATCH` subset).
//!
//! A pure, IO-free function that lowers a tiny, well-defined slice of Cypher
//! into the KQL graph dialect this crate already understands
//! (`make-graph` + `graph-match`, see [`crate::parser`]). The output is a KQL
//! string; feed it to [`crate::kql_to_sql`] /
//! [`crate::kql_to_sql_with_schemas`] to obtain SQL.
//!
//! # Supported subset
//!
//! ```cypher
//! MATCH (a[:Label])-[r[:TYPE]]->(b[:Label])
//! [WHERE <preds>]
//! RETURN <proj>
//! [LIMIT n]
//! ```
//!
//! - Single hop only. Directed (`-[r]->`, `<-[r]-`) and undirected (`-[r]-`,
//!   treated as forward in v1).
//! - Node `:Label` → `| where <var>_<label_col> == 'Label'` after the match.
//! - Rel `:TYPE` → the `graph-match` `[r:TYPE]` edge filter.
//! - `WHERE` simple comparisons (`= <> < > <= >=`) on `<var>.<prop>` vs a
//!   string/number literal, AND-joined.
//! - `RETURN <var>.<prop> [AS <alias>]`, or bare `RETURN <var>`.
//! - `LIMIT n` → `| take n`.
//!
//! # Why projections are flattened
//!
//! `graph-match` lowers to a CTE (`_gm_result`) whose columns are the *project
//! aliases* (`a_name`, …). KQL's own `where`/`take` then run against that CTE,
//! and KQL's expression grammar rejects dotted refs (`a.name`). So every
//! `<var>.<prop>` referenced by a node-label filter or `WHERE` predicate is
//! added to the `graph-match` projection (aliased `<var>_<prop>`), and the
//! trailing `| where` references those flat alias names.
//!
//! # Deferred constructs (rejected with a precise error)
//!
//! Multi-hop, variable-length `-[*..]->`, `OPTIONAL MATCH`, multiple `MATCH`,
//! `WITH`, `ORDER BY`, aggregation, `CREATE/SET/DELETE/MERGE`, `shortestPath`,
//! `RETURN *`.

use crate::ParseError;

/// How the abstract Cypher graph maps onto concrete edge/node tables and
/// columns. All emitted KQL is parameterized by this binding.
#[derive(Debug, Clone)]
pub struct GraphBinding {
    /// Edge table — the pipeline source and the `make-graph` edge table.
    pub edge_table: String,
    /// Node table joined on both endpoints.
    pub node_table: String,
    /// Node primary-key column (`make-graph ... on <id_col>`).
    pub id_col: String,
    /// Edge source-endpoint column.
    pub src_col: String,
    /// Edge destination-endpoint column.
    pub dst_col: String,
    /// Edge type/relationship column (informational; the underlying
    /// `graph-match` lowers `:TYPE` against the conventional `"type"` column).
    pub type_col: String,
    /// Node label column used to lower `:Label` node filters.
    pub label_col: String,
}

/// Translate a single-hop Cypher `MATCH` into KQL.
///
/// Returns `Err(ParseError("cypher: …"))` for unsupported / malformed input.
pub fn cypher_to_kql(cypher: &str, g: &GraphBinding) -> Result<String, ParseError> {
    let toks = tokenize(cypher)?;
    let mut p = Parser::new(toks);
    let query = p.parse_query()?;
    emit(&query, g)
}

// =====================================================================
// Tokenizer (self-contained — deliberately not the KQL lexer)
// =====================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Bareword: keyword or identifier. Keyword-ness is decided by the parser.
    Word(String),
    /// Quoted string literal (single or double quotes), unescaped contents.
    Str(String),
    /// Numeric literal, kept as written so it round-trips into KQL.
    Num(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Comma,
    Colon,
    Star,
    /// `-`
    Dash,
    /// `->`
    ArrowR,
    /// `<-`
    ArrowL,
    Eq,    // =
    Ne,    // <>
    Lt,    // <
    Gt,    // >
    Le,    // <=
    Ge,    // >=
}

fn tokenize(src: &str) -> Result<Vec<Tok>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let n = chars.len();
    let mut out = Vec::new();

    while i < n {
        let c = chars[i];

        if c.is_whitespace() {
            i += 1;
            continue;
        }

        match c {
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                out.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                out.push(Tok::RBracket);
                i += 1;
            }
            '.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '-' => {
                // `->` or `-`
                if i + 1 < n && chars[i + 1] == '>' {
                    out.push(Tok::ArrowR);
                    i += 2;
                } else {
                    out.push(Tok::Dash);
                    i += 1;
                }
            }
            '<' => {
                // `<-`, `<=`, `<>`, `<`
                if i + 1 < n && chars[i + 1] == '-' {
                    out.push(Tok::ArrowL);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '=' {
                    out.push(Tok::Le);
                    i += 2;
                } else if i + 1 < n && chars[i + 1] == '>' {
                    out.push(Tok::Ne);
                    i += 2;
                } else {
                    out.push(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < n && chars[i + 1] == '=' {
                    out.push(Tok::Ge);
                    i += 2;
                } else {
                    out.push(Tok::Gt);
                    i += 1;
                }
            }
            '=' => {
                out.push(Tok::Eq);
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < n {
                    let d = chars[i];
                    if d == '\\' && i + 1 < n {
                        // Minimal escape handling so `\'` / `\"` don't end the string.
                        s.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if d == quote {
                        closed = true;
                        i += 1;
                        break;
                    }
                    s.push(d);
                    i += 1;
                }
                if !closed {
                    return Err(ParseError(
                        "cypher: unterminated string literal".to_string(),
                    ));
                }
                out.push(Tok::Str(s));
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                while i < n && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                out.push(Tok::Num(chars[start..i].iter().collect()));
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Tok::Word(chars[start..i].iter().collect()));
            }
            other => {
                return Err(ParseError(format!(
                    "cypher: unexpected character `{other}`"
                )));
            }
        }
    }

    Ok(out)
}

// =====================================================================
// AST
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum Direction {
    Forward,
    Backward,
    Undirected,
}

#[derive(Debug, Clone)]
struct NodePat {
    var: String,
    label: Option<String>,
}

#[derive(Debug, Clone)]
struct RelPat {
    var: String,
    rel_type: Option<String>,
    direction: Direction,
}

#[derive(Debug, Clone)]
struct Comparison {
    var: String,
    prop: String,
    op: CmpOp,
    /// Already SQL/KQL-ready literal text (string literals are single-quoted).
    literal: String,
}

#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl CmpOp {
    /// KQL operator text. `=`→`==`, `<>`→`!=`; the rest pass through.
    fn kql(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone)]
enum ReturnItem {
    /// `RETURN <var>.<prop> [AS <alias>]`
    Prop {
        var: String,
        prop: String,
        alias: Option<String>,
    },
    /// `RETURN <var>` (bare) — expands to id + label columns.
    Var { var: String },
}

#[derive(Debug, Clone)]
struct Query {
    a: NodePat,
    rel: RelPat,
    b: NodePat,
    where_preds: Vec<Comparison>,
    returns: Vec<ReturnItem>,
    limit: Option<i64>,
}

// =====================================================================
// Parser (recursive descent)
// =====================================================================

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(toks: Vec<Tok>) -> Self {
        Parser { toks, pos: 0 }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume a bareword if it matches `kw` (case-insensitive).
    fn eat_kw(&mut self, kw: &str) -> bool {
        if let Some(Tok::Word(w)) = self.peek() {
            if w.eq_ignore_ascii_case(kw) {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    /// Peek a bareword keyword without consuming (case-insensitive).
    fn peek_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case(kw))
    }

    fn expect_word(&mut self) -> Result<String, ParseError> {
        match self.bump() {
            Some(Tok::Word(w)) => Ok(w),
            other => Err(ParseError(format!(
                "cypher: expected an identifier, got {other:?}"
            ))),
        }
    }

    fn parse_query(&mut self) -> Result<Query, ParseError> {
        // Reject up front any leading construct we do not support.
        self.reject_unsupported_leading()?;

        if !self.eat_kw("MATCH") {
            return Err(ParseError(
                "cypher: expected MATCH at start of query".to_string(),
            ));
        }

        let (a, rel, b) = self.parse_pattern()?;

        // A second relationship/node ⇒ multi-hop.
        if matches!(self.peek(), Some(Tok::Dash | Tok::ArrowL)) {
            return Err(unsupported("multi-hop pattern"));
        }

        // A second MATCH / OPTIONAL MATCH / WITH before RETURN ⇒ unsupported.
        if self.peek_kw("MATCH") || self.peek_kw("OPTIONAL") {
            return Err(unsupported("multiple MATCH clauses"));
        }
        if self.peek_kw("WITH") {
            return Err(unsupported("WITH"));
        }

        let where_preds = if self.eat_kw("WHERE") {
            self.parse_where()?
        } else {
            Vec::new()
        };

        if !self.eat_kw("RETURN") {
            return Err(ParseError(
                "cypher: expected RETURN".to_string(),
            ));
        }
        let returns = self.parse_return()?;

        // Post-RETURN tail: only LIMIT is allowed.
        if self.peek_kw("ORDER") {
            return Err(unsupported("ORDER BY"));
        }
        if self.peek_kw("SKIP") {
            return Err(unsupported("SKIP"));
        }

        let limit = if self.eat_kw("LIMIT") {
            Some(self.parse_limit()?)
        } else {
            None
        };

        if self.peek_kw("UNION") {
            return Err(unsupported("UNION"));
        }

        if self.pos != self.toks.len() {
            return Err(ParseError(format!(
                "cypher: unexpected trailing tokens after query: {:?}",
                &self.toks[self.pos..]
            )));
        }

        Ok(Query {
            a,
            rel,
            b,
            where_preds,
            returns,
            limit,
        })
    }

    /// Reject write/aggregate/optional constructs before we even reach MATCH.
    fn reject_unsupported_leading(&self) -> Result<(), ParseError> {
        for (kw, feat) in [
            ("OPTIONAL", "OPTIONAL MATCH"),
            ("CREATE", "CREATE"),
            ("MERGE", "MERGE"),
            ("SET", "SET"),
            ("DELETE", "DELETE"),
            ("REMOVE", "REMOVE"),
            ("CALL", "CALL"),
            ("UNWIND", "UNWIND"),
        ] {
            if self.peek_kw(kw) {
                return Err(unsupported(feat));
            }
        }
        Ok(())
    }

    /// `(a[:L])-[r[:T]]->(b[:L])` (or `<-`, or undirected `-`).
    fn parse_pattern(&mut self) -> Result<(NodePat, RelPat, NodePat), ParseError> {
        let a = self.parse_node()?;

        // Leading side of the relationship: `<-[` (backward) or `-[` (forward/undirected).
        let leading_backward = if self.eat(&Tok::ArrowL) {
            true
        } else if self.eat(&Tok::Dash) {
            false
        } else {
            return Err(ParseError(
                "cypher: expected a relationship `-[...]-` after the first node".to_string(),
            ));
        };

        if !self.eat(&Tok::LBracket) {
            return Err(ParseError(
                "cypher: expected `[` to open relationship pattern".to_string(),
            ));
        }

        // Relationship variable (optional in Cypher; default to `r`).
        let rel_var = if matches!(self.peek(), Some(Tok::Word(_))) {
            self.expect_word()?
        } else {
            "r".to_string()
        };

        // `:TYPE` filter, or variable-length `*` (unsupported).
        let mut rel_type = None;
        if self.eat(&Tok::Colon) {
            rel_type = Some(self.expect_word()?);
        }
        if matches!(self.peek(), Some(Tok::Star)) {
            return Err(unsupported("variable-length relationship `-[*..]->`"));
        }

        if !self.eat(&Tok::RBracket) {
            return Err(ParseError(
                "cypher: expected `]` to close relationship pattern".to_string(),
            ));
        }

        // Trailing side: `->` (forward), `-` (undirected), and the backward
        // case was opened with `<-` so here it must close with `-`.
        let trailing_forward = if self.eat(&Tok::ArrowR) {
            true
        } else if self.eat(&Tok::Dash) {
            false
        } else {
            return Err(ParseError(
                "cypher: expected `->` or `-` to close the relationship".to_string(),
            ));
        };

        let direction = match (leading_backward, trailing_forward) {
            (false, true) => Direction::Forward,    // -[]->
            (true, false) => Direction::Backward,   // <-[]-
            (false, false) => Direction::Undirected, // -[]-
            (true, true) => {
                // <-[]-> : both arrows — not a thing in v1.
                return Err(unsupported("bidirectional `<-[]->` relationship"));
            }
        };

        let b = self.parse_node()?;

        let rel = RelPat {
            var: rel_var,
            rel_type,
            direction,
        };
        Ok((a, rel, b))
    }

    /// `(var[:Label])`. The variable is required in v1 (anonymous nodes `()`
    /// have no handle to project, so we reject them).
    fn parse_node(&mut self) -> Result<NodePat, ParseError> {
        if !self.eat(&Tok::LParen) {
            return Err(ParseError(
                "cypher: expected `(` to open a node pattern".to_string(),
            ));
        }
        let var = match self.peek() {
            Some(Tok::Word(_)) => self.expect_word()?,
            _ => {
                return Err(unsupported("anonymous node `()`"));
            }
        };
        let label = if self.eat(&Tok::Colon) {
            Some(self.expect_word()?)
        } else {
            None
        };
        if !self.eat(&Tok::RParen) {
            return Err(ParseError(
                "cypher: expected `)` to close a node pattern".to_string(),
            ));
        }
        Ok(NodePat { var, label })
    }

    /// AND-joined comparisons. `OR` and parenthesized predicates are deferred.
    fn parse_where(&mut self) -> Result<Vec<Comparison>, ParseError> {
        let mut preds = Vec::new();
        loop {
            preds.push(self.parse_comparison()?);
            if self.eat_kw("AND") {
                continue;
            }
            if self.peek_kw("OR") {
                return Err(unsupported("OR in WHERE"));
            }
            break;
        }
        Ok(preds)
    }

    fn parse_comparison(&mut self) -> Result<Comparison, ParseError> {
        let var = self.expect_word()?;
        if !self.eat(&Tok::Dot) {
            return Err(ParseError(
                "cypher: WHERE predicate must be `<var>.<prop> <op> <literal>`".to_string(),
            ));
        }
        let prop = self.expect_word()?;
        let op = match self.bump() {
            Some(Tok::Eq) => CmpOp::Eq,
            Some(Tok::Ne) => CmpOp::Ne,
            Some(Tok::Lt) => CmpOp::Lt,
            Some(Tok::Gt) => CmpOp::Gt,
            Some(Tok::Le) => CmpOp::Le,
            Some(Tok::Ge) => CmpOp::Ge,
            other => {
                return Err(ParseError(format!(
                    "cypher: expected a comparison operator in WHERE, got {other:?}"
                )));
            }
        };
        let literal = self.parse_literal()?;
        Ok(Comparison {
            var,
            prop,
            op,
            literal,
        })
    }

    /// A string or numeric literal, rendered as KQL-ready text.
    fn parse_literal(&mut self) -> Result<String, ParseError> {
        match self.bump() {
            Some(Tok::Str(s)) => Ok(format!("'{}'", s.replace('\'', "''"))),
            Some(Tok::Num(n)) => Ok(n),
            other => Err(ParseError(format!(
                "cypher: expected a string or number literal, got {other:?}"
            ))),
        }
    }

    fn parse_return(&mut self) -> Result<Vec<ReturnItem>, ParseError> {
        if matches!(self.peek(), Some(Tok::Star)) {
            return Err(unsupported("RETURN *"));
        }
        if self.peek_kw("DISTINCT") {
            return Err(unsupported("RETURN DISTINCT"));
        }

        let mut items = Vec::new();
        loop {
            let var = self.expect_word()?;
            // Reject aggregation like count(...) / collect(...).
            if matches!(self.peek(), Some(Tok::LParen)) {
                return Err(unsupported("aggregation / function call in RETURN"));
            }
            if self.eat(&Tok::Dot) {
                let prop = self.expect_word()?;
                let alias = if self.eat_kw("AS") {
                    Some(self.expect_word()?)
                } else {
                    None
                };
                items.push(ReturnItem::Prop { var, prop, alias });
            } else {
                // Bare `RETURN <var>` — `AS` not meaningful, reject if present
                // for clarity.
                if self.eat_kw("AS") {
                    return Err(ParseError(
                        "cypher: `AS` is only supported on `<var>.<prop>` return items"
                            .to_string(),
                    ));
                }
                items.push(ReturnItem::Var { var });
            }
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        Ok(items)
    }

    fn parse_limit(&mut self) -> Result<i64, ParseError> {
        match self.bump() {
            Some(Tok::Num(n)) => n
                .parse::<i64>()
                .map_err(|_| ParseError(format!("cypher: invalid LIMIT value `{n}`"))),
            other => Err(ParseError(format!(
                "cypher: LIMIT expects an integer, got {other:?}"
            ))),
        }
    }
}

/// Build the standard "not supported yet" error for a deferred feature.
fn unsupported(feature: &str) -> ParseError {
    ParseError(format!(
        "cypher: {feature} not supported yet (v1 single-hop MATCH)"
    ))
}

// =====================================================================
// Emitter
// =====================================================================

/// A single `var.prop` projection needed by the emitted graph-match, keyed by
/// its flat output alias so trailing KQL ops can reference it.
struct Projection {
    var: String,
    col: String,
    /// Flat alias in `_gm_result` (`<var>_<col>` unless overridden by RETURN AS).
    alias: String,
}

fn emit(q: &Query, g: &GraphBinding) -> Result<String, ParseError> {
    // Validate that all referenced variables are one of the two node vars or
    // the relationship var. `graph-match` would reject unknown vars anyway, but
    // a Cypher-level message is friendlier.
    let known: [&str; 3] = [&q.a.var, &q.rel.var, &q.b.var];
    let check_var = |v: &str| -> Result<(), ParseError> {
        if known.iter().any(|k| *k == v) {
            Ok(())
        } else {
            Err(ParseError(format!(
                "cypher: unknown variable `{v}` (pattern declares {}, {}, {})",
                q.a.var, q.rel.var, q.b.var
            )))
        }
    };

    // ---- Collect projections in a stable order, de-duplicated by alias. ----
    let mut projections: Vec<Projection> = Vec::new();
    let mut push_proj = |var: &str, col: &str, alias: String| {
        if !projections.iter().any(|p| p.alias == alias) {
            projections.push(Projection {
                var: var.to_string(),
                col: col.to_string(),
                alias,
            });
        }
    };

    // 1) RETURN items first, so user-facing columns come first in `_gm_result`.
    for item in &q.returns {
        match item {
            ReturnItem::Prop { var, prop, alias } => {
                check_var(var)?;
                let out = alias.clone().unwrap_or_else(|| format!("{var}_{prop}"));
                push_proj(var, prop, out);
            }
            ReturnItem::Var { var } => {
                check_var(var)?;
                // Bare var → id + label columns.
                push_proj(var, &g.id_col, format!("{var}_id"));
                push_proj(var, &g.label_col, format!("{var}_label"));
            }
        }
    }

    // 2) Node-label filters need the label column projected.
    let mut label_filters: Vec<(String, String)> = Vec::new(); // (alias, label)
    for node in [&q.a, &q.b] {
        if let Some(label) = &node.label {
            let alias = format!("{}_{}", node.var, g.label_col);
            push_proj(&node.var, &g.label_col, alias.clone());
            label_filters.push((alias, label.clone()));
        }
    }

    // 3) WHERE predicates need their referenced props projected.
    let mut where_clauses: Vec<String> = Vec::new();
    for pred in &q.where_preds {
        check_var(&pred.var)?;
        let alias = format!("{}_{}", pred.var, pred.prop);
        push_proj(&pred.var, &pred.prop, alias.clone());
        where_clauses.push(format!("{} {} {}", alias, pred.op.kql(), pred.literal));
    }

    if projections.is_empty() {
        return Err(ParseError(
            "cypher: RETURN must project at least one column".to_string(),
        ));
    }

    // ---- graph-match pattern (always emitted forward; v1 swaps endpoints
    // for backward direction so the underlying forward-only matcher is correct).
    let (left, right) = match q.rel.direction {
        Direction::Backward => (&q.b.var, &q.a.var),
        _ => (&q.a.var, &q.b.var),
    };
    let rel_pat = match &q.rel.rel_type {
        Some(t) => format!("[{}:{}]", q.rel.var, t),
        None => format!("[{}]", q.rel.var),
    };

    // ---- project list ----
    let proj_list = projections
        .iter()
        .map(|p| format!("{}.{} as {}", p.var, p.col, p.alias))
        .collect::<Vec<_>>()
        .join(", ");

    // ---- assemble pipeline ----
    let mut kql = format!(
        "{edges} | make-graph {src} --> {dst} with {nodes} on {id} \
         | graph-match ({left})-{rel}->({right}) project {proj}",
        edges = g.edge_table,
        src = g.src_col,
        dst = g.dst_col,
        nodes = g.node_table,
        id = g.id_col,
        rel = rel_pat,
        proj = proj_list,
    );

    // Label filters + WHERE predicates run against the flat `_gm_result` columns.
    let mut filters: Vec<String> = Vec::new();
    for (alias, label) in &label_filters {
        filters.push(format!("{} == '{}'", alias, label.replace('\'', "''")));
    }
    filters.extend(where_clauses);
    for f in filters {
        kql.push_str(&format!(" | where {f}"));
    }

    if let Some(n) = q.limit {
        kql.push_str(&format!(" | take {n}"));
    }

    Ok(kql)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{kql_to_sql_with_schemas, SchemaMap};

    fn binding() -> GraphBinding {
        GraphBinding {
            edge_table: "github_edges".to_string(),
            node_table: "github_nodes".to_string(),
            id_col: "id".to_string(),
            src_col: "src".to_string(),
            dst_col: "dst".to_string(),
            type_col: "type".to_string(),
            label_col: "labels".to_string(),
        }
    }

    fn tr(cypher: &str) -> String {
        cypher_to_kql(cypher, &binding()).expect("cypher should translate")
    }

    // ---- happy path: the canonical example ----------------------------
    #[test]
    fn basic_forward_match_projects_both_endpoints() {
        let kql = tr("MATCH (a)-[r:HAS_ARTIFACT]->(b) RETURN a.name, b.name");
        assert!(kql.contains("github_edges"), "{kql}");
        assert!(
            kql.contains("make-graph src --> dst with github_nodes on id"),
            "{kql}"
        );
        assert!(
            kql.contains("graph-match (a)-[r:HAS_ARTIFACT]->(b)"),
            "{kql}"
        );
        assert!(
            kql.contains("project a.name as a_name, b.name as b_name"),
            "{kql}"
        );
    }

    // ---- node :Label filter -------------------------------------------
    #[test]
    fn node_label_lowers_to_where_on_label_col() {
        let kql = tr("MATCH (a:Commit)-[r]->(b) RETURN a.sha");
        // Label column must be projected (aliased a_labels) and filtered.
        assert!(kql.contains("a.labels as a_labels"), "{kql}");
        assert!(kql.contains("| where a_labels == 'Commit'"), "{kql}");
    }

    // ---- WHERE simple comparisons -------------------------------------
    #[test]
    fn where_string_and_number_predicates() {
        let kql = tr("MATCH (a)-[r]->(b) WHERE a.x = 'v' AND b.n > 3 RETURN a.x, b.n");
        // `=` → `==`, projected flat aliases referenced in trailing where.
        assert!(kql.contains("| where a_x == 'v'"), "{kql}");
        assert!(kql.contains("| where b_n > 3"), "{kql}");
    }

    #[test]
    fn where_not_equal_lowers_to_bang_eq() {
        let kql = tr("MATCH (a)-[r]->(b) WHERE a.x <> 'v' RETURN a.x");
        assert!(kql.contains("| where a_x != 'v'"), "{kql}");
    }

    #[test]
    fn where_passthrough_operators() {
        let kql = tr("MATCH (a)-[r]->(b) WHERE a.n <= 5 AND b.m >= 2 RETURN a.n, b.m");
        assert!(kql.contains("a_n <= 5"), "{kql}");
        assert!(kql.contains("b_m >= 2"), "{kql}");
    }

    // ---- RETURN AS alias ----------------------------------------------
    #[test]
    fn return_alias_overrides_default_column_name() {
        let kql = tr("MATCH (a)-[r]->(b) RETURN a.name AS n");
        assert!(kql.contains("a.name as n"), "{kql}");
    }

    // ---- LIMIT ---------------------------------------------------------
    #[test]
    fn limit_lowers_to_take() {
        let kql = tr("MATCH (a)-[r]->(b) RETURN a.name LIMIT 10");
        assert!(kql.contains("| take 10"), "{kql}");
    }

    // ---- undirected (treated as forward) ------------------------------
    #[test]
    fn undirected_treated_as_forward() {
        let kql = tr("MATCH (a)-[r:LINKS]-(b) RETURN a.name");
        assert!(
            kql.contains("graph-match (a)-[r:LINKS]->(b)"),
            "{kql}"
        );
    }

    // ---- backward direction swaps endpoints ---------------------------
    #[test]
    fn backward_swaps_endpoints() {
        let kql = tr("MATCH (a)<-[r:CHILD]-(b) RETURN a.name");
        // The forward-only matcher sees (b)-[r:CHILD]->(a).
        assert!(
            kql.contains("graph-match (b)-[r:CHILD]->(a)"),
            "{kql}"
        );
    }

    // ---- bare RETURN <var> --------------------------------------------
    #[test]
    fn bare_return_var_expands_to_id_and_label() {
        let kql = tr("MATCH (a)-[r]->(b) RETURN a");
        assert!(
            kql.contains("a.id as a_id, a.labels as a_label"),
            "{kql}"
        );
    }

    // ---- unsupported constructs → Err ---------------------------------
    #[test]
    fn multi_hop_rejected() {
        let err = cypher_to_kql(
            "MATCH (a)-[r]->(b)-[s]->(c) RETURN a.name",
            &binding(),
        );
        let msg = err.unwrap_err().0;
        assert!(msg.contains("multi-hop"), "{msg}");
        assert!(msg.contains("not supported yet"), "{msg}");
    }

    #[test]
    fn create_rejected() {
        let err = cypher_to_kql("CREATE (a:Foo) RETURN a", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("CREATE"), "{msg}");
        assert!(msg.contains("not supported yet"), "{msg}");
    }

    #[test]
    fn variable_length_rejected() {
        let err = cypher_to_kql("MATCH (a)-[r*1..3]->(b) RETURN a.name", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("variable-length"), "{msg}");
    }

    #[test]
    fn return_star_rejected() {
        let err = cypher_to_kql("MATCH (a)-[r]->(b) RETURN *", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("RETURN *"), "{msg}");
    }

    #[test]
    fn optional_match_rejected() {
        let err = cypher_to_kql("OPTIONAL MATCH (a)-[r]->(b) RETURN a.name", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("OPTIONAL MATCH"), "{msg}");
    }

    #[test]
    fn order_by_rejected() {
        let err = cypher_to_kql(
            "MATCH (a)-[r]->(b) RETURN a.name ORDER BY a.name",
            &binding(),
        );
        let msg = err.unwrap_err().0;
        assert!(msg.contains("ORDER BY"), "{msg}");
    }

    #[test]
    fn with_rejected() {
        let err = cypher_to_kql(
            "MATCH (a)-[r]->(b) WITH a RETURN a.name",
            &binding(),
        );
        let msg = err.unwrap_err().0;
        assert!(msg.contains("WITH"), "{msg}");
    }

    // ---- compile-through: emitted KQL must lower to SQL ----------------
    #[test]
    fn compiles_through_to_sql_basic() {
        let kql = tr("MATCH (a)-[r:HAS_ARTIFACT]->(b) RETURN a.name, b.name");
        let res = kql_to_sql_with_schemas(&kql, &SchemaMap::default());
        assert!(res.is_ok(), "KQL did not compile: {kql}\nerr: {res:?}");
    }

    #[test]
    fn compiles_through_to_sql_with_label_where_and_limit() {
        let kql = tr(
            "MATCH (a:Commit)-[r:HAS_ARTIFACT]->(b) WHERE a.x = 'v' AND b.n > 3 RETURN a.name AS n, b.name LIMIT 10",
        );
        let res = kql_to_sql_with_schemas(&kql, &SchemaMap::default());
        assert!(res.is_ok(), "KQL did not compile: {kql}\nerr: {res:?}");
    }

    #[test]
    fn compiles_through_bare_return_var() {
        let kql = tr("MATCH (a)-[r]->(b) RETURN a");
        let res = kql_to_sql_with_schemas(&kql, &SchemaMap::default());
        assert!(res.is_ok(), "KQL did not compile: {kql}\nerr: {res:?}");
    }
}
