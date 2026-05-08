//! `kyma workspace {list,select}`. Real implementation lands in Task 28.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd {
    /// List workspaces visible to the current credential.
    List,
    /// Select a workspace by slug; saved into ~/.config/kyma/credentials.toml.
    Select { slug: String },
}

pub async fn run(_cmd: WorkspaceCmd, _api_url: &str) -> anyhow::Result<()> {
    unimplemented!("Task 28")
}
