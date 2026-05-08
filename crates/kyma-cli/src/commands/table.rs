//! `kyma table list --db <db>`.

use crate::api_client::ApiClient;
use crate::profile::load;
use anyhow::{anyhow, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum TableCmd {
    /// List tables in a database (in the currently-selected workspace).
    List {
        #[arg(long)]
        db: String,
    },
}

pub async fn run(cmd: TableCmd, api_url: &str) -> Result<()> {
    let creds = load()?;
    let token = creds.token.clone().ok_or_else(|| anyhow!("not logged in"))?;
    let slug = creds
        .workspace_slug
        .clone()
        .ok_or_else(|| anyhow!("no workspace selected"))?;
    let url = creds.api_url.unwrap_or_else(|| api_url.to_string());
    let api = ApiClient::new(url, Some(token));
    match cmd {
        TableCmd::List { db } => {
            for t in api.list_tables(&slug, &db).await? {
                println!("{}  [{}]", t.name, t.columns.join(", "));
            }
        }
    }
    Ok(())
}
