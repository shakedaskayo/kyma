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
//! Variable-length inside a multi-hop chain, `WITH`, `ORDER BY`, aggregation,
//! `CREATE/SET/DELETE/MERGE`, `RETURN *`. (Backward hops inside a multi-hop
//! chain and `OPTIONAL MATCH` ARE supported — see below.)
//!
//! Multiple patterns — comma-separated or in successive `MATCH` clauses — ARE
//! supported, including non-linear (star / tree / cyclic) and disjoint shapes:
//! a continuation that extends the chain merges into it (`(a)-[r]->(b),
//! (b)-[s]->(c)` ≡ `(a)-[r]->(b)-[s]->(c)`), while any other segment lowers to a
//! general multi-segment `graph-match` join over the shared variables (a
//! segment sharing nothing is a cartesian product). Variable-length is
//! single-pattern only.
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
    /// Already SQL/KQL-ready RHS text. For scalar ops this is a single literal
    /// (string literals single-quoted); for `In` it is a parenthesized list
    /// `(v1, v2, …)` so emission produces a valid KQL `col in (…)` expression.
    literal: String,
}

/// A boolean WHERE expression tree, used only for mixed-precedence predicates
/// (parens / mixed AND-OR / NOT). Leaves are [`Comparison`]s; `And`/`Or` are
/// n-ary. Rendered to a single parenthesized KQL `where` expression — KQL's
/// own `or`/`and`/`not` parser then re-establishes precedence from the parens.
#[derive(Debug, Clone)]
enum WhereExpr {
    Cmp(Comparison),
    Not(Box<WhereExpr>),
    And(Vec<WhereExpr>),
    Or(Vec<WhereExpr>),
}

impl WhereExpr {
    /// Render to a KQL boolean expression over the flat `{var}_{prop}` aliases.
    fn render_kql(&self) -> String {
        match self {
            WhereExpr::Cmp(c) => c.op.render_clause(&format!("{}_{}", c.var, c.prop), &c.literal),
            WhereExpr::Not(inner) => format!("not ({})", inner.render_kql()),
            WhereExpr::And(terms) => {
                let parts: Vec<String> = terms.iter().map(WhereExpr::render_kql).collect();
                format!("({})", parts.join(" and "))
            }
            WhereExpr::Or(terms) => {
                let parts: Vec<String> = terms.iter().map(WhereExpr::render_kql).collect();
                format!("({})", parts.join(" or "))
            }
        }
    }

    /// Collect every leaf comparison's `(var, prop)` for projection.
    fn collect_leaves<'a>(&'a self, out: &mut Vec<(&'a str, &'a str)>) {
        match self {
            WhereExpr::Cmp(c) => out.push((c.var.as_str(), c.prop.as_str())),
            WhereExpr::Not(inner) => inner.collect_leaves(out),
            WhereExpr::And(terms) | WhereExpr::Or(terms) => {
                for t in terms {
                    t.collect_leaves(out);
                }
            }
        }
    }
}

/// If `e` is a flat single-connective conjunction/disjunction of plain
/// comparisons (no NOT, no nesting), return the flat predicate list + whether
/// it is OR-joined. Otherwise `None` (the caller keeps the tree). This is what
/// lets the common case reuse the unchanged flat lowering + shortestPath pins.
fn try_flatten_where(e: &WhereExpr) -> Option<(Vec<Comparison>, bool)> {
    match e {
        WhereExpr::Cmp(c) => Some((vec![c.clone()], false)),
        WhereExpr::And(terms) | WhereExpr::Or(terms) => {
            let or = matches!(e, WhereExpr::Or(_));
            let mut preds = Vec::with_capacity(terms.len());
            for t in terms {
                match t {
                    WhereExpr::Cmp(c) => preds.push(c.clone()),
                    _ => return None,
                }
            }
            Some((preds, or))
        }
        WhereExpr::Not(_) => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `n.prop IN [a, b, c]` — set membership; the RHS literal is a list.
    In,
    /// `n.prop STARTS WITH 'p'` — prefix match.
    StartsWith,
    /// `n.prop ENDS WITH 's'` — suffix match.
    EndsWith,
    /// `n.prop CONTAINS 's'` — substring match.
    Contains,
    /// `n.prop IS NULL` — null test (no RHS literal).
    IsNull,
    /// `n.prop IS NOT NULL` — non-null test (no RHS literal).
    IsNotNull,
}

impl CmpOp {
    /// KQL operator text for the infix operators. `=`→`==`, `<>`→`!=`; the rest
    /// pass through. `In` → KQL `in`; the string ops → `startswith`/`endswith`/
    /// `contains`. The null ops are unary functions, not infix — see
    /// [`CmpOp::render_clause`] — so their text here is only for completeness.
    fn kql(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
            CmpOp::In => "in",
            CmpOp::StartsWith => "startswith",
            CmpOp::EndsWith => "endswith",
            CmpOp::Contains => "contains",
            CmpOp::IsNull => "isnull",
            CmpOp::IsNotNull => "isnotnull",
        }
    }

    /// Render the full KQL predicate over a flat `{var}_{prop}` `alias`. Most
    /// ops are infix (`alias op literal`); the null ops lower to the KQL
    /// `isnull(...)` / `isnotnull(...)` functions (no RHS), which the KQL→SQL
    /// stage turns into SQL `IS [NOT] NULL`.
    fn render_clause(self, alias: &str, literal: &str) -> String {
        match self {
            CmpOp::IsNull => format!("isnull({alias})"),
            CmpOp::IsNotNull => format!("isnotnull({alias})"),
            _ => format!("{alias} {} {literal}", self.kql()),
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
    /// An aggregate `RETURN count(*)|count(<v>)|sum(<v>.<p>)|avg|min|max(…)
    /// [AS <alias>]`. Other RETURN items become GROUP BY keys.
    Aggregate {
        func: String,
        target: AggTarget,
        alias: Option<String>,
    },
}

/// The argument of an aggregate: `count(*)`, `count(<var>)`, or
/// `<func>(<var>.<prop>)`.
#[derive(Debug, Clone)]
enum AggTarget {
    Star,
    Var(String),
    Prop(String, String),
}

#[derive(Debug, Clone)]
struct Query {
    /// Pattern node chain `n0, n1, … nN` (length = `rels.len() + 1`).
    nodes: Vec<NodePat>,
    /// Pattern relationships `r0 … r{N-1}`; `rels[i]` connects `nodes[i]`→`nodes[i+1]`.
    rels: Vec<RelPat>,
    /// Flat WHERE: a single connective (all-AND or all-OR) of plain
    /// comparisons. Populated for the common case so the existing lowering +
    /// the shortestPath endpoint-pin extraction keep working unchanged. Empty
    /// when the WHERE needs the boolean tree (`where_tree`) instead.
    where_preds: Vec<Comparison>,
    /// `true` ⇒ the flat WHERE comparisons are OR-joined; `false` ⇒ AND-joined.
    where_or: bool,
    /// Mixed-precedence WHERE (parentheses, mixed AND/OR, or NOT) that cannot be
    /// expressed as a flat single connective. When `Some`, it drives a single
    /// `| where (<rendered>)` and `where_preds` is empty. KQL's own
    /// precedence-correct `where` parser handles the rendered parens.
    where_tree: Option<WhereExpr>,
    returns: Vec<ReturnItem>,
    limit: Option<i64>,
    /// `Some(path_var)` when the MATCH is `<path_var> = shortestPath((a)-[…]->(b))`.
    /// The single hop in `nodes`/`rels` is the wrapped pattern; the lowering
    /// emits `graph-shortest-path` instead of `graph-match`.
    shortest_path: Option<String>,
    /// Additional pattern segments that don't continue the primary chain (star /
    /// tree / disjoint), each `(nodes, rels, optional)`. `optional` marks an
    /// `OPTIONAL MATCH` segment (lowered as a LEFT JOIN). Empty for a single
    /// linear pattern; non-empty ⇒ a multi-segment `graph-match` join.
    extra_segments: Vec<(Vec<NodePat>, Vec<RelPat>, bool)>,
    /// `RETURN DISTINCT` → a trailing `| distinct`.
    distinct: bool,
    /// `ORDER BY <var>.<prop> [ASC|DESC]` items (`desc` true ⇒ descending) →
    /// a trailing `| sort by …`.
    order_by: Vec<(String, String, bool)>,
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

        // Additional patterns — via `,` or a subsequent `MATCH` clause. A
        // continuation that extends the primary chain (its first node is the
        // primary's tail) merges into it (keeping pure chains on the efficient
        // single-segment path, e.g. `(a)-[r]->(b), (b)-[s]->(c)` ≡ the chain
        // `(a)-[r]->(b)-[s]->(c)`); any other segment (star / tree / disjoint)
        // is kept separate and lowered via a general multi-segment join.
        // shortestPath is single-pattern, so this only applies to plain MATCH.
        let mut extra_segments: Vec<(Vec<NodePat>, Vec<RelPat>, bool)> = Vec::new();
        if shortest_path.is_none() {
            // `clause_optional` tracks whether the current clause is an
            // `OPTIONAL MATCH` (LEFT JOIN); a `,` continues the current clause.
            let mut clause_optional = false;
            loop {
                let cont = if self.eat(&Tok::Comma) {
                    true
                } else if self.peek_kw("OPTIONAL") {
                    self.eat_kw("OPTIONAL");
                    if !self.eat_kw("MATCH") {
                        return Err(ParseError(
                            "cypher: expected MATCH after OPTIONAL".to_string(),
                        ));
                    }
                    clause_optional = true;
                    true
                } else if self.peek_kw("MATCH") {
                    self.eat_kw("MATCH");
                    clause_optional = false;
                    true
                } else {
                    false
                };
                if !cont {
                    break;
                }
                // Parse this segment fully: node ( rel node )+.
                let mut seg_nodes = vec![self.parse_node()?];
                let mut seg_rels: Vec<RelPat> = Vec::new();
                loop {
                    seg_rels.push(self.parse_rel()?);
                    seg_nodes.push(self.parse_node()?);
                    if !matches!(self.peek(), Some(Tok::Dash | Tok::ArrowL)) {
                        break;
                    }
                }
                let primary_tail = nodes.last().map(|n| n.var.clone()).unwrap_or_default();
                if extra_segments.is_empty() && !clause_optional && seg_nodes[0].var == primary_tail
                {
                    // Mandatory continuation of the primary chain → merge.
                    rels.extend(seg_rels);
                    nodes.extend(seg_nodes.into_iter().skip(1));
                } else {
                    extra_segments.push((seg_nodes, seg_rels, clause_optional));
                }
            }
        }

        // A trailing MATCH/OPTIONAL after shortestPath, or WITH ⇒ unsupported.
        if self.peek_kw("OPTIONAL") || self.peek_kw("MATCH") {
            return Err(unsupported("MATCH / OPTIONAL MATCH after shortestPath"));
        }
        if self.peek_kw("WITH") {
            return Err(unsupported("WITH"));
        }

        let (where_preds, where_or, where_tree) = if self.eat_kw("WHERE") {
            let tree = self.parse_where_expr()?;
            match try_flatten_where(&tree) {
                // Flat single-connective of plain comparisons → existing lowering.
                Some((preds, or)) => (preds, or, None),
                // Parens / mixed AND-OR / NOT → drive the tree lowering instead.
                None => (Vec::new(), false, Some(tree)),
            }
        } else {
            (Vec::new(), false, None)
        };

        if !self.eat_kw("RETURN") {
            return Err(ParseError(
                "cypher: expected RETURN".to_string(),
            ));
        }
        let distinct = self.eat_kw("DISTINCT");
        let returns = self.parse_return()?;

        // ORDER BY <var>.<prop> [ASC|DESC], …
        let mut order_by: Vec<(String, String, bool)> = Vec::new();
        if self.eat_kw("ORDER") {
            if !self.eat_kw("BY") {
                return Err(ParseError("cypher: expected BY after ORDER".to_string()));
            }
            loop {
                let var = self.expect_word()?;
                if !self.eat(&Tok::Dot) {
                    return Err(ParseError(
                        "cypher: ORDER BY expects `<var>.<prop>`".to_string(),
                    ));
                }
                let prop = self.expect_word()?;
                let desc = if self.eat_kw("DESC") {
                    true
                } else {
                    self.eat_kw("ASC");
                    false
                };
                order_by.push((var, prop, desc));
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
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
            where_or,
            where_tree,
            returns,
            limit,
            shortest_path,
            extra_segments,
            distinct,
            order_by,
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

    /// Parse a full boolean WHERE expression with standard precedence
    /// (`OR` < `AND` < `NOT` < atom), where an atom is a comparison or a
    /// parenthesized sub-expression. The caller flattens the common
    /// single-connective case back into `where_preds` via [`try_flatten_where`];
    /// genuinely mixed/parenthesized/negated expressions keep the tree.
    fn parse_where_expr(&mut self) -> Result<WhereExpr, ParseError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<WhereExpr, ParseError> {
        let mut terms = vec![self.parse_and_expr()?];
        while self.eat_kw("OR") {
            terms.push(self.parse_and_expr()?);
        }
        Ok(if terms.len() == 1 {
            terms.pop().unwrap()
        } else {
            WhereExpr::Or(terms)
        })
    }

    fn parse_and_expr(&mut self) -> Result<WhereExpr, ParseError> {
        let mut terms = vec![self.parse_not_expr()?];
        while self.eat_kw("AND") {
            terms.push(self.parse_not_expr()?);
        }
        Ok(if terms.len() == 1 {
            terms.pop().unwrap()
        } else {
            WhereExpr::And(terms)
        })
    }

    fn parse_not_expr(&mut self) -> Result<WhereExpr, ParseError> {
        if self.eat_kw("NOT") {
            return Ok(WhereExpr::Not(Box::new(self.parse_not_expr()?)));
        }
        self.parse_atom_expr()
    }

    fn parse_atom_expr(&mut self) -> Result<WhereExpr, ParseError> {
        if self.eat(&Tok::LParen) {
            let inner = self.parse_or_expr()?;
            if !self.eat(&Tok::RParen) {
                return Err(ParseError(
                    "cypher: expected `)` to close a WHERE group".to_string(),
                ));
            }
            return Ok(inner);
        }
        Ok(WhereExpr::Cmp(self.parse_comparison()?))
    }

    fn parse_comparison(&mut self) -> Result<Comparison, ParseError> {
        let var = self.expect_word()?;
        if !self.eat(&Tok::Dot) {
            return Err(ParseError(
                "cypher: WHERE predicate must be `<var>.<prop> <op> <literal>`".to_string(),
            ));
        }
        let prop = self.expect_word()?;
        // `<var>.<prop> IN [a, b, c]` — set membership over a literal list.
        if self.eat_kw("IN") {
            let literal = self.parse_in_list()?;
            return Ok(Comparison {
                var,
                prop,
                op: CmpOp::In,
                literal,
            });
        }
        // String operators: `STARTS WITH` / `ENDS WITH` / `CONTAINS 'lit'`.
        if self.eat_kw("STARTS") {
            if !self.eat_kw("WITH") {
                return Err(ParseError(
                    "cypher: expected WITH after STARTS".to_string(),
                ));
            }
            let literal = self.parse_literal()?;
            return Ok(Comparison {
                var,
                prop,
                op: CmpOp::StartsWith,
                literal,
            });
        }
        if self.eat_kw("ENDS") {
            if !self.eat_kw("WITH") {
                return Err(ParseError("cypher: expected WITH after ENDS".to_string()));
            }
            let literal = self.parse_literal()?;
            return Ok(Comparison {
                var,
                prop,
                op: CmpOp::EndsWith,
                literal,
            });
        }
        if self.eat_kw("CONTAINS") {
            let literal = self.parse_literal()?;
            return Ok(Comparison {
                var,
                prop,
                op: CmpOp::Contains,
                literal,
            });
        }
        // `<var>.<prop> IS [NOT] NULL` — null test (no RHS).
        if self.eat_kw("IS") {
            let negated = self.eat_kw("NOT");
            if !self.eat_kw("NULL") {
                return Err(ParseError(
                    "cypher: expected NULL after IS [NOT]".to_string(),
                ));
            }
            return Ok(Comparison {
                var,
                prop,
                op: if negated {
                    CmpOp::IsNotNull
                } else {
                    CmpOp::IsNull
                },
                literal: String::new(),
            });
        }
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

    /// Parse `[lit, lit, …]` after `IN`, returning a KQL-ready parenthesized
    /// list `(lit, lit, …)`. An empty list is a parse error (the first
    /// `parse_literal` rejects `]`).
    fn parse_in_list(&mut self) -> Result<String, ParseError> {
        if !self.eat(&Tok::LBracket) {
            return Err(ParseError(
                "cypher: expected `[` after IN, e.g. n.x IN [1, 2, 3]".to_string(),
            ));
        }
        let mut items = Vec::new();
        loop {
            items.push(self.parse_literal()?);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        if !self.eat(&Tok::RBracket) {
            return Err(ParseError(
                "cypher: expected `]` to close the IN list".to_string(),
            ));
        }
        Ok(format!("({})", items.join(", ")))
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
        // `RETURN DISTINCT` is consumed by the caller (parse_query) before this.

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
            // Aggregates: count(*) | count(<v>) | <func>(<v>.<p>) for
            // count/sum/avg/min/max, plus collect(<v>.<p>) → list aggregation.
            let lf = var.to_ascii_lowercase();
            if matches!(lf.as_str(), "count" | "sum" | "avg" | "min" | "max" | "collect")
                && matches!(self.peek(), Some(Tok::LParen))
            {
                self.eat(&Tok::LParen);
                let target = if self.eat(&Tok::Star) {
                    AggTarget::Star
                } else {
                    let v = self.expect_word()?;
                    if self.eat(&Tok::Dot) {
                        AggTarget::Prop(v, self.expect_word()?)
                    } else {
                        AggTarget::Var(v)
                    }
                };
                if !self.eat(&Tok::RParen) {
                    return Err(ParseError(format!("cypher: expected `)` after {lf}(…)")));
                }
                // sum/avg/min/max need a column; only count takes `*` or a bare var.
                if lf != "count" && !matches!(target, AggTarget::Prop(_, _)) {
                    return Err(unsupported(&format!(
                        "{lf}(…) requires a property argument, e.g. {lf}(a.weight)"
                    )));
                }
                let alias = if self.eat_kw("AS") {
                    Some(self.expect_word()?)
                } else {
                    None
                };
                items.push(ReturnItem::Aggregate {
                    func: lf,
                    target,
                    alias,
                });
                if !self.eat(&Tok::Comma) {
                    break;
                }
                continue;
            }
            // Reject other aggregation / function calls like collect(...).
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
    // The endpoint pins are AND-ed id equalities; an OR-joined WHERE can't pin
    // both endpoints.
    if q.where_or {
        return Err(unsupported("OR in WHERE with shortestPath"));
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

/// Render one pattern segment as a forward `graph-match` chain `(n0)-[e0]->(n1)…`.
/// A single backward hop swaps endpoints (the matcher is forward-only); backward
/// inside a multi-hop segment, and any variable-length hop, are rejected (the
/// multi-segment join is fixed-length).
fn segment_pattern(nodes: &[NodePat], rels: &[RelPat]) -> Result<String, ParseError> {
    let rel_pat = |r: &RelPat| match &r.rel_type {
        Some(t) => format!("[{}:{}]", r.var, t),
        None => format!("[{}]", r.var),
    };
    if rels.iter().any(|r| r.var_length.is_some()) {
        return Err(unsupported(
            "variable-length relationship in a multi-pattern MATCH",
        ));
    }
    if rels.len() == 1 {
        let (left, right) = match rels[0].direction {
            Direction::Backward => (&nodes[1].var, &nodes[0].var),
            _ => (&nodes[0].var, &nodes[1].var),
        };
        Ok(format!("({left})-{}->({right})", rel_pat(&rels[0])))
    } else {
        let mut p = format!("({})", nodes[0].var);
        for (i, r) in rels.iter().enumerate() {
            if r.direction == Direction::Backward {
                return Err(unsupported(
                    "backward `<-` relationship in a multi-hop segment",
                ));
            }
            p.push_str(&format!("-{}->({})", rel_pat(r), nodes[i + 1].var));
        }
        Ok(p)
    }
}

/// One or more `graph-match` segment patterns for `(nodes, rels)`. A forward
/// (or single-hop) chain is one segment; a multi-hop chain containing a backward
/// hop is decomposed into single-hop forward segments (each backward hop swapped
/// by [`segment_pattern`]) that the general join re-stitches via shared vars —
/// so `(a)-[r]->(b)<-[s]-(c)` becomes `(a)-[r]->(b), (c)-[s]->(b)`.
fn segment_patterns(nodes: &[NodePat], rels: &[RelPat]) -> Result<Vec<String>, ParseError> {
    if rels.len() > 1 && rels.iter().any(|r| r.direction == Direction::Backward) {
        let mut out = Vec::with_capacity(rels.len());
        for i in 0..rels.len() {
            // One hop: nodes[i]→nodes[i+1] joined by the single rel rels[i].
            out.push(segment_pattern(&nodes[i..=i + 1], &rels[i..=i])?);
        }
        Ok(out)
    } else {
        Ok(vec![segment_pattern(nodes, rels)?])
    }
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
        .chain(q.extra_segments.iter().flat_map(|(ns, rs, _)| {
            ns.iter()
                .map(|n| n.var.as_str())
                .chain(rs.iter().map(|r| r.var.as_str()))
        }))
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
    //    `count(...)` items become summarize aggregates; the other items become
    //    both projected columns and the GROUP BY keys.
    let mut aggregates: Vec<String> = Vec::new();
    let mut group_keys: Vec<String> = Vec::new();
    for item in &q.returns {
        match item {
            ReturnItem::Prop { var, prop, alias } => {
                check_var(var)?;
                let out = alias.clone().unwrap_or_else(|| format!("{var}_{prop}"));
                push_proj(var, prop, out.clone());
                group_keys.push(out);
            }
            ReturnItem::Var { var } => {
                check_var(var)?;
                // Bare var → id + label columns.
                push_proj(var, &g.id_col, format!("{var}_id"));
                push_proj(var, &g.label_col, format!("{var}_label"));
                group_keys.push(format!("{var}_id"));
                group_keys.push(format!("{var}_label"));
            }
            ReturnItem::Aggregate { func, target, alias } => {
                let out = alias.clone().unwrap_or_else(|| func.clone());
                // Cypher `collect` → KQL `make_list` (→ SQL `array_agg`); the
                // rest share their name with the KQL/SQL aggregate.
                let kfn = if func == "collect" { "make_list" } else { func.as_str() };
                let expr = match target {
                    AggTarget::Star => format!("{kfn}(*)"),
                    AggTarget::Var(v) => {
                        check_var(v)?;
                        let a = format!("{v}_id");
                        push_proj(v, &g.id_col, a.clone());
                        format!("{kfn}({a})")
                    }
                    AggTarget::Prop(v, p) => {
                        check_var(v)?;
                        let a = format!("{v}_{p}");
                        push_proj(v, p, a.clone());
                        format!("{kfn}({a})")
                    }
                };
                aggregates.push(format!("{out} = {expr}"));
            }
            ReturnItem::PathLength { .. } => {
                // `length(<path>)` is only meaningful in a shortestPath query,
                // which is lowered before reaching here.
                return Err(unsupported("length(<path>) outside a shortestPath query"));
            }
        }
    }

    // 2) Node-label filters need the label column projected (across all segments).
    let mut label_filters: Vec<(String, String)> = Vec::new(); // (alias, label)
    let all_nodes = q
        .nodes
        .iter()
        .chain(q.extra_segments.iter().flat_map(|(ns, _, _)| ns.iter()));
    for node in all_nodes {
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
        where_clauses.push(pred.op.render_clause(&alias, &pred.literal));
    }
    // Mixed-precedence WHERE: project every leaf prop, then render the tree to
    // one parenthesized clause (KQL re-derives precedence from the parens).
    if let Some(tree) = &q.where_tree {
        let mut leaves = Vec::new();
        tree.collect_leaves(&mut leaves);
        for (var, prop) in leaves {
            check_var(var)?;
            push_proj(var, prop, format!("{var}_{prop}"));
        }
        where_clauses.push(tree.render_kql());
    }

    // 4) ORDER BY props need their column projected; collect the sort items.
    let mut sort_items: Vec<String> = Vec::new();
    for (var, prop, desc) in &q.order_by {
        check_var(var)?;
        let alias = format!("{var}_{prop}");
        push_proj(var, prop, alias.clone());
        sort_items.push(format!("{alias} {}", if *desc { "desc" } else { "asc" }));
    }

    if projections.is_empty() {
        if aggregates.is_empty() {
            return Err(ParseError(
                "cypher: RETURN must project at least one column".to_string(),
            ));
        }
        // `count(*)` with no group keys: the graph-match still needs a column,
        // so project the first node's id (the summarize ignores it).
        projections.push(Projection {
            var: q.nodes[0].var.clone(),
            col: g.id_col.clone(),
            alias: format!("{}_id", q.nodes[0].var),
        });
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

    // Multi-segment (star / tree / disjoint) patterns lower to a general
    // `graph-match` over comma-separated segments; a single variable-length hop
    // to `graph-var-match`; everything else to the fixed-length linear chain.
    // A backward hop inside a multi-hop chain is also lowered via the general
    // join (the linear matcher is forward-only), by decomposing into single-hop
    // segments.
    let primary_backward_multihop =
        q.rels.len() > 1 && q.rels.iter().any(|r| r.direction == Direction::Backward);
    let mut kql = if !q.extra_segments.is_empty() || primary_backward_multihop {
        let mut segs = segment_patterns(&q.nodes, &q.rels)?;
        for (sn, sr, opt) in &q.extra_segments {
            for p in segment_patterns(sn, sr)? {
                segs.push(if *opt { format!("OPTIONAL {p}") } else { p });
            }
        }
        format!(
            "{edges} | make-graph {src} --> {dst} with {nodes} on {id} \
             | graph-match {pattern} project {proj}",
            edges = g.edge_table,
            src = g.src_col,
            dst = g.dst_col,
            nodes = g.node_table,
            id = g.id_col,
            pattern = segs.join(", "),
            proj = proj_list,
        )
    } else if q.rels.len() == 1 && q.rels[0].var_length.is_some() {
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
    // Label filters are always AND (one `| where` each). WHERE comparisons are
    // OR-joined into a single parenthesized clause when `where_or`, else AND.
    let mut filters: Vec<String> = Vec::new();
    for (alias, label) in &label_filters {
        filters.push(format!("{} == '{}'", alias, label.replace('\'', "''")));
    }
    if !where_clauses.is_empty() {
        if q.where_or {
            filters.push(format!("({})", where_clauses.join(" or ")));
        } else {
            filters.extend(where_clauses);
        }
    }
    for f in filters {
        kql.push_str(&format!(" | where {f}"));
    }

    // count(...) aggregates run after the row filters: group the non-aggregate
    // RETURN items, count per group (or globally when there are no group keys).
    if !aggregates.is_empty() {
        if group_keys.is_empty() {
            kql.push_str(&format!(" | summarize {}", aggregates.join(", ")));
        } else {
            kql.push_str(&format!(
                " | summarize {} by {}",
                aggregates.join(", "),
                group_keys.join(", ")
            ));
        }
    }

    // RETURN DISTINCT → distinct rows; ORDER BY → sort; LIMIT → take. Distinct
    // before sort so the ordering applies to the de-duplicated rows.
    if q.distinct {
        kql.push_str(" | distinct");
    }
    if !sort_items.is_empty() {
        kql.push_str(&format!(" | sort by {}", sort_items.join(", ")));
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
    fn star_multi_pattern_lowers_to_multi_segment_graph_match() {
        // Shared START var `a` (not a chain) → multi-segment graph-match.
        let kql = tr("MATCH (a)-[r]->(b), (a)-[s]->(c) RETURN a.name, b.name, c.name");
        assert!(
            kql.contains("graph-match (a)-[r]->(b), (a)-[s]->(c)"),
            "star → comma-separated segments: {kql}"
        );
        crate::kql_to_sql(&kql).expect("star multi-pattern KQL must compile");
        // Disjoint patterns are a valid (cartesian) multi-segment match too.
        let disj = tr("MATCH (a)-[r]->(b), (c)-[s]->(d) RETURN a.name, d.name");
        assert!(disj.contains("graph-match (a)-[r]->(b), (c)-[s]->(d)"), "{disj}");
        crate::kql_to_sql(&disj).expect("disjoint multi-pattern KQL must compile");
    }

    #[test]
    fn optional_match_lowers_to_left_join_segment() {
        // OPTIONAL MATCH → a LEFT-joined segment in the multi-segment graph-match.
        let kql = tr("MATCH (a)-[r]->(b) OPTIONAL MATCH (b)-[s]->(c) RETURN a.name, c.name");
        assert!(
            kql.contains("graph-match (a)-[r]->(b), OPTIONAL (b)-[s]->(c)"),
            "OPTIONAL segment marked: {kql}"
        );
        let sql = crate::kql_to_sql(&kql).expect("OPTIONAL KQL must compile");
        assert!(sql.contains("LEFT JOIN"), "optional segment is a LEFT JOIN: {sql}");
    }

    #[test]
    fn backward_hop_in_multihop_decomposes_to_general_join() {
        // `(a)-[r]->(b)<-[s]-(c)` (mixed direction) → single-hop segments joined
        // on the shared `b` (the backward hop swapped to forward `(c)-[s]->(b)`).
        let kql = tr("MATCH (a)-[r]->(b)<-[s]-(c) RETURN a.name, c.name");
        assert!(
            kql.contains("graph-match (a)-[r]->(b), (c)-[s]->(b)"),
            "mixed-direction chain decomposes: {kql}"
        );
        crate::kql_to_sql(&kql).expect("decomposed mixed-direction KQL must compile");
    }

    #[test]
    fn multi_pattern_rejections() {
        // Variable-length inside a multi-segment pattern is unsupported.
        assert!(
            tr_err("MATCH (a)-[r*1..2]->(b), (a)-[s]->(c) RETURN a.name")
                .contains("variable-length"),
            "var-length in a multi-segment pattern should be rejected"
        );
        // OPTIONAL without MATCH is a parse error.
        assert!(
            tr_err("MATCH (a)-[r]->(b) OPTIONAL (b)-[s]->(c) RETURN a.name")
                .contains("MATCH"),
            "OPTIONAL must be followed by MATCH"
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

    #[test]
    fn where_in_list_lowers_to_kql_in_and_compiles() {
        // String list.
        let kql = tr("MATCH (a)-[r]->(b) WHERE a.kind IN ['x', 'y', 'z'] RETURN a.kind");
        assert!(kql.contains("| where a_kind in ('x', 'y', 'z')"), "{kql}");
        // The lowered KQL must compile all the way to SQL `IN (…)` — the
        // string-shape check alone misses invalid-KQL / invalid-plan bugs.
        let sql = crate::kql_to_sql(&kql).expect("IN-list KQL must compile to SQL");
        assert!(sql.to_uppercase().contains(" IN ("), "{sql}");
        // Numeric list.
        let kqln = tr("MATCH (a)-[r]->(b) WHERE b.n IN [1, 2, 3] RETURN b.n");
        assert!(kqln.contains("| where b_n in (1, 2, 3)"), "{kqln}");
        crate::kql_to_sql(&kqln).expect("numeric IN-list KQL must compile to SQL");
    }

    #[test]
    fn where_in_list_combines_with_and() {
        let kql =
            tr("MATCH (a)-[r]->(b) WHERE a.kind IN ['x', 'y'] AND b.n > 3 RETURN a.kind, b.n");
        assert!(kql.contains("a_kind in ('x', 'y')"), "{kql}");
        assert!(kql.contains("b_n > 3"), "{kql}");
        crate::kql_to_sql(&kql).expect("IN + AND KQL must compile to SQL");
    }

    #[test]
    fn where_empty_in_list_is_rejected() {
        let g = binding();
        assert!(
            cypher_to_kql("MATCH (a)-[r]->(b) WHERE a.kind IN [] RETURN a.kind", &g).is_err(),
            "empty IN list must be a parse error"
        );
    }

    #[test]
    fn where_string_operators_lower_to_kql_and_compile() {
        // STARTS WITH → KQL startswith → SQL LIKE 'p%'.
        let sw = tr("MATCH (a)-[r]->(b) WHERE a.name STARTS WITH 'pre' RETURN a.name");
        assert!(sw.contains("| where a_name startswith 'pre'"), "{sw}");
        let sql = crate::kql_to_sql(&sw).expect("STARTS WITH KQL must compile to SQL");
        assert!(sql.to_uppercase().contains("LIKE"), "{sql}");
        // ENDS WITH → endswith → LIKE '%s'.
        let ew = tr("MATCH (a)-[r]->(b) WHERE a.name ENDS WITH 'suf' RETURN a.name");
        assert!(ew.contains("| where a_name endswith 'suf'"), "{ew}");
        crate::kql_to_sql(&ew).expect("ENDS WITH KQL must compile to SQL");
        // CONTAINS → contains → LIKE '%sub%'.
        let ct = tr("MATCH (a)-[r]->(b) WHERE a.name CONTAINS 'sub' RETURN a.name");
        assert!(ct.contains("| where a_name contains 'sub'"), "{ct}");
        crate::kql_to_sql(&ct).expect("CONTAINS KQL must compile to SQL");
    }

    #[test]
    fn where_starts_without_with_is_rejected() {
        let g = binding();
        assert!(
            cypher_to_kql("MATCH (a)-[r]->(b) WHERE a.name STARTS 'pre' RETURN a.name", &g)
                .is_err(),
            "STARTS without WITH must be a parse error"
        );
    }

    #[test]
    fn where_is_null_and_is_not_null_lower_and_compile() {
        let n = tr("MATCH (a)-[r]->(b) WHERE a.deleted IS NULL RETURN a.name");
        assert!(n.contains("| where isnull(a_deleted)"), "{n}");
        let sql = crate::kql_to_sql(&n).expect("IS NULL KQL must compile to SQL");
        assert!(sql.to_uppercase().contains("IS NULL"), "{sql}");
        let nn = tr("MATCH (a)-[r]->(b) WHERE a.deleted IS NOT NULL RETURN a.name");
        assert!(nn.contains("| where isnotnull(a_deleted)"), "{nn}");
        let sql2 = crate::kql_to_sql(&nn).expect("IS NOT NULL KQL must compile to SQL");
        assert!(sql2.to_uppercase().contains("IS NOT NULL"), "{sql2}");
    }

    #[test]
    fn where_is_null_in_tree_and_with_optional() {
        // Companion to OPTIONAL MATCH (which yields NULLs): combine in a tree.
        let kql = tr(
            "MATCH (a)-[r]->(b) OPTIONAL MATCH (b)-[s]->(c) \
             WHERE a.k = 'x' AND (c.id IS NULL OR c.k = 'y') RETURN a.k",
        );
        assert!(kql.contains("(a_k == 'x' and (isnull(c_id) or c_k == 'y'))"), "{kql}");
        crate::kql_to_sql(&kql).expect("IS NULL in tree must compile to SQL");
    }

    #[test]
    fn where_is_without_null_is_rejected() {
        let g = binding();
        assert!(
            cypher_to_kql("MATCH (a)-[r]->(b) WHERE a.k IS 'x' RETURN a.k", &g).is_err(),
            "IS without NULL must be a parse error"
        );
    }

    #[test]
    fn where_string_op_combines_with_and() {
        let kql = tr(
            "MATCH (a)-[r]->(b) WHERE a.name STARTS WITH 'p' AND b.n > 3 RETURN a.name, b.n",
        );
        assert!(kql.contains("a_name startswith 'p'"), "{kql}");
        assert!(kql.contains("b_n > 3"), "{kql}");
        crate::kql_to_sql(&kql).expect("string-op + AND KQL must compile to SQL");
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
    fn multi_hop_backward_hop_now_decomposes() {
        // Regression guard: a backward hop in a multi-hop chain used to be
        // rejected; it now decomposes into single-hop segments joined on the
        // shared node (see backward_hop_in_multihop_decomposes_to_general_join).
        let kql = cypher_to_kql("MATCH (a)-[r1]->(b)<-[r2]-(c) RETURN a.name", &binding())
            .expect("mixed-direction multi-hop is supported");
        assert!(
            kql.contains("graph-match (a)-[r1]->(b), (c)-[r2]->(b)"),
            "{kql}"
        );
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
    fn count_aggregation_lowers_to_summarize() {
        // Grouped count: non-aggregate items become GROUP BY keys.
        let grouped = tr("MATCH (a)-[r]->(b) RETURN a.name, count(*) AS n");
        assert!(
            grouped.contains("| summarize n = count(*) by a_name"),
            "grouped count: {grouped}"
        );
        crate::kql_to_sql(&grouped).expect("grouped count compiles");
        // Global count: no group keys, throwaway projection keeps graph-match valid.
        let global = tr("MATCH (a)-[r]->(b) RETURN count(*)");
        assert!(
            global.contains("| summarize count = count(*)") && !global.contains("count = count(*) by"),
            "global count: {global}"
        );
        crate::kql_to_sql(&global).expect("global count compiles");
        // count(<var>) counts that var's id.
        let cv = tr("MATCH (a)-[r]->(b) RETURN a.name, count(b) AS bs");
        assert!(cv.contains("| summarize bs = count(b_id) by a_name"), "{cv}");
        crate::kql_to_sql(&cv).expect("count(var) compiles");
    }

    #[test]
    fn sum_avg_min_max_aggregates_over_a_property() {
        // sum/avg/min/max take a property and group by the non-aggregate items.
        let s = tr("MATCH (a)-[r]->(b) RETURN a.name, sum(b.weight) AS w");
        assert!(s.contains("| summarize w = sum(b_weight) by a_name"), "{s}");
        crate::kql_to_sql(&s).expect("sum compiles");
        let m = tr("MATCH (a)-[r]->(b) RETURN min(b.weight) AS lo, max(b.weight) AS hi");
        assert!(
            m.contains("| summarize lo = min(b_weight), hi = max(b_weight)"),
            "{m}"
        );
        crate::kql_to_sql(&m).expect("min/max compiles");
        // collect(prop) → KQL make_list → SQL array_agg, grouped by the keys.
        let c = tr("MATCH (a)-[r]->(b) RETURN a.name, collect(b.name) AS bs");
        assert!(c.contains("| summarize bs = make_list(b_name) by a_name"), "{c}");
        crate::kql_to_sql(&c).expect("collect compiles to array_agg");
        // collect needs a property argument (like sum/avg/min/max).
        assert!(
            tr_err("MATCH (a)-[r]->(b) RETURN collect(b)").contains("property argument"),
            "collect(<var>) rejected"
        );
        // sum/avg/min/max need a property — sum(*) / sum(<var>) are rejected.
        assert!(
            tr_err("MATCH (a)-[r]->(b) RETURN sum(*)").contains("property argument"),
            "sum(*) rejected"
        );
        assert!(
            tr_err("MATCH (a)-[r]->(b) RETURN avg(b)").contains("property argument"),
            "avg(<var>) rejected"
        );
    }

    #[test]
    fn where_or_and_and_join_correctly() {
        // OR → one parenthesized `| where (… or …)`.
        let or = tr("MATCH (a)-[r]->(b) WHERE a.name = 'x' OR b.name = 'y' RETURN a.name");
        assert!(
            or.contains("| where (a_name == 'x' or b_name == 'y')"),
            "OR-joined: {or}"
        );
        crate::kql_to_sql(&or).expect("OR where compiles");
        // AND → separate `| where` per predicate (unchanged).
        let and = tr("MATCH (a)-[r]->(b) WHERE a.name = 'x' AND b.name = 'y' RETURN a.name");
        assert!(
            and.contains("| where a_name == 'x'") && and.contains("| where b_name == 'y'"),
            "AND-joined: {and}"
        );
        // Mixed AND/OR now lowers via the boolean tree with standard precedence:
        // `a AND b OR c` ⇒ `((a and b) or c)`.
        let mixed = tr(
            "MATCH (a)-[r]->(b) WHERE a.name='x' AND b.name='y' OR a.name='z' RETURN a.name",
        );
        assert!(
            mixed.contains("| where ((a_name == 'x' and b_name == 'y') or a_name == 'z')"),
            "mixed AND/OR precedence: {mixed}"
        );
        crate::kql_to_sql(&mixed).expect("mixed AND/OR where compiles");
    }

    #[test]
    fn docs_graph_md_cypher_example_compiles() {
        // The exact example published in docs/site/query/graph.md must parse +
        // lower + compile to SQL — combines anonymous typed var-length, label,
        // STARTS WITH, parens + NOT WHERE, ORDER BY, LIMIT.
        let kql = tr(
            "MATCH (a:service)-[:CALLS*1..3]->(b) \
             WHERE a.name STARTS WITH 'api-' AND (b.tier = 'db' OR NOT b.healthy = 'true') \
             RETURN a.name, b.name ORDER BY a.name LIMIT 50",
        );
        crate::kql_to_sql(&kql).expect("docs graph.md cypher example must compile to SQL");
    }

    #[test]
    fn where_parenthesized_group_overrides_precedence() {
        // `a AND (b OR c)` keeps the OR grouped — distinct from `a AND b OR c`.
        let kql = tr(
            "MATCH (a)-[r]->(b) WHERE a.k = 'x' AND (b.n = 1 OR b.n = 2) RETURN a.k",
        );
        assert!(
            kql.contains("| where (a_k == 'x' and (b_n == 1 or b_n == 2))"),
            "{kql}"
        );
        crate::kql_to_sql(&kql).expect("parenthesized WHERE must compile to SQL");
    }

    #[test]
    fn where_not_negation_lowers_and_compiles() {
        let kql = tr("MATCH (a)-[r]->(b) WHERE NOT a.name CONTAINS 'tmp' RETURN a.name");
        assert!(kql.contains("| where not (a_name contains 'tmp')"), "{kql}");
        crate::kql_to_sql(&kql).expect("NOT WHERE must compile to SQL");
    }

    #[test]
    fn where_unclosed_group_is_rejected() {
        let g = binding();
        assert!(
            cypher_to_kql("MATCH (a)-[r]->(b) WHERE (a.k = 1 AND b.n = 2 RETURN a.k", &g).is_err(),
            "unclosed WHERE group must be a parse error"
        );
    }

    #[test]
    fn order_by_and_distinct_lower_to_sort_and_distinct() {
        // Regression: ORDER BY + RETURN DISTINCT used to be rejected.
        let kql = tr("MATCH (a)-[r]->(b) RETURN DISTINCT a.name ORDER BY a.name DESC");
        assert!(kql.contains("| distinct"), "DISTINCT → | distinct: {kql}");
        assert!(kql.contains("| sort by a_name desc"), "ORDER BY DESC → sort: {kql}");
        crate::kql_to_sql(&kql).expect("DISTINCT + ORDER BY KQL must compile");

        // Default ASC; multiple sort keys; ORDER BY then LIMIT (top-k).
        let multi = tr("MATCH (a)-[r]->(b) RETURN a.name, b.name ORDER BY a.name, b.name DESC LIMIT 3");
        assert!(
            multi.contains("| sort by a_name asc, b_name desc") && multi.contains("| take 3"),
            "{multi}"
        );
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
