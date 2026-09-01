//! Load a [`TestGraph`] into a running pensieve.
//!
//! Mirrors the house end-to-end flow (`scripts/test-graph.sh`):
//! 1. `pensieve-cli create-database <db>` + `create-table` with explicit schemas
//!    (ingest's auto-create would otherwise land the columns in the dynamic
//!    `props` catch-all, invisible to `graph-match` SQL).
//! 2. `POST /v1/ingest` NDJSON with `X-Database` / `X-Table` headers, one
//!    object per node/edge row, chunked.
//! 3. `pensieve-cli create-graph --db <db> --name <name> --nodes <name>_nodes
//!    --edges <name>_edges` — the column names below match the CLI's
//!    [`GraphSpec`] defaults (`id`/`labels`/`src`/`dst`/`type`), so no column
//!    overrides are needed.
//!
//! Table shapes (identical to the conventional stored-graph layout used in
//! `cypher_query_it.rs` and the data-source graphs):
//! - `<name>_nodes(id:string, labels:string, name:string)` — `labels` holds
//!   the node's **primary** (first) label only; Cypher `:Label` filters are
//!   string equality on this column.
//! - `<name>_edges(src:string, dst:string, type:string)`.

use anyhow::{bail, Context};

use crate::model::TestGraph;

/// Where the engine + its admin CLI live. The CLI reads `PENSIEVE_CATALOG_URL`
/// from the environment itself (registration is a catalog write — there is no
/// HTTP endpoint for `create-graph`; the CLI *is* how the codebase does it).
pub struct EngineConfig {
    /// HTTP base, e.g. `http://127.0.0.1:8080`.
    pub base_url: String,
    /// Target database.
    pub database: String,
    /// Optional bearer token for `/v1/ingest`.
    pub token: Option<String>,
    /// Path to the `pensieve-cli` binary.
    pub cli_bin: String,
}

impl EngineConfig {
    /// From env: `PENSIEVE_BASE_URL` (default `http://127.0.0.1:8080`),
    /// `PENSIEVE_DATABASE` (default `default`), `PENSIEVE_TOKEN`,
    /// `PENSIEVE_CLI_BIN` (default `./target/debug/pensieve-cli`).
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("PENSIEVE_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
            database: std::env::var("PENSIEVE_DATABASE").unwrap_or_else(|_| "default".into()),
            token: std::env::var("PENSIEVE_TOKEN").ok(),
            cli_bin: std::env::var("PENSIEVE_CLI_BIN")
                .unwrap_or_else(|_| "./target/debug/pensieve-cli".into()),
        }
    }

    fn run_cli(&self, args: &[&str]) -> anyhow::Result<String> {
        let out = std::process::Command::new(&self.cli_bin)
            .args(args)
            .output()
            .with_context(|| format!("spawn {} {args:?}", self.cli_bin))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() {
            bail!("{cli} {args:?} failed: {stdout}{stderr}", cli = self.cli_bin);
        }
        Ok(stdout)
    }
}

/// NDJSON rows for the node table. `labels` = primary label (see module docs).
fn nodes_ndjson(g: &TestGraph) -> Vec<String> {
    g.nodes
        .iter()
        .map(|n| {
            let name = n
                .props
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(n.id.as_str());
            serde_json::json!({
                "id": n.id,
                "labels": n.primary_label().unwrap_or(""),
                "name": name,
            })
            .to_string()
        })
        .collect()
}

/// NDJSON rows for the edge table.
fn edges_ndjson(g: &TestGraph) -> Vec<String> {
    g.edges
        .iter()
        .map(|e| {
            serde_json::json!({ "src": e.src, "dst": e.dst, "type": e.etype }).to_string()
        })
        .collect()
}

async fn ingest_table(
    cfg: &EngineConfig,
    client: &reqwest::Client,
    table: &str,
    lines: &[String],
) -> anyhow::Result<()> {
    const CHUNK: usize = 4000;
    for chunk in lines.chunks(CHUNK) {
        let body = chunk.join("\n");
        let mut req = client
            .post(format!("{}/v1/ingest", cfg.base_url))
            .header("X-Database", &cfg.database)
            .header("X-Table", table)
            .header("Content-Type", "application/x-ndjson")
            .body(body);
        if let Some(t) = &cfg.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.context("POST /v1/ingest")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("ingest into {table} failed: HTTP {status}: {body}");
        }
    }
    Ok(())
}

/// Create tables, ingest the graph, and register it as stored graph `name`
/// over `{name}_nodes` / `{name}_edges` in `cfg.database`.
///
/// Idempotent enough for a fresh stack: `create-database` on an existing
/// database is tolerated; pre-existing tables/graphs of the same name are
/// not (run against a reset stack, as the gate script does).
pub async fn load_graph(cfg: &EngineConfig, g: &TestGraph, name: &str) -> anyhow::Result<()> {
    let nodes_table = format!("{name}_nodes");
    let edges_table = format!("{name}_edges");

    // 1. Provision. create-database errors if it already exists — tolerate
    //    only that case (subsequent steps fail loudly if something is wrong).
    if let Err(e) = cfg.run_cli(&["create-database", &cfg.database]) {
        eprintln!("note: create-database: {e:#} (continuing — likely already exists)");
    }
    cfg.run_cli(&[
        "create-table",
        "--db",
        &cfg.database,
        "--name",
        &nodes_table,
        "--schema",
        "id:string,labels:string,name:string",
    ])?;
    cfg.run_cli(&[
        "create-table",
        "--db",
        &cfg.database,
        "--name",
        &edges_table,
        "--schema",
        "src:string,dst:string,type:string",
    ])?;

    // 2. Ingest rows.
    let client = reqwest::Client::new();
    ingest_table(cfg, &client, &nodes_table, &nodes_ndjson(g)).await?;
    ingest_table(cfg, &client, &edges_table, &edges_ndjson(g)).await?;

    // 3. Register the stored graph (column names match GraphSpec defaults).
    cfg.run_cli(&[
        "create-graph",
        "--db",
        &cfg.database,
        "--name",
        name,
        "--nodes",
        &nodes_table,
        "--edges",
        &edges_table,
    ])?;
    Ok(())
}
