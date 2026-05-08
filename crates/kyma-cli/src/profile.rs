//! Single-profile credential storage at `~/.config/kyma/credentials.toml`.
//! Slice 4 will turn this into a multi-profile `profiles.toml`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub api_url: Option<String>,
    pub token: Option<String>,
    pub workspace_slug: Option<String>,
    pub workspace_id: Option<String>,
    pub mcp_endpoint: Option<String>,
}

fn path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("no config dir")?.join("kyma");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("credentials.toml"))
}

pub fn load() -> Result<Credentials> {
    let p = path()?;
    if !p.exists() {
        return Ok(Credentials::default());
    }
    let bytes = fs::read_to_string(&p)?;
    Ok(toml::from_str(&bytes)?)
}

pub fn save(c: &Credentials) -> Result<()> {
    let p = path()?;
    let s = toml::to_string_pretty(c)?;
    fs::write(&p, s)?;
    Ok(())
}
