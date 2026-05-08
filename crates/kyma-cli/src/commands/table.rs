//! `kyma table list --db <db>`. Real impl in Task 29.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum TableCmd {
    /// List tables in a database (in the currently-selected workspace).
    List {
        #[arg(long)]
        db: String,
    },
}

pub async fn run(_cmd: TableCmd, _api_url: &str) -> anyhow::Result<()> {
    unimplemented!("Task 29")
}
