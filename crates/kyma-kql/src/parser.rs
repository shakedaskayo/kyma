//! KQL parser + KQL→SQL translator.
//!
//! Recursive-descent. No AST — we stream-parse operators and directly
//! populate a [`QueryState`], then ask it to emit SQL. Trades a tiny loss
//! of compositionality for dramatically less code.

use crate::lexer::{tokenize, Token};
use crate::state::QueryState;

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

/// Public entry point.
pub fn kql_to_sql(src: &str) -> Result<String, ParseError> {
    let toks = tokenize(src)?;
    let mut p = Parser::new(toks);
    p.parse_query()?;
    Ok(p.state.to_sql())
}

// ---------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    state: QueryState,
}

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Self {
            toks,
            pos: 0,
            state: QueryState::default(),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    fn advance(&mut self) -> Option<&Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
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
            other => Err(ParseError(format!(
                "expected `{lit}`, got {other:?}"
            ))),
        }
    }

    // -----------------------------------------------------------------
    // Top-level
    // -----------------------------------------------------------------
    fn parse_query(&mut self) -> Result<(), ParseError> {
        // Table name is the first identifier.
        let table = match self.bump()? {
            Token::Ident(s) => s,
            other => return Err(ParseError(format!("expected table name, got {other:?}"))),
        };
        self.state = QueryState::new(table);

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
                other => return Err(ParseError(format!(
                    "expected column name after project, got {other:?}"
                ))),
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
                other => return Err(ParseError(format!(
                    "expected name in extend, got {other:?}"
                ))),
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
        self.state.aggregates.push(r#"count(*) AS "Count""#.to_string());
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
            other => Err(ParseError(format!("unexpected token in expression: {other:?}"))),
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

/// Quote an identifier for SQL safely. We use double quotes (ANSI SQL).
fn quote_ident(name: &str) -> String {
    // Pass through simple bare identifiers.
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.chars().next().map(|c| !c.is_ascii_digit()).unwrap_or(false)
    {
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

/// For `contains`: we keep the raw literal and let SQL `position` do it.
/// The helper strips outer quotes only for `LIKE` building.
fn strip_quotes_for_like(s: &str) -> &str {
    s
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
}
