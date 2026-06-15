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
//! MATCH (a[:Label])-[r[:TYPE]]->(b[:Label])[-[r2[:T2]]->(c)…]
//! [WHERE <preds>]
//! RETURN <proj>
//! [LIMIT n]
//! ```
//!
//! - One or more **forward** hops. A single hop also supports directed
//!   (`-[r]->`, `<-[r]-`) and undirected (`-[r]-`, treated as forward); a
//!   multi-hop chain is forward-only (a backward `<-` hop is rejected, as it
//!   would break the linear node order the matcher needs).
//! - A **single variable-length** hop `-[r*M..N]->` (also `*`, `*N`, `*..N`,
//!   `*M..`) lowers to `graph-var-match`, binding the `(a, b)` endpoint pair
//!   over a bounded recursive traversal. Directed and undirected; an open
//!   upper bound is capped at [`DEFAULT_VAR_LENGTH_MAX`]. The relationship
//!   variable denotes a whole path, so projecting `r.*` is rejected.
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
//! Variable-length inside a multi-hop chain, backward hops inside a multi-hop
//! chain, `OPTIONAL MATCH`, non-chain (star / disjoint) multi-pattern MATCH,
//! `WITH`, `ORDER BY`, aggregation, `CREATE/SET/DELETE/MERGE`, `RETURN *`.
//!
//! Multiple patterns — comma-separated or in successive `MATCH` clauses — ARE
//! supported when each continues the chain (its first node is the previous
//! pattern's last node), e.g. `MATCH (a)-[r]->(b), (b)-[s]->(c)` ≡ the chain
//! `(a)-[r]->(b)-[s]->(c)`. Disjoint / star multi-patterns need a join and stay
//! deferred.
//!
//! `shortestPath` IS supported in the form
//! `MATCH p = shortestPath((a)-[*..N]->(b)) WHERE a.<id> = '…' AND b.<id> = '…'
//! RETURN length(p)` — it lowers to the `graph-shortest-path` operator (both
//! endpoints must be pinned by id in WHERE; the only projection is `length(p)`).

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
    /// `Some((min, max))` for a variable-length relationship `-[*min..max]-`;
    /// `None` for an ordinary single hop.
    var_length: Option<(u64, u64)>,
}

/// Upper bound substituted for an open variable-length range (`[*]`, `[*N..]`).
/// The engine lowers var-length to a `max`-bounded recursive CTE that must
/// terminate, so an unbounded Cypher range is capped here. Generous enough for
/// realistic graph queries; an explicit `[*min..max]` overrides it.
const DEFAULT_VAR_LENGTH_MAX: u64 = 10;

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
    /// `RETURN length(<path>) [AS <alias>]` — only valid in a `shortestPath`
    /// query; projects the shortest-path hop count.
    PathLength {
        path_var: String,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct Query {
    /// Pattern node chain `n0, n1, … nN` (length = `rels.len() + 1`).
    nodes: Vec<NodePat>,
    /// Pattern relationships `r0 … r{N-1}`; `rels[i]` connects `nodes[i]`→`nodes[i+1]`.
    rels: Vec<RelPat>,
    where_preds: Vec<Comparison>,
    returns: Vec<ReturnItem>,
    limit: Option<i64>,
    /// `Some(path_var)` when the MATCH is `<path_var> = shortestPath((a)-[…]->(b))`.
    /// The single hop in `nodes`/`rels` is the wrapped pattern; the lowering
    /// emits `graph-shortest-path` instead of `graph-match`.
    shortest_path: Option<String>,
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

        // Optional `<path> = shortestPath( <pattern> )` wrapper. Detected by a
        // bareword immediately followed by `=`.
        let mut shortest_path: Option<String> = None;
        if matches!(self.peek(), Some(Tok::Word(_)))
            && matches!(self.toks.get(self.pos + 1), Some(Tok::Eq))
        {
            let pvar = self.expect_word()?;
            self.eat(&Tok::Eq); // consume `=`
            let func = self.expect_word()?;
            if !func.eq_ignore_ascii_case("shortestPath") {
                return Err(unsupported(&format!(
                    "path function `{func}` (only shortestPath is supported)"
                )));
            }
            if !self.eat(&Tok::LParen) {
                return Err(ParseError(
                    "cypher: expected `(` after shortestPath".to_string(),
                ));
            }
            shortest_path = Some(pvar);
        }

        // Pattern chain: node ( rel node )+ — one or more hops.
        let mut nodes = vec![self.parse_node()?];
        let mut rels: Vec<RelPat> = Vec::new();
        loop {
            rels.push(self.parse_rel()?);
            nodes.push(self.parse_node()?);
            // Another hop iff the next token starts a relationship (`-` or `<-`).
            if !matches!(self.peek(), Some(Tok::Dash | Tok::ArrowL)) {
                break;
            }
        }

        // Close the `shortestPath( … )` wrapper.
        if shortest_path.is_some() && !self.eat(&Tok::RParen) {
            return Err(ParseError(
                "cypher: expected `)` to close shortestPath(...)".to_string(),
            ));
        }

        // Additional patterns — via `,` or a subsequent `MATCH` clause — are
        // supported when they CONTINUE the chain: the new pattern's first node
        // is the current last node, e.g. `(a)-[r]->(b), (b)-[s]->(c)` which is
        // the chain `(a)-[r]->(b)-[s]->(c)`. Non-chain (star / disjoint)
        // multi-patterns need a join and stay unsupported. shortestPath is
        // single-pattern, so chaining only applies to the plain MATCH form.
        if shortest_path.is_none() {
            loop {
                let cont = if self.eat(&Tok::Comma) {
                    true
                } else if self.peek_kw("MATCH") {
                    self.eat_kw("MATCH");
                    true
                } else {
                    false
                };
                if !cont {
                    break;
                }
                let first = self.parse_node()?;
                let last_var = nodes.last().map(|n| n.var.clone()).unwrap_or_default();
                if first.var != last_var {
                    return Err(unsupported(
                        "non-chain multi-pattern MATCH (patterns must share the connecting node, \
                         e.g. `(a)-[r]->(b), (b)-[s]->(c)`)",
                    ));
                }
                // Append the continuation hops; the shared first node is already
                // the chain's tail, so it is not pushed again.
                loop {
                    rels.push(self.parse_rel()?);
                    nodes.push(self.parse_node()?);
                    if !matches!(self.peek(), Some(Tok::Dash | Tok::ArrowL)) {
                        break;
                    }
                }
            }
        }

        // OPTIONAL MATCH, a trailing MATCH after shortestPath, or WITH ⇒ unsupported.
        if self.peek_kw("OPTIONAL") || self.peek_kw("MATCH") {
            return Err(unsupported("OPTIONAL MATCH / non-chain multiple MATCH"));
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
            nodes,
            rels,
            where_preds,
            returns,
            limit,
            shortest_path,
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

    /// Parse one relationship between two nodes: `-[r[:T]]->` (forward),
    /// `<-[r[:T]]-` (backward), or `-[r[:T]]-` (undirected). The flanking nodes
    /// are parsed by the caller's chain loop in [`Parser::parse_query`].
    fn parse_rel(&mut self) -> Result<RelPat, ParseError> {
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

        // `:TYPE` filter.
        let mut rel_type = None;
        if self.eat(&Tok::Colon) {
            rel_type = Some(self.expect_word()?);
        }

        // Variable-length spec `*`, `*N`, `*M..N`, `*M..`, `*..N` (openCypher:
        // `*` ⇒ 1..∞, `*N` ⇒ exactly N, `*..N` ⇒ 1..N, `*M..` ⇒ M..∞). The
        // number lexer greedily eats `.`, so a present lower bound arrives fused
        // with the range dots — `*1..3` lexes as `Star Num("1..3")`, `*1..` as
        // `Star Num("1..")`, `*N` as `Star Num("N")` — while an absent lower
        // bound keeps the dots separate — `*..3` is `Star Dot Dot Num("3")`. We
        // split the fused token on `..`. Unbounded upper bounds are capped at
        // [`DEFAULT_VAR_LENGTH_MAX`] so the recursive lowering terminates.
        let var_length = if self.eat(&Tok::Star) {
            let parse_bound = |s: &str| -> Result<u64, ParseError> {
                s.parse::<u64>()
                    .map_err(|_| ParseError(format!("cypher: invalid variable-length bound `{s}`")))
            };
            let (min, max) = if matches!(self.peek(), Some(Tok::Num(_))) {
                let raw = match self.bump() {
                    Some(Tok::Num(s)) => s,
                    _ => unreachable!("peeked a Num"),
                };
                if let Some(idx) = raw.find("..") {
                    let lo = &raw[..idx];
                    let hi = &raw[idx + 2..];
                    let min = if lo.is_empty() { 1 } else { parse_bound(lo)? };
                    let max = if hi.is_empty() {
                        DEFAULT_VAR_LENGTH_MAX
                    } else {
                        parse_bound(hi)?
                    };
                    (min, max)
                } else {
                    let n = parse_bound(&raw)?; // `*N` ⇒ exactly N
                    (n, n)
                }
            } else if self.eat(&Tok::Dot) {
                // `*..N` / `*..` — lower bound omitted (defaults to 1).
                if !self.eat(&Tok::Dot) {
                    return Err(ParseError(
                        "cypher: expected `..` in variable-length relationship".to_string(),
                    ));
                }
                let max = if matches!(self.peek(), Some(Tok::Num(_))) {
                    match self.bump() {
                        Some(Tok::Num(s)) => parse_bound(&s)?,
                        _ => unreachable!("peeked a Num"),
                    }
                } else {
                    DEFAULT_VAR_LENGTH_MAX
                };
                (1, max)
            } else {
                (1, DEFAULT_VAR_LENGTH_MAX) // bare `*` ⇒ 1..cap
            };
            if max == 0 {
                return Err(ParseError(
                    "cypher: variable-length max must be ≥ 1".to_string(),
                ));
            }
            if min > max {
                return Err(ParseError(format!(
                    "cypher: variable-length min ({min}) must be ≤ max ({max})"
                )));
            }
            Some((min, max))
        } else {
            None
        };

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

        Ok(RelPat {
            var: rel_var,
            rel_type,
            direction,
            var_length,
        })
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
            // `length(<path>)` — the only supported function (shortestPath length).
            if var.eq_ignore_ascii_case("length") && matches!(self.peek(), Some(Tok::LParen)) {
                self.eat(&Tok::LParen);
                let path_var = self.expect_word()?;
                if !self.eat(&Tok::RParen) {
                    return Err(ParseError(
                        "cypher: expected `)` after length(<path>)".to_string(),
                    ));
                }
                let alias = if self.eat_kw("AS") {
                    Some(self.expect_word()?)
                } else {
                    None
                };
                items.push(ReturnItem::PathLength { path_var, alias });
                if !self.eat(&Tok::Comma) {
                    break;
                }
                continue;
            }
            // Reject other aggregation / function calls like count(...) / collect(...).
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

/// Lower `<path> = shortestPath((a)-[…]->(b))` onto the `graph-shortest-path`
/// KQL operator. Both endpoints must be pinned by `<var>.<id_col> = '…'` in
/// WHERE (that's how the operator gets its source/target), and the only
/// supported projection is `length(<path>)` (the hop count).
fn emit_shortest_path(q: &Query, g: &GraphBinding, path_var: &str) -> Result<String, ParseError> {
    if q.rels.len() != 1 {
        return Err(unsupported("shortestPath over a multi-hop pattern"));
    }
    let a = q.nodes[0].var.as_str();
    let b = q.nodes[1].var.as_str();
    // Both endpoints pinned by `<var>.<id_col> = '<lit>'` in WHERE.
    let pin = |var: &str| -> Option<&str> {
        q.where_preds
            .iter()
            .find(|p| p.var == var && p.prop == g.id_col && matches!(p.op, CmpOp::Eq))
            .map(|p| p.literal.as_str())
    };
    let (src, tgt) = match (pin(a), pin(b)) {
        (Some(s), Some(t)) => (s, t),
        _ => {
            return Err(ParseError(format!(
                "cypher: shortestPath requires both endpoints pinned by `{id}` equality \
                 (e.g. WHERE {a}.{id} = '…' AND {b}.{id} = '…')",
                id = g.id_col
            )));
        }
    };
    // Every WHERE predicate must be one of the two endpoint pins.
    let pins = q
        .where_preds
        .iter()
        .filter(|p| p.prop == g.id_col && matches!(p.op, CmpOp::Eq) && (p.var == a || p.var == b))
        .count();
    if q.where_preds.len() != pins {
        return Err(unsupported(
            "WHERE predicates other than the two endpoint id pins in a shortestPath query",
        ));
    }
    let max = q
        .rels
        .first()
        .and_then(|r| r.var_length.map(|(_, m)| m))
        .unwrap_or(DEFAULT_VAR_LENGTH_MAX);
    let dir_kw = match q.rels[0].direction {
        Direction::Forward => "forward",
        Direction::Backward => "backward",
        Direction::Undirected => "both",
    };
    // RETURN must be a single `length(<path>)`. The graph-shortest-path operator
    // yields the length in a `depth` column, which we project as-is. An `AS`
    // alias is rejected for now: KQL `project` is a bare column list (no `as`),
    // and `extend`-renaming does not compose with the operator's CTEs.
    if q.returns.len() != 1 {
        return Err(unsupported(
            "shortestPath RETURN must be a single length(<path>)",
        ));
    }
    match &q.returns[0] {
        ReturnItem::PathLength {
            path_var: pv,
            alias,
        } => {
            if pv != path_var {
                return Err(ParseError(format!(
                    "cypher: length() of unknown path `{pv}` (the path is `{path_var}`)"
                )));
            }
            if alias.is_some() {
                return Err(unsupported(
                    "AS alias on length(<path>) (the shortest-path length is returned in the `depth` column)",
                ));
            }
        }
        _ => {
            return Err(unsupported(
                "shortestPath RETURN supports only length(<path>)",
            ));
        }
    }
    Ok(format!(
        "{edges} | graph-shortest-path source {src} target {tgt} \
         from {srccol} to {dstcol} max-hops {max} direction {dir_kw} | project depth",
        edges = g.edge_table,
        srccol = g.src_col,
        dstcol = g.dst_col,
    ))
}

fn emit(q: &Query, g: &GraphBinding) -> Result<String, ParseError> {
    // shortestPath queries have their own lowering onto `graph-shortest-path`.
    if let Some(path_var) = &q.shortest_path {
        return emit_shortest_path(q, g, path_var);
    }

    // Validate that all referenced variables are one of the two node vars or
    // the relationship var. `graph-match` would reject unknown vars anyway, but
    // a Cypher-level message is friendlier.
    let known: Vec<&str> = q
        .nodes
        .iter()
        .map(|n| n.var.as_str())
        .chain(q.rels.iter().map(|r| r.var.as_str()))
        .collect();
    let check_var = |v: &str| -> Result<(), ParseError> {
        if known.iter().any(|k| *k == v) {
            Ok(())
        } else {
            Err(ParseError(format!(
                "cypher: unknown variable `{v}` (pattern declares {})",
                known.join(", ")
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
            ReturnItem::PathLength { .. } => {
                // `length(<path>)` is only meaningful in a shortestPath query,
                // which is lowered before reaching here.
                return Err(unsupported("length(<path>) outside a shortestPath query"));
            }
        }
    }

    // 2) Node-label filters need the label column projected.
    let mut label_filters: Vec<(String, String)> = Vec::new(); // (alias, label)
    for node in &q.nodes {
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

    // ---- project list ---- (shared by the graph-match and graph-var-match arms)
    let proj_list = projections
        .iter()
        .map(|p| format!("{}.{} as {}", p.var, p.col, p.alias))
        .collect::<Vec<_>>()
        .join(", ");

    let rel_pat = |r: &RelPat| match &r.rel_type {
        Some(t) => format!("[{}:{}]", r.var, t),
        None => format!("[{}]", r.var),
    };

    // A single variable-length hop lowers to `graph-var-match` (endpoint-pair
    // binding over a bounded recursive traversal); everything else is the
    // fixed-length `graph-match` chain.
    let mut kql = if q.rels.len() == 1 && q.rels[0].var_length.is_some() {
        let r = &q.rels[0];
        let (min, max) = r.var_length.unwrap();
        // `a` is the left node, `b` the right; the arrow direction drives the
        // traversal (no endpoint swap — graph-var-match takes `direction`).
        let a = &q.nodes[0].var;
        let b = &q.nodes[1].var;
        let dir_kw = match r.direction {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
            Direction::Undirected => "both",
        };
        format!(
            "{edges} | make-graph {src} --> {dst} with {nodes} on {id} \
             | graph-var-match ({a})-{rel}->({b}) min-hops {min} max-hops {max} \
             direction {dir_kw} project {proj}",
            edges = g.edge_table,
            src = g.src_col,
            dst = g.dst_col,
            nodes = g.node_table,
            id = g.id_col,
            rel = rel_pat(r),
            proj = proj_list,
        )
    } else {
        // Fixed-length chain. A single hop preserves the backward-swap so the
        // forward-only matcher is correct; a multi-hop pattern emits a forward
        // chain and rejects a backward (or variable-length) hop, which would
        // break the linear node order the matcher expects.
        let pattern = if q.rels.len() == 1 {
            let (left, right) = match q.rels[0].direction {
                Direction::Backward => (&q.nodes[1].var, &q.nodes[0].var),
                _ => (&q.nodes[0].var, &q.nodes[1].var),
            };
            format!("({left})-{}->({right})", rel_pat(&q.rels[0]))
        } else {
            let mut p = format!("({})", q.nodes[0].var);
            for (i, r) in q.rels.iter().enumerate() {
                if r.var_length.is_some() {
                    return Err(unsupported(
                        "variable-length relationship inside a multi-hop pattern",
                    ));
                }
                if r.direction == Direction::Backward {
                    return Err(unsupported(
                        "backward `<-` relationship in a multi-hop pattern",
                    ));
                }
                p.push_str(&format!("-{}->({})", rel_pat(r), q.nodes[i + 1].var));
            }
            p
        };
        format!(
            "{edges} | make-graph {src} --> {dst} with {nodes} on {id} \
             | graph-match {pattern} project {proj}",
            edges = g.edge_table,
            src = g.src_col,
            dst = g.dst_col,
            nodes = g.node_table,
            id = g.id_col,
            proj = proj_list,
        )
    };

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

    // ---- multi-pattern / multiple MATCH (chain continuation) ----------
    #[test]
    fn comma_pattern_continuing_the_chain_merges_into_one_match() {
        let kql = tr("MATCH (a)-[r]->(b), (b)-[s]->(c) RETURN a.name, c.name");
        assert!(
            kql.contains("graph-match (a)-[r]->(b)-[s]->(c)"),
            "comma-continued chain should merge: {kql}"
        );
        crate::kql_to_sql(&kql).expect("merged chain KQL must compile");
    }

    #[test]
    fn second_match_clause_continuing_the_chain_merges() {
        let kql = tr("MATCH (a)-[r]->(b) MATCH (b)-[s]->(c) RETURN a.name");
        assert!(kql.contains("graph-match (a)-[r]->(b)-[s]->(c)"), "{kql}");
    }

    #[test]
    fn multi_pattern_rejections() {
        // Non-chain (disjoint / star) multi-patterns need a join — unsupported.
        assert!(
            tr_err("MATCH (a)-[r]->(b), (c)-[s]->(d) RETURN a.name").contains("non-chain"),
            "disjoint multi-pattern should be rejected"
        );
        // OPTIONAL MATCH still unsupported.
        assert!(
            tr_err("MATCH (a)-[r]->(b) OPTIONAL MATCH (b)-[s]->(c) RETURN a.name")
                .contains("OPTIONAL"),
            "OPTIONAL MATCH should be rejected"
        );
    }

    // ---- variable-length relationships --------------------------------
    fn tr_err(cypher: &str) -> String {
        cypher_to_kql(cypher, &binding())
            .expect_err("should be rejected")
            .0
    }

    #[test]
    fn var_length_forward_lowers_to_graph_var_match() {
        let kql = tr("MATCH (a)-[r*1..3]->(b) RETURN a.name, b.name");
        assert!(
            kql.contains("graph-var-match (a)-[r]->(b) min-hops 1 max-hops 3 direction forward"),
            "{kql}"
        );
        assert!(
            kql.contains("project a.name as a_name, b.name as b_name"),
            "{kql}"
        );
        // still goes through make-graph so the operator can join node columns
        assert!(kql.contains("make-graph src --> dst with github_nodes on id"), "{kql}");
    }

    #[test]
    fn var_length_typed_and_directions() {
        // typed relationship is carried into the operator pattern
        let kql = tr("MATCH (a)-[r:CALLS*2..4]->(b) RETURN a.name");
        assert!(kql.contains("graph-var-match (a)-[r:CALLS]->(b) min-hops 2 max-hops 4 direction forward"), "{kql}");
        // backward: direction backward, no endpoint swap
        let back = tr("MATCH (a)<-[r*1..2]-(b) RETURN a.name, b.name");
        assert!(back.contains("graph-var-match (a)-[r]->(b) min-hops 1 max-hops 2 direction backward"), "{back}");
        // undirected: direction both
        let undir = tr("MATCH (a)-[r*1..2]-(b) RETURN a.name");
        assert!(undir.contains("direction both"), "{undir}");
    }

    #[test]
    fn var_length_bound_forms() {
        // bare `*` ⇒ 1..cap(10)
        assert!(tr("MATCH (a)-[r*]->(b) RETURN a.name").contains("min-hops 1 max-hops 10"));
        // `*N` ⇒ exactly N
        assert!(tr("MATCH (a)-[r*3]->(b) RETURN a.name").contains("min-hops 3 max-hops 3"));
        // `*..N` ⇒ 1..N
        assert!(tr("MATCH (a)-[r*..4]->(b) RETURN a.name").contains("min-hops 1 max-hops 4"));
        // `*M..` ⇒ M..cap
        assert!(tr("MATCH (a)-[r*2..]->(b) RETURN a.name").contains("min-hops 2 max-hops 10"));
    }

    // ---- shortestPath ------------------------------------------------
    #[test]
    fn shortest_path_lowers_to_graph_shortest_path() {
        let kql = tr(
            "MATCH p = shortestPath((a)-[r*1..5]->(b)) WHERE a.id = 'x' AND b.id = 'y' \
             RETURN length(p)",
        );
        assert!(
            kql.contains(
                "graph-shortest-path source 'x' target 'y' from src to dst max-hops 5 \
                 direction forward"
            ),
            "{kql}"
        );
        // Length is returned in the natural `depth` column.
        assert!(kql.contains("| project depth"), "{kql}");
        // The emitted KQL must actually compile to SQL (guards against invalid
        // operator syntax — e.g. `project depth as d`, which KQL rejects).
        crate::kql_to_sql(&kql).expect("shortestPath KQL must compile to SQL");
    }

    #[test]
    fn shortest_path_alias_directions_and_open_bound() {
        // Undirected ⇒ direction both; bare `*` ⇒ max-hops cap (10).
        let both = tr(
            "MATCH p = shortestPath((a)-[*]-(b)) WHERE a.id='x' AND b.id='y' RETURN length(p)",
        );
        assert!(
            both.contains("max-hops 10 direction both"),
            "{both}"
        );
        // Backward.
        let back = tr(
            "MATCH p = shortestPath((a)<-[*1..4]-(b)) WHERE a.id='x' AND b.id='y' RETURN length(p)",
        );
        assert!(back.contains("direction backward"), "{back}");
    }

    #[test]
    fn shortest_path_rejections() {
        // Missing one endpoint pin.
        assert!(
            tr_err("MATCH p = shortestPath((a)-[*]->(b)) WHERE a.id='x' RETURN length(p)")
                .contains("both endpoints pinned"),
            "expected endpoint-pin rejection"
        );
        // length() outside a shortestPath query.
        assert!(
            tr_err("MATCH (a)-[r]->(b) RETURN length(a)").contains("shortestPath"),
            "length() must require shortestPath"
        );
        // AS alias on length() is not supported yet.
        assert!(
            tr_err(
                "MATCH p = shortestPath((a)-[*]->(b)) WHERE a.id='x' AND b.id='y' \
                 RETURN length(p) AS d"
            )
            .contains("alias"),
            "AS alias on length() should be rejected"
        );
        // Extra WHERE predicate beyond the two pins.
        assert!(
            tr_err(
                "MATCH p = shortestPath((a)-[*]->(b)) WHERE a.id='x' AND b.id='y' AND a.name='z' \
                 RETURN length(p)"
            )
            .contains("WHERE"),
            "extra WHERE predicate should be rejected"
        );
    }

    #[test]
    fn var_length_rejections() {
        // Projecting the relationship var is rejected by the graph-var-match
        // operator (a path has no edge columns) — surfaces at KQL→SQL, since
        // cypher_to_kql happily emits the project list.
        let kql = tr("MATCH (a)-[r*1..3]->(b) RETURN r.type");
        let err = crate::kql_to_sql(&kql).expect_err("r.* in a var-length match must be rejected");
        assert!(err.0.contains("no columns"), "{}", err.0);
        // var-length inside a multi-hop pattern is unsupported (cypher lowering).
        assert!(
            tr_err("MATCH (a)-[r*1..2]->(b)-[s]->(c) RETURN a.name").contains("multi-hop"),
            "expected multi-hop rejection"
        );
        // min > max (cypher parse).
        assert!(tr_err("MATCH (a)-[r*3..1]->(b) RETURN a.name").contains("min"));
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

    // ---- multi-hop (follow-up): forward N-hop chains ------------------
    #[test]
    fn multi_hop_two_hops_translates_and_compiles() {
        let kql = tr("MATCH (a)-[r1:CALLS]->(b)-[r2:DEPENDS]->(c) RETURN a.name, c.name");
        assert!(
            kql.contains("graph-match (a)-[r1:CALLS]->(b)-[r2:DEPENDS]->(c)"),
            "forward 2-hop chain: {kql}"
        );
        assert!(
            kql.contains("a.name as a_name") && kql.contains("c.name as c_name"),
            "endpoint projections: {kql}"
        );
        assert!(
            kql_to_sql_with_schemas(&kql, &SchemaMap::default()).is_ok(),
            "2-hop must compile through to SQL: {kql}"
        );
    }

    #[test]
    fn multi_hop_three_hops_with_where_on_middle_compiles() {
        let kql =
            tr("MATCH (a)-[r1]->(b)-[r2]->(c)-[r3]->(d) WHERE b.kind = 'svc' RETURN a.name, d.name");
        assert!(
            kql.contains("graph-match (a)-[r1]->(b)-[r2]->(c)-[r3]->(d)"),
            "forward 3-hop chain: {kql}"
        );
        assert!(
            kql.contains("where b_kind == 'svc'"),
            "WHERE on a middle node folds into a flat filter: {kql}"
        );
        assert!(
            kql_to_sql_with_schemas(&kql, &SchemaMap::default()).is_ok(),
            "3-hop must compile through to SQL: {kql}"
        );
    }

    #[test]
    fn multi_hop_backward_hop_rejected() {
        // A backward hop inside a multi-hop chain breaks the linear node order
        // the forward-only matcher needs — v1 rejects it with a clear message.
        let err = cypher_to_kql("MATCH (a)-[r1]->(b)<-[r2]-(c) RETURN a.name", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("backward") && msg.contains("multi-hop"), "{msg}");
    }

    // ---- unsupported constructs → Err ---------------------------------

    #[test]
    fn create_rejected() {
        let err = cypher_to_kql("CREATE (a:Foo) RETURN a", &binding());
        let msg = err.unwrap_err().0;
        assert!(msg.contains("CREATE"), "{msg}");
        assert!(msg.contains("not supported yet"), "{msg}");
    }

    #[test]
    fn variable_length_now_supported() {
        // Regression guard: a single var-length hop used to be rejected; it now
        // lowers to graph-var-match (see var_length_* tests above).
        let kql = cypher_to_kql("MATCH (a)-[r*1..3]->(b) RETURN a.name", &binding())
            .expect("single var-length hop is supported");
        assert!(kql.contains("graph-var-match"), "{kql}");
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
