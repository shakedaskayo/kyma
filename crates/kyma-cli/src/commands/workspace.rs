//! `kyma workspace {list,select}`.

use crate::api_client::ApiClient;
use crate::profile::{load, save};
use anyhow::{anyhow, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd {
    /// List workspaces visible to the current credential.
    List,
    /// Select a workspace by slug; saved into ~/.config/kyma/credentials.toml.
    Select { slug: String },
}

pub async fn run(cmd: WorkspaceCmd, api_url: &str) -> Result<()> {
    let creds = load().unwrap_or_default();
    let token = creds.token.clone();
    let api = ApiClient::new(api_url.to_string(), token);

    match cmd {
        WorkspaceCmd::List => {
            let workspaces = api.list_workspaces().await?;
            if workspaces.is_empty() {
                println!("No workspaces found.");
                return Ok(());
            }
            let current = creds.workspace_slug.as_deref().unwrap_or("");
            for ws in &workspaces {
                let marker = if ws.slug == current { "*" } else { " " };
                println!("{marker} {} ({})  [{}]", ws.name, ws.slug, ws.plan);
            }
        }
        WorkspaceCmd::Select { slug } => {
            let workspaces = api.list_workspaces().await?;
            let ws = workspaces
                .into_iter()
                .find(|w| w.slug == slug)
                .ok_or_else(|| anyhow!("workspace '{}' not found", slug))?;

            let mut creds = creds;
            creds.workspace_slug = Some(ws.slug.clone());
            creds.workspace_id = Some(ws.id.clone());
            creds.mcp_endpoint = Some(ws.mcp_endpoint.clone());
            save(&creds)?;
            println!("Selected workspace: {} ({})", ws.name, ws.slug);
        }
    }
    Ok(())
}
