//! `kyma-local` — the single-binary **context engine** for local machines.
//!
//! One binary, zero infra: an embedded **SQLite catalog** + a **local-filesystem
//! object store** + the in-process columnar engine + the **stdio MCP** server.
//! No Postgres, no MinIO, no HTTP, no auth. A coding agent (Claude Code, Cursor,
//! Windsurf, …) spawns `kyma-local mcp` and gets the full toolset over
//! stdin/stdout: durable graph-aware **memory** (`memory_search` / `save_memory`
//! / …) *and* live **data/graph** tools (`run_kql` / `run_sql` /
//! `graph_traverse` / …) — the same tools the hosted server exposes.
//!
//! Data lives under `~/.kyma` (override with `KYMA_HOME` / `KYMA_LOCAL_DB` /
//! `KYMA_LOCAL_DATA`):
//!   - `~/.kyma/catalog.db`  — embedded catalog (metadata, memory graph)
//!   - `~/.kyma/data/`       — columnar extents (object store)
//!
//! **stdout is the MCP protocol channel** — all logs go to stderr.

#![forbid(unsafe_code)]

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_format_tlm::TelemetryFormat;
use kyma_mcp::{serve_stdio, McpState, ServerInfo, ToolDispatch};
use kyma_server::agent::SharedToolCtx;
use kyma_storage::{build_object_store, StorageConfig};
use tracing::info;

#[derive(Debug, Parser)]
#[command(
    name = "kyma-local",
    about = "kyma local — single-binary context engine (memory + live data + graph) over stdio MCP",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the Model Context Protocol over stdio (default). Point any coding
    /// agent's MCP config at this command.
    Mcp,
    /// Print the resolved local paths and exit (diagnostics).
    Info,
}

/// Resolved on-disk locations for the local engine.
struct Paths {
    catalog_db: String,
    data_root: String,
}

fn resolve_paths() -> Paths {
    let home = std::env::var("KYMA_HOME").unwrap_or_else(|_| {
        let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{base}/.kyma")
    });
    let catalog_db =
        std::env::var("KYMA_LOCAL_DB").unwrap_or_else(|_| format!("{home}/catalog.db"));
    let data_root =
        std::env::var("KYMA_LOCAL_DATA").unwrap_or_else(|_| format!("{home}/data"));
    Paths { catalog_db, data_root }
}

#[tokio::main]
async fn main() -> Result<()> {
    // CRITICAL: logs to stderr — stdout is reserved for the MCP JSON-RPC channel.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let paths = resolve_paths();

    if matches!(cli.command, Some(Command::Info)) {
        eprintln!("kyma-local — single-binary context engine");
        eprintln!("  catalog : {}", paths.catalog_db);
        eprintln!("  data    : {}", paths.data_root);
        eprintln!("  serve   : kyma-local mcp   (stdio MCP; memory + data + graph tools)");
        return Ok(());
    }

    // Ensure the data directories exist before opening the catalog / store.
    if let Some(parent) = std::path::Path::new(&paths.catalog_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(&paths.data_root)
        .with_context(|| format!("creating data root {}", paths.data_root))?;

    // 1. Embedded catalog (metadata + memory graph).
    let catalog: Arc<dyn Catalog> = Arc::new(
        SqliteCatalog::connect(&paths.catalog_db)
            .await
            .map_err(|e| anyhow::anyhow!("opening catalog at {}: {e}", paths.catalog_db))?,
    );
    info!(catalog = %paths.catalog_db, "embedded catalog ready");

    // 2. Local-filesystem object store + columnar format.
    let store = build_object_store(&StorageConfig::Local { root: paths.data_root.clone() })
        .context("building local object store")?;
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "kyma-local"));
    info!(data = %paths.data_root, "local object store ready");

    // 3. The context-engine toolset over stdio — no Postgres pool (local mode).
    let shared = SharedToolCtx { catalog, format, pool: None };
    let state = McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo {
            name: "kyma-local".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };

    info!("serving MCP over stdio (memory + data + graph); stdin/stdout is the protocol channel");
    serve_stdio(state).await.context("stdio MCP loop")?;
    Ok(())
}
