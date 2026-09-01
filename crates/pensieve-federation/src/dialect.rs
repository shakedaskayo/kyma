//! T-SQL flavoring for DataFusion's unparser.
//!
//! DataFusion ships Postgres/MySQL/SQLite unparser dialects but no T-SQL one.
//! Two layers close the gap for the constructs pensieve actually pushes down:
//!
//! 1. A [`CustomDialect`](datafusion::sql::unparser::dialect::CustomDialect)
//!    configured for double-quoted identifiers (valid under T-SQL's default
//!    `QUOTED_IDENTIFIER ON`, which TDS clients negotiate).
//! 2. An [`AstAnalyzer`] pass over the unparsed `sqlparser` AST fixing what
//!    the dialect options can't express:
//!    - `LIMIT n`  → `TOP (n)` (T-SQL has no LIMIT clause);
//!    - `LIMIT n OFFSET m` → `OFFSET m ROWS FETCH NEXT n ROWS ONLY` (only
//!      valid with `ORDER BY`, which DataFusion emits whenever the plan is
//!      ordered — un-ordered OFFSET is left as-is and surfaces a remote
//!      error rather than silently wrong results);
//!    - boolean literals `true`/`false` → `1`/`0` (T-SQL `bit` comparisons).
//!
//! Anything the analyzer can't express stays untouched: datafusion-federation
//! sends the SQL and the remote error is surfaced verbatim to the caller,
//! which is the debuggable failure mode we want while the dialect widens.

use std::ops::ControlFlow;
use std::sync::Arc;

use datafusion::error::Result as DfResult;
use datafusion::sql::sqlparser::ast::{
    self, Expr, Fetch, Offset, OffsetRows, SetExpr, Statement, Top, TopQuantity, Value,
    VisitMut, VisitorMut,
};
use datafusion::sql::unparser::dialect::{CustomDialectBuilder, Dialect};
use datafusion_federation::sql::AstAnalyzer;

/// Unparser dialect for T-SQL endpoints (Microsoft Fabric SQL endpoint,
/// Azure SQL, SQL Server).
pub fn tsql_dialect() -> Arc<dyn Dialect> {
    Arc::new(
        CustomDialectBuilder::new()
            .with_identifier_quote_style('"')
            .build(),
    )
}

/// AST analyzer applying the T-SQL rewrites described in the module docs.
pub fn tsql_ast_analyzer() -> AstAnalyzer {
    Box::new(rewrite_statement)
}

fn rewrite_statement(mut stmt: Statement) -> DfResult<Statement> {
    let mut rewriter = TsqlRewriter;
    let _ = stmt.visit(&mut rewriter);
    Ok(stmt)
}

struct TsqlRewriter;

impl VisitorMut for TsqlRewriter {
    type Break = ();

    fn post_visit_query(&mut self, query: &mut ast::Query) -> ControlFlow<Self::Break> {
        rewrite_limit(query);
        ControlFlow::Continue(())
    }

    fn post_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Self::Break> {
        if let Expr::Value(Value::Boolean(b)) = expr {
            *expr = Expr::Value(Value::Number(if *b { "1" } else { "0" }.into(), false));
        }
        ControlFlow::Continue(())
    }
}

/// Move a `LIMIT` into T-SQL form on one query level.
fn rewrite_limit(query: &mut ast::Query) {
    if query.limit.is_none() && query.offset.is_none() {
        return;
    }
    let has_order_by = !query.order_by.as_ref().map_or(true, |o| o.exprs.is_empty());

    match (&query.offset, has_order_by) {
        // LIMIT without OFFSET → TOP (n) on the SELECT (works unordered too).
        (None, _) => {
            if let Some(limit) = query.limit.take() {
                if let SetExpr::Select(select) = query.body.as_mut() {
                    if select.top.is_none() {
                        select.top = Some(Top {
                            with_ties: false,
                            percent: false,
                            quantity: Some(TopQuantity::Expr(limit)),
                        });
                    }
                }
            }
        }
        // LIMIT+OFFSET with ORDER BY → OFFSET … ROWS FETCH NEXT … ROWS ONLY.
        (Some(_), true) => {
            if let Some(Offset { rows, .. }) = query.offset.as_mut() {
                if matches!(rows, OffsetRows::None) {
                    *rows = OffsetRows::Rows;
                }
            }
            if let Some(limit) = query.limit.take() {
                query.fetch = Some(Fetch {
                    with_ties: false,
                    percent: false,
                    quantity: Some(limit),
                });
            }
        }
        // OFFSET without ORDER BY is not expressible in T-SQL; leave the
        // query untouched and let the remote error surface.
        (Some(_), false) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::sql::sqlparser::dialect::GenericDialect;
    use datafusion::sql::sqlparser::parser::Parser;

    fn rewrite(sql: &str) -> String {
        let stmt = Parser::parse_sql(&GenericDialect {}, sql)
            .unwrap()
            .remove(0);
        rewrite_statement(stmt).unwrap().to_string()
    }

    #[test]
    fn limit_becomes_top() {
        assert_eq!(
            rewrite("SELECT a FROM t LIMIT 10"),
            "SELECT TOP (10) a FROM t"
        );
    }

    #[test]
    fn limit_offset_with_order_by_becomes_offset_fetch() {
        assert_eq!(
            rewrite("SELECT a FROM t ORDER BY a LIMIT 10 OFFSET 5"),
            // sqlparser renders FETCH FIRST; T-SQL accepts FIRST and NEXT alike.
            "SELECT a FROM t ORDER BY a OFFSET 5 ROWS FETCH FIRST 10 ROWS ONLY"
        );
    }

    #[test]
    fn boolean_literals_become_bits() {
        assert_eq!(
            rewrite("SELECT a FROM t WHERE flag = true AND other = false"),
            "SELECT a FROM t WHERE flag = 1 AND other = 0"
        );
    }

    #[test]
    fn nested_subquery_limit_is_rewritten() {
        assert_eq!(
            rewrite("SELECT a FROM (SELECT a FROM t LIMIT 3) AS s"),
            "SELECT a FROM (SELECT TOP (3) a FROM t) AS s"
        );
    }

    #[test]
    fn plain_query_untouched() {
        assert_eq!(
            rewrite("SELECT a, b FROM t WHERE a > 5"),
            "SELECT a, b FROM t WHERE a > 5"
        );
    }
}
