//! `kyma ingest --db <db> --table <t> <file>`.

use crate::api_client::ApiClient;
use crate::profile::load;
use anyhow::{anyhow, Result};
use std::fs;

pub async fn run(api_url: &str, db: &str, table: &str, file: &str) -> Result<()> {
    let creds = load()?;
    let token = creds.token.clone().ok_or_else(|| anyhow!("not logged in"))?;
    let slug = creds
        .workspace_slug
        .clone()
        .ok_or_else(|| anyhow!("no workspace selected"))?;
    let url = creds.api_url.unwrap_or_else(|| api_url.to_string());

    let bytes = fs::read_to_string(file)?;
    let api = ApiClient::new(url, Some(token));
    api.ingest(&slug, db, table, &bytes).await?;
    let lines = bytes.lines().filter(|l| !l.trim().is_empty()).count();
    println!("ingested {lines} rows into {db}.{table} (workspace {slug})");
    Ok(())
}
