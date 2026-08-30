//! Differential runner: oracle vs an engine under test.
//!
//! The harness is deliberately decoupled from pensieve: anything that can answer
//! a [`ChainPattern`] with node-id tuples (the [`GraphEngineUnderTest`]
//! trait) can be differentially validated. [`HttpPensieveEngine`] is the live
//! implementation that submits the pattern as Cypher over
//! `POST /v1/query` (`content-type: application/x-cypher`).
//!
//! Comparison is **set-based and order-insensitive**: the engine returns a
//! bag of rows in arbitrary order; both sides are collapsed to
//! `BTreeSet<Vec<String>>` before diffing (see `oracle::match_chain` docs for
//! why duplicates are expected on the engine side).

use std::collections::BTreeSet;

use anyhow::{bail, Context};

use crate::model::TestGraph;
use crate::oracle::{self, ChainPattern};

/// Anything that can evaluate a forward MATCH chain and return the bound
/// node-id tuples (`[n0, n1, …, nK]` per row, possibly with duplicates).
#[async_trait::async_trait]
pub trait GraphEngineUnderTest {
    async fn match_chain(&self, pattern: &ChainPattern) -> anyhow::Result<Vec<Vec<String>>>;
}

/// One pattern's divergence (or engine error).
#[derive(Debug)]
pub struct Mismatch {
    /// The Cypher text submitted to the engine.
    pub cypher: String,
    /// Distinct tuple counts on each side.
    pub oracle_count: usize,
    pub engine_count: usize,
    /// Tuples the oracle produced but the engine did not (sample, ≤ 10).
    pub missing: Vec<Vec<String>>,
    /// Tuples the engine produced but the oracle did not (sample, ≤ 10).
    pub unexpected: Vec<Vec<String>>,
    /// Set when the engine errored instead of answering.
    pub error: Option<String>,
}

/// Outcome of [`run_differential`] over one graph + pattern set.
#[derive(Debug, Default)]
pub struct DiffReport {
    /// Number of patterns evaluated.
    pub patterns_run: usize,
    /// Distinct-tuple totals across all clean patterns (visibility only).
    pub tuples_compared: usize,
    pub mismatches: Vec<Mismatch>,
}

impl DiffReport {
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Human-readable diff, one block per mismatch.
    pub fn render(&self) -> String {
        let mut s = format!(
            "{} patterns, {} distinct tuples compared, {} mismatch(es)\n",
            self.patterns_run,
            self.tuples_compared,
            self.mismatches.len()
        );
        for m in &self.mismatches {
            s.push_str(&format!("\n--- MISMATCH: {}\n", m.cypher));
            if let Some(e) = &m.error {
                s.push_str(&format!("    engine error: {e}\n"));
                continue;
            }
            s.push_str(&format!(
                "    oracle: {} tuples, engine: {} tuples\n",
                m.oracle_count, m.engine_count
            ));
            for t in &m.missing {
                s.push_str(&format!("    missing from engine: ({})\n", t.join(", ")));
            }
            for t in &m.unexpected {
                s.push_str(&format!("    unexpected in engine: ({})\n", t.join(", ")));
            }
        }
        s
    }
}

/// Run every pattern through both the oracle (locally, over `graph`) and the
/// engine, and diff the distinct tuple sets.
pub async fn run_differential<E: GraphEngineUnderTest>(
    engine: &E,
    graph: &TestGraph,
    patterns: &[ChainPattern],
) -> DiffReport {
    let oracle = oracle::Oracle::new(graph);
    let mut report = DiffReport::default();
    for pattern in patterns {
        report.patterns_run += 1;
        let cypher = pattern.to_cypher();
        let expected = oracle.match_chain(pattern);
        let got: BTreeSet<Vec<String>> = match engine.match_chain(pattern).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(e) => {
                report.mismatches.push(Mismatch {
                    cypher,
                    oracle_count: expected.len(),
                    engine_count: 0,
                    missing: Vec::new(),
                    unexpected: Vec::new(),
                    error: Some(format!("{e:#}")),
                });
                continue;
            }
        };
        if got == expected {
            report.tuples_compared += expected.len();
            continue;
        }
        let missing: Vec<_> = expected.difference(&got).take(10).cloned().collect();
        let unexpected: Vec<_> = got.difference(&expected).take(10).cloned().collect();
        report.mismatches.push(Mismatch {
            cypher,
            oracle_count: expected.len(),
            engine_count: got.len(),
            missing,
            unexpected,
            error: None,
        });
    }
    report
}

// =====================================================================
// Live-pensieve engine over HTTP
// =====================================================================

/// Submits patterns as Cypher to a running pensieve.
///
/// Wire contract (matches `crates/pensieve-server/tests/cypher_query_it.rs` and
/// `scripts/test-graph.sh`):
/// - `POST {base_url}/v1/query`, body = Cypher text,
///   `content-type: application/x-cypher`,
///   `x-database: <db>`, `x-graph: <db>/<graph>`,
///   optional `authorization: Bearer <token>`.
/// - Response: `application/x-ndjson` — one JSON object per result row, with
///   one key per RETURN projection. [`ChainPattern::to_cypher`] projects
///   `n0.id, n1.id, …`, which arrive as columns `n0_id, n1_id, …`.
pub struct HttpPensieveEngine {
    pub base_url: String,
    pub database: String,
    pub graph: String,
    pub token: Option<String>,
    client: reqwest::Client,
}

impl HttpPensieveEngine {
    pub fn new(
        base_url: impl Into<String>,
        database: impl Into<String>,
        graph: impl Into<String>,
        token: Option<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            database: database.into(),
            graph: graph.into(),
            token,
            client: reqwest::Client::new(),
        }
    }

    /// Engine location from env: `PENSIEVE_BASE_URL` (default
    /// `http://127.0.0.1:8080`), `PENSIEVE_DATABASE` (default `default`),
    /// `PENSIEVE_TOKEN` (optional bearer token).
    pub fn from_env(graph: &str) -> Self {
        Self::new(
            std::env::var("PENSIEVE_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            std::env::var("PENSIEVE_DATABASE").unwrap_or_else(|_| "default".into()),
            graph,
            std::env::var("PENSIEVE_TOKEN").ok(),
        )
    }
}

#[async_trait::async_trait]
impl GraphEngineUnderTest for HttpPensieveEngine {
    async fn match_chain(&self, pattern: &ChainPattern) -> anyhow::Result<Vec<Vec<String>>> {
        let cypher = pattern.to_cypher();
        let mut req = self
            .client
            .post(format!("{}/v1/query", self.base_url))
            .header("content-type", "application/x-cypher")
            .header("x-database", &self.database)
            .header("x-graph", format!("{}/{}", self.database, self.graph))
            .body(cypher.clone());
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.context("POST /v1/query")?;
        let status = resp.status();
        let body = resp.text().await.context("read query response body")?;
        if !status.is_success() {
            bail!("query `{cypher}` failed: HTTP {status}: {body}");
        }

        let n_cols = pattern.nodes.len();
        let mut rows = Vec::new();
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)
                .with_context(|| format!("bad NDJSON line from engine: {line}"))?;
            let obj = v
                .as_object()
                .with_context(|| format!("NDJSON line is not an object: {line}"))?;
            let mut tuple = Vec::with_capacity(n_cols);
            for i in 0..n_cols {
                let key = format!("n{i}_id");
                let cell = obj
                    .get(&key)
                    .with_context(|| format!("column `{key}` missing in row: {line}"))?;
                tuple.push(match cell {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            }
            rows.push(tuple);
        }
        Ok(rows)
    }
}
