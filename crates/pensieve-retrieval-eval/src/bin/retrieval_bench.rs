//! End-to-end retrieval benchmark against a RUNNING kyma engine over HTTP.
//!
//! Flow:
//!   1. Generate a seeded clustered-gaussian dataset for the tier
//!      (pr=100k, nightly=1M, release=10M rows; 384-dim, L2-normalized).
//!   2. Ingest via `POST /v1/ingest` NDJSON in batches. Rows are
//!      `{"id": "v0000123", "text": "...", "embedding": [f32; dim]}` —
//!      the vector column is named `embedding`, matching the convention in
//!      `scripts/test-vectors.sh` and `kyma-server/src/search` (the unified
//!      search vector leg + `cosine_distance` UDF operate on
//!      `FixedSizeList<Float32>` columns).
//!   3. Run N seeded queries via `POST /v1/query` SQL:
//!      `SELECT id, cosine_distance(embedding, make_array(..)) AS d
//!       FROM <table> ORDER BY d ASC, id ASC LIMIT k`.
//!   4. Compare each result set against the in-process exact oracle on the
//!      same generated data → recall@k. Brute-force SQL IS the oracle today,
//!      so recall must be 1.0 — this run validates the harness itself and
//!      captures the latency baseline. S1 ANN work makes recall < 1.0
//!      possible and gates it at >= 0.95.
//!   5. Emit a JSON report to stdout and `--out=PATH`.
//!
//! The target table must already exist with a `vector(dim)` column —
//! ingest auto-create infers plain Utf8 columns, which `cosine_distance`
//! can't use. `scripts/retrieval-bench.sh` provisions it via
//! `kyma-cli create-table --schema 'id:string,text:string,embedding:vector(384)'`.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, ValueEnum};
use kyma_retrieval_eval::datasets::{clustered_gaussian, queries_from_dataset};
use kyma_retrieval_eval::metrics::{mean, recall_at_k, LatencyStats};
use kyma_retrieval_eval::oracle::{ExactOracle, Metric};
use serde::Serialize;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// Benchmark tier — controls dataset size.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Tier {
    /// 100k vectors — fast enough for PR CI.
    Pr,
    /// 1M vectors.
    Nightly,
    /// 10M vectors (needs ~16 GiB RAM for the in-process oracle copy).
    Release,
}

impl Tier {
    fn rows(self) -> usize {
        match self {
            Tier::Pr => 100_000,
            Tier::Nightly => 1_000_000,
            Tier::Release => 10_000_000,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Tier::Pr => "pr",
            Tier::Nightly => "nightly",
            Tier::Release => "release",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "retrieval_bench",
    about = "Retrieval-quality + latency benchmark against a running kyma engine"
)]
struct Args {
    /// Engine base URL (scheme optional; bare host:port accepted).
    #[arg(long, env = "KYMA_HTTP_ADDR", default_value = "http://127.0.0.1:8080")]
    engine_url: String,

    /// Bearer token for engines with auth enabled.
    #[arg(long, env = "KYMA_AUTH_TOKEN")]
    auth_token: Option<String>,

    /// Dataset tier: pr=100k, nightly=1M, release=10M vectors.
    #[arg(long, value_enum, default_value = "pr")]
    tier: Tier,

    /// Number of seeded queries to run.
    #[arg(long, default_value_t = 100)]
    queries: usize,

    /// Top-k per query (recall@k cutoff).
    #[arg(long, default_value_t = 10)]
    k: usize,

    /// Vector dimensionality.
    #[arg(long, default_value_t = 384)]
    dim: usize,

    /// RNG seed (dataset + queries derive from this).
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Target database.
    #[arg(long, default_value = "retrieval_bench")]
    db: String,

    /// Target table (must exist with an `embedding vector(dim)` column).
    #[arg(long, default_value = "vectors")]
    table: String,

    /// Rows per ingest request.
    #[arg(long, default_value_t = 1000)]
    batch_size: usize,

    /// Skip ingest (data already loaded from an identical-seed prior run).
    #[arg(long, default_value_t = false)]
    skip_ingest: bool,

    /// Also write the JSON report to this path.
    #[arg(long)]
    out: Option<std::path::PathBuf>,
}

#[derive(Debug, Serialize)]
struct Report {
    tier: String,
    n: usize,
    dim: usize,
    queries: usize,
    k: usize,
    seed: u64,
    recall_at_10: f64,
    /// Per-query recall min — catches a single bad query hiding in the mean.
    recall_min: f64,
    latency: LatencyStats,
    ingest_secs: f64,
    timestamp: String,
    engine_url: String,
}

struct Engine {
    base: String,
    auth: Option<String>,
    http: reqwest::blocking::Client,
}

impl Engine {
    fn new(url: &str, auth: Option<String>) -> Result<Self> {
        // load-test.sh exports KYMA_HTTP_ADDR=127.0.0.1:8080 (no scheme);
        // accept both forms.
        let base = if url.starts_with("http://") || url.starts_with("https://") {
            url.trim_end_matches('/').to_string()
        } else {
            format!("http://{}", url.trim_end_matches('/'))
        };
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_mins(5))
            .build()
            .context("building HTTP client")?;
        Ok(Self { base, auth, http })
    }

    fn with_auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.auth {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    fn health(&self) -> Result<()> {
        let resp = self
            .http
            .get(format!("{}/health", self.base))
            .send()
            .with_context(|| format!("engine unreachable at {}", self.base))?;
        if !resp.status().is_success() {
            bail!("engine health check failed: HTTP {}", resp.status());
        }
        Ok(())
    }

    fn ingest_ndjson(&self, db: &str, table: &str, body: String) -> Result<()> {
        let resp = self
            .with_auth(self.http.post(format!("{}/v1/ingest", self.base)))
            .header("X-Database", db)
            .header("X-Table", table)
            // The table is pre-provisioned with a typed vector column; fail
            // loudly instead of silently creating a stringly-typed table.
            .header("X-Auto-Create", "false")
            .header("Content-Type", "application/x-ndjson")
            .body(body)
            .send()
            .context("sending ingest batch")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().unwrap_or_default();
            bail!("ingest failed: HTTP {status}: {text}");
        }
        Ok(())
    }

    /// POST SQL to /v1/query; the engine answers NDJSON (one JSON object per
    /// row). Returns the rows in order.
    fn query_sql(&self, db: &str, sql: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .with_auth(self.http.post(format!("{}/v1/query", self.base)))
            .header("X-Database", db)
            .header("Content-Type", "application/sql")
            .body(sql.to_string())
            .send()
            .context("sending query")?;
        let status = resp.status();
        let text = resp.text().context("reading query response")?;
        if !status.is_success() {
            bail!("query failed: HTTP {status}: {text}");
        }
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).with_context(|| format!("bad result row: {l}")))
            .collect()
    }
}

fn row_id(i: usize) -> String {
    format!("v{i:09}")
}

/// Build one NDJSON ingest body for rows [start, end).
fn ndjson_batch(dataset: &[Vec<f32>], start: usize, end: usize, n_clusters: usize) -> String {
    let mut out = String::with_capacity((end - start) * 64);
    for (offset, vec) in dataset[start..end].iter().enumerate() {
        let i = start + offset;
        let _ = write!(
            out,
            r#"{{"id":"{}","text":"synthetic vector row {} cluster {}","embedding":["#,
            row_id(i),
            i,
            i % n_clusters
        );
        for (j, x) in vec.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            let _ = write!(out, "{x}");
        }
        out.push_str("]}\n");
    }
    out
}

/// `make_array(0.1, -0.2, ...)` — Display on f32 prints the shortest
/// round-trip representation, and the engine UDF casts f64 literals back to
/// f32, so the engine sees bit-identical query vectors.
fn make_array_sql(q: &[f32]) -> String {
    let mut s = String::with_capacity(q.len() * 12 + 12);
    s.push_str("make_array(");
    for (j, x) in q.iter().enumerate() {
        if j > 0 {
            s.push(',');
        }
        let _ = write!(s, "{x}");
    }
    s.push(')');
    s
}

const N_CLUSTERS: usize = 64;
const QUERY_PERTURB: f64 = 0.05;

fn run(args: &Args) -> Result<Report> {
    let engine = Engine::new(&args.engine_url, args.auth_token.clone())?;
    engine.health()?;

    let n = args.tier.rows();
    eprintln!(
        "[bench] generating {} vectors (dim={}, clusters={}, seed={})",
        n, args.dim, N_CLUSTERS, args.seed
    );
    let dataset = clustered_gaussian(n, args.dim, N_CLUSTERS, args.seed);
    let queries = queries_from_dataset(&dataset, args.queries, QUERY_PERTURB, args.seed ^ 0x5EED);

    // ── Ingest ──
    let ingest_start = Instant::now();
    if args.skip_ingest {
        eprintln!("[bench] --skip-ingest: assuming {n} rows already loaded");
    } else {
        let mut start = 0;
        while start < n {
            let end = (start + args.batch_size).min(n);
            let body = ndjson_batch(&dataset, start, end, N_CLUSTERS);
            engine
                .ingest_ndjson(&args.db, &args.table, body)
                .with_context(|| format!("ingest batch rows {start}..{end}"))?;
            start = end;
            if start % (args.batch_size * 20) == 0 || start == n {
                eprintln!("[bench] ingested {start}/{n}");
            }
        }
    }
    let ingest_secs = ingest_start.elapsed().as_secs_f64();

    // Row-count sanity: the recall comparison is meaningless if the engine
    // is missing (or duplicating) rows.
    let count_rows = engine.query_sql(
        &args.db,
        &format!("SELECT COUNT(*) AS c FROM {}", args.table),
    )?;
    let engine_count = count_rows
        .first()
        .and_then(|r| r.get("c"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("COUNT(*) returned no parseable row: {count_rows:?}"))?;
    if engine_count != n as u64 {
        bail!("engine row count {engine_count} != expected {n} (stale table? rerun without --skip-ingest on a fresh db)");
    }

    // ── Queries vs oracle ──
    let mut recalls = Vec::with_capacity(queries.len());
    let mut latencies = Vec::with_capacity(queries.len());
    for (qi, q) in queries.iter().enumerate() {
        let truth: Vec<String> = ExactOracle::top_k(&dataset, q, args.k, Metric::Cosine)
            .into_iter()
            .map(|(idx, _)| row_id(idx))
            .collect();

        let sql = format!(
            "SELECT id, cosine_distance(embedding, {arr}) AS d FROM {tbl} ORDER BY d ASC, id ASC LIMIT {k}",
            arr = make_array_sql(q),
            tbl = args.table,
            k = args.k,
        );
        let t0 = Instant::now();
        let rows = engine.query_sql(&args.db, &sql)?;
        latencies.push(t0.elapsed());

        let got: Vec<String> = rows
            .iter()
            .filter_map(|r| r.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .collect();
        let r = recall_at_k(&got, &truth, args.k);
        recalls.push(r);
        if (qi + 1) % 25 == 0 {
            eprintln!("[bench] {}/{} queries (running recall {:.4})", qi + 1, queries.len(), mean(&recalls));
        }
    }

    Ok(Report {
        tier: args.tier.as_str().to_string(),
        n,
        dim: args.dim,
        queries: args.queries,
        k: args.k,
        seed: args.seed,
        recall_at_10: mean(&recalls),
        recall_min: recalls.iter().copied().fold(f64::INFINITY, f64::min),
        latency: LatencyStats::from_durations(&latencies),
        ingest_secs,
        timestamp: chrono::Utc::now().to_rfc3339(),
        engine_url: args.engine_url.clone(),
    })
}

fn main() -> Result<()> {
    let args = Args::parse();
    let report = run(&args)?;
    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");
    if let Some(path) = &args.out {
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("writing report to {}", path.display()))?;
        eprintln!("[bench] report written to {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_defaults_parse() {
        let args = Args::try_parse_from(["retrieval_bench"]).unwrap();
        assert_eq!(args.queries, 100);
        assert_eq!(args.k, 10);
        assert_eq!(args.dim, 384);
        assert_eq!(args.db, "retrieval_bench");
        assert_eq!(args.table, "vectors");
        assert!(matches!(args.tier, Tier::Pr));
        assert_eq!(args.tier.rows(), 100_000);
    }

    #[test]
    fn args_full_parse() {
        let args = Args::try_parse_from([
            "retrieval_bench",
            "--tier=nightly",
            "--engine-url=http://localhost:9999",
            "--queries=5",
            "--k=20",
            "--out=/tmp/r.json",
            "--skip-ingest",
        ])
        .unwrap();
        assert!(matches!(args.tier, Tier::Nightly));
        assert_eq!(args.tier.rows(), 1_000_000);
        assert_eq!(args.queries, 5);
        assert_eq!(args.k, 20);
        assert!(args.skip_ingest);
        assert_eq!(args.out.unwrap().to_str().unwrap(), "/tmp/r.json");
    }

    #[test]
    fn args_reject_bad_tier() {
        assert!(Args::try_parse_from(["retrieval_bench", "--tier=weekly"]).is_err());
    }

    #[test]
    fn engine_url_normalization() {
        let e = Engine::new("127.0.0.1:8080", None).unwrap();
        assert_eq!(e.base, "http://127.0.0.1:8080");
        let e = Engine::new("https://kyma.example.com/", Some("t".into())).unwrap();
        assert_eq!(e.base, "https://kyma.example.com");
    }

    #[test]
    fn ndjson_batch_shape() {
        let data = vec![vec![1.0_f32, -0.5], vec![0.25_f32, 0.75]];
        let body = ndjson_batch(&data, 0, 2, 64);
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        let row: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(row["id"], "v000000000");
        // Compare numerically: Display prints `1` for 1.0_f32, which serde
        // reads back as an integer Number — same f64 value on the wire.
        let emb: Vec<f64> = row["embedding"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(emb, vec![1.0, -0.5]);
        assert!(row["text"].as_str().unwrap().contains("cluster 0"));
    }

    #[test]
    fn make_array_round_trips_f32() {
        let q = [0.1_f32, -2.5e-7, 1.0];
        let sql = make_array_sql(&q);
        assert!(sql.starts_with("make_array(") && sql.ends_with(')'));
        let inner = &sql["make_array(".len()..sql.len() - 1];
        for (tok, orig) in inner.split(',').zip(q.iter()) {
            #[allow(clippy::cast_possible_truncation)]
            let back = tok.parse::<f64>().unwrap() as f32;
            assert_eq!(back.to_bits(), orig.to_bits());
        }
    }
}
