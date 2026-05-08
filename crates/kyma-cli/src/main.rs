//! kyma CLI.
//!
//! Slice 2 retrofit: this is now primarily an HTTPS client for the Kyma cloud
//! control-plane (`cloud.kyma.dev/api`). The legacy direct-Postgres admin
//! commands are preserved under `kyma-cli db <cmd>` so the bootstrap script
//! that ships in the runtime Docker image (`kyma-bootstrap.sh`) and any
//! self-hosted users keep working unchanged.

mod api_client;
mod commands;
mod profile;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kyma-cli", about = "kyma CLI")]
struct Cli {
    /// Cloud control-plane base URL (used by login/workspace/ingest/table).
    #[arg(long, env = "KYMA_API_URL", default_value = "https://cloud.kyma.dev")]
    api_url: String,

    /// Postgres connection URL (used only by legacy `db` subcommands).
    #[arg(
        long,
        env = "KYMA_CATALOG_URL",
        default_value = "postgres://kyma:kyma_dev@localhost:5433/kyma"
    )]
    catalog_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Authenticate with cloud.kyma.dev via browser flow.
    Login,
    /// Manage workspaces visible to the current credential.
    Workspace {
        #[command(subcommand)]
        cmd: commands::workspace::WorkspaceCmd,
    },
    /// Ingest an NDJSON file into a table in the selected workspace.
    Ingest {
        #[arg(long)]
        db: String,
        #[arg(long)]
        table: String,
        file: String,
    },
    /// Inspect tables in the selected workspace.
    Table {
        #[command(subcommand)]
        cmd: commands::table::TableCmd,
    },
    /// Legacy direct-Postgres admin commands (used by kyma-bootstrap.sh).
    Db {
        #[command(subcommand)]
        cmd: commands::legacy::DbCmd,
    },
    /// Print the CLI version.
    Version,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Login => commands::login::run(&cli.api_url).await,
        Command::Workspace { cmd } => commands::workspace::run(cmd, &cli.api_url).await,
        Command::Ingest { db, table, file } => {
            commands::ingest::run(&cli.api_url, &db, &table, &file).await
        }
        Command::Table { cmd } => commands::table::run(cmd, &cli.api_url).await,
        Command::Db { cmd } => commands::legacy::run(cmd, &cli.catalog_url).await,
        Command::Version => {
            println!("kyma-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
