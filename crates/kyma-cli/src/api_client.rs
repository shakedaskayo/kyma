//! Thin HTTPS client for the Kyma cloud control-plane API.
//!
//! Slice 2: workspace listing, ingest, and table listing. Real command
//! bodies in T27/T28/T29 — this module is just the transport.

use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;

pub struct ApiClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base: String, token: Option<String>) -> Self {
        Self {
            base,
            token,
            http: reqwest::Client::new(),
        }
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(t) = &self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {t}"));
        }
        req
    }

    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceRow>> {
        #[derive(Deserialize)]
        struct Wrap {
            workspaces: Vec<WorkspaceRow>,
        }
        let res = self
            .auth(self.http.get(format!("{}/api/workspaces", self.base)))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(anyhow!("HTTP {}", res.status()));
        }
        Ok(res.json::<Wrap>().await?.workspaces)
    }

    pub async fn ingest(
        &self,
        workspace_slug: &str,
        database: &str,
        table: &str,
        ndjson: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/api/workspaces/{}/ingest/{}/{}",
            self.base, workspace_slug, database, table
        );
        let res = self
            .auth(
                self.http
                    .post(url)
                    .header(CONTENT_TYPE, "application/x-ndjson")
                    .body(ndjson.to_owned()),
            )
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(anyhow!("ingest HTTP {}", res.status()));
        }
        Ok(())
    }

    pub async fn list_tables(&self, workspace_slug: &str, db: &str) -> Result<Vec<TableRow>> {
        #[derive(Deserialize)]
        struct Wrap {
            tables: Vec<TableRow>,
        }
        let res = self
            .auth(self.http.get(format!(
                "{}/api/workspaces/{}/databases/{}/tables",
                self.base, workspace_slug, db
            )))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(anyhow!("HTTP {}", res.status()));
        }
        Ok(res.json::<Wrap>().await?.tables)
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub plan: String,
    pub kind: String,
    pub mcp_endpoint: String,
    pub kyma_endpoint: String,
}

#[derive(Debug, Deserialize)]
pub struct TableRow {
    pub name: String,
    pub columns: Vec<String>,
}
