//! Accumulator used to build up an SQL query while parsing KQL operators.
//!
//! KQL is a pipeline: the output of one operator is the input of the next.
//! SQL is a flat SELECT. The `QueryState` captures what's been said so far;
//! `to_sql` renders at the end.

/// Holds the graph definition recorded by `make-graph` for use by a
/// following `graph-match` operator.
#[derive(Debug, Clone)]
pub(crate) struct GraphDef {
    /// The edge table (the table that was active when `make-graph` ran).
    pub edge_table: String,
    /// The edge column that holds the source node id.
    pub src_col: String,
    /// The edge column that holds the destination node id.
    pub dst_col: String,
    /// The node table to join for property lookup.
    pub node_table: String,
    /// The node table's primary-key column (joined to src/dst).
    pub id_col: String,
}

#[derive(Debug, Default)]
pub(crate) struct QueryState {
    pub table: String,
    /// The original root table this pipeline reads from. Unlike `table`,
    /// which graph operators reassign to a CTE name (`_gt_result`, …), this
    /// is set once at construction and never changes. `union` uses it to
    /// determine a sub-pipeline branch's schema for the column superset.
    pub root: String,
    /// Explicit projections, e.g. "timestamp", "status".
    pub select: Vec<String>,
    /// Columns to *exclude* (project-away). Can't be combined with `select`.
    pub exclude: Vec<String>,
    /// `extend` clauses: (name, sql_expr).
    pub extend: Vec<(String, String)>,
    /// AND'd together WHERE fragments.
    pub where_clauses: Vec<String>,
    /// Grouping keys (col or bin expression), populated by `summarize … by`.
    pub group_by: Vec<String>,
    /// Aggregate SELECT items: `"count(*) AS Count"`, `"avg(latency) AS avg_latency"`.
    pub aggregates: Vec<String>,
    /// Ordering: (sql_expr, DESC?).
    pub order_by: Vec<(String, bool)>,
    pub limit: Option<u64>,
    pub distinct: bool,
    /// Graph definition recorded by `make-graph`; consumed by `graph-match`.
    pub graph_def: Option<GraphDef>,
    /// CTEs to emit in a `WITH [RECURSIVE] …` prelude. Tuple:
    /// `(name, body, needs_recursive)`. When any cte has
    /// `needs_recursive = true`, the whole WITH clause becomes
    /// `WITH RECURSIVE`.
    ///
    /// Graph operators (`graph-traverse`, `graph-shortest-path`) populate
    /// these; regular operators leave them empty and the SQL stays flat.
    pub ctes: Vec<(String, String, bool)>,
    /// Set by `summarize`. While true, a following `where` (HAVING) or `project`
    /// (post-aggregation projection) must run against the aggregated result, not
    /// the pre-aggregation rows — so those operators first
    /// [`rebase_aggregation`](Self::rebase_aggregation) to read from a sealed CTE.
    pub aggregated: bool,
    /// Monotonic suffix for the sealed-aggregation CTE names (`_agg0`, …).
    pub agg_seq: u32,
}

impl QueryState {
    pub(crate) fn new(table: impl Into<String>) -> Self {
        let table = table.into();
        Self {
            root: table.clone(),
            table,
            ..Default::default()
        }
    }

    /// The original root table (the table named at the head of the pipeline),
    /// independent of any CTE reassignment of `table` by graph operators.
    pub(crate) fn root_table(&self) -> &str {
        &self.root
    }

    /// Seal the current aggregation (`summarize`) as a top-level CTE and reset
    /// the state to read from it, so a following `where`/`project`/`sort`/`take`
    /// builds a flat SELECT over the aggregated columns (HAVING + post-agg
    /// projection). Keeps any graph-operator CTEs alongside the new `_aggN` —
    /// no nesting. Idempotent per aggregation: clears `aggregated`.
    pub(crate) fn rebase_aggregation(&mut self) {
        let mut items: Vec<String> = self.group_by.clone();
        items.extend(self.aggregates.iter().cloned());
        let mut body = format!("SELECT {} FROM {}", items.join(", "), self.table);
        if !self.where_clauses.is_empty() {
            body.push_str(" WHERE ");
            body.push_str(&self.where_clauses.join(" AND "));
        }
        if !self.group_by.is_empty() {
            body.push_str(" GROUP BY ");
            body.push_str(&self.group_by.join(", "));
        }
        let name = format!("_agg{}", self.agg_seq);
        self.agg_seq += 1;
        self.ctes.push((name.clone(), body, false));
        // Read from the sealed CTE; subsequent operators are flat over it.
        self.table = name;
        self.select.clear();
        self.exclude.clear();
        self.extend.clear();
        self.where_clauses.clear();
        self.group_by.clear();
        self.aggregates.clear();
        self.order_by.clear();
        self.limit = None;
        self.distinct = false;
        self.aggregated = false;
    }

    pub(crate) fn to_sql(&self) -> String {
        let prelude = self.render_cte_prelude();
        // SELECT list.
        let select_clause = if !self.aggregates.is_empty() {
            // summarize => GROUP BY + aggregate expressions.
            let mut items: Vec<String> = self.group_by.iter().cloned().collect();
            items.extend(self.aggregates.iter().cloned());
            items.join(", ")
        } else if self.distinct {
            let mut items: Vec<String> = Vec::new();
            if self.select.is_empty() {
                if self.extend.is_empty() {
                    items.push("*".to_string());
                } else {
                    for (name, expr) in &self.extend {
                        items.push(format!("({expr}) AS {name}"));
                    }
                }
            } else {
                for col in &self.select {
                    if let Some((_, expr)) = self.extend.iter().find(|(n, _)| n == col) {
                        items.push(format!("({expr}) AS {col}"));
                    } else {
                        items.push(col.clone());
                    }
                }
            }
            items.join(", ")
        } else {
            let mut items = Vec::new();
            if self.select.is_empty() && self.exclude.is_empty() {
                items.push("*".to_string());
            } else if !self.select.is_empty() {
                items.extend(self.select.iter().cloned());
            } else {
                // project-away — DataFusion supports the `* EXCEPT (...)`
                // wildcard option, so excluded columns are actually dropped.
                // (A bare `SELECT *` here leaked embedding vectors into
                // search previews and broke unified-search RRF dedup.)
                items.push(format!("* EXCEPT ({})", self.exclude.join(", ")));
            }
            for (name, expr) in &self.extend {
                items.push(format!("({expr}) AS {name}"));
            }
            items.join(", ")
        };

        let distinct = if self.distinct && self.aggregates.is_empty() {
            "DISTINCT "
        } else {
            ""
        };

        let mut sql = format!(
            "{prelude}SELECT {distinct}{select_clause} FROM {}",
            self.table
        );

        if !self.where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&self.where_clauses.join(" AND "));
        }

        if !self.group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&self.group_by.join(", "));
        }

        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            let parts: Vec<String> = self
                .order_by
                .iter()
                .map(|(c, desc)| {
                    if *desc {
                        format!("{c} DESC")
                    } else {
                        format!("{c} ASC")
                    }
                })
                .collect();
            sql.push_str(&parts.join(", "));
        }

        if let Some(n) = self.limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }

        sql
    }

    fn render_cte_prelude(&self) -> String {
        if self.ctes.is_empty() {
            return String::new();
        }
        let any_recursive = self.ctes.iter().any(|(_, _, r)| *r);
        let keyword = if any_recursive {
            "WITH RECURSIVE "
        } else {
            "WITH "
        };
        let parts: Vec<String> = self
            .ctes
            .iter()
            .map(|(name, body, _)| format!("{name} AS ({body})"))
            .collect();
        format!("{keyword}{} ", parts.join(", "))
    }
}
