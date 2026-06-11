//! Confluence data source — spaces, pages, authors, and the page tree as a graph.
//!
//! Authenticates with a stored Atlassian OAuth2 token (shares the `cloudId`
//! resolution with the Jira data source). Fetches the most recently modified
//! pages (bounded) via CQL search. Ids are prefixed `conf:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const EXPAND: &str = "space,version,ancestors,history";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfluenceConfig {
    #[serde(default)]
    credential_id: Option<Uuid>,
    #[serde(default)]
    cloud_id: Option<String>,
    /// Optional: restrict to these space keys. Empty = all accessible.
    #[serde(default)]
    spaces: Vec<String>,
    #[serde(default = "default_max_pages")]
    max_pages_per_tick: usize,
}
fn default_max_pages() -> usize {
    5
}

pub struct ConfluenceDataSource;

#[async_trait]
impl DataSource for ConfluenceDataSource {
    fn type_id(&self) -> &'static str {
        "confluence"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "confluence".into(),
            label: "Confluence".into(),
            category: "knowledge".into(),
            description: "Spaces, pages, authors, and the page hierarchy from Atlassian \
                          Confluence."
                .into(),
            brand: "confluence".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            default_schedule_ms: 10 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("confluence_nodes".into()),
            config_defaults: None,
            graph_name: Some("confluence".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: ConfluenceConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parsed.credential_id.is_none() {
            return Err(ConfigError("an Atlassian OAuth credential is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let config: ConfluenceConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        let cloud_id = match config
            .cloud_id
            .clone()
            .or_else(|| cursor.and_then(|c| c.get("cloud_id")).and_then(Value::as_str).map(String::from))
        {
            Some(c) => c,
            None => crate::oauth_http::resolve_atlassian_cloud_id(&ctx.http, &token).await?,
        };
        let base = format!("https://api.atlassian.com/ex/confluence/{cloud_id}/wiki/rest/api");

        let cql = if config.spaces.is_empty() {
            "type=page ORDER BY lastmodified DESC".to_string()
        } else {
            let quoted: Vec<String> = config.spaces.iter().map(|s| format!("\"{s}\"")).collect();
            format!("type=page AND space in ({}) ORDER BY lastmodified DESC", quoted.join(","))
        };

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        let mut start = 0usize;
        let limit = 50usize;
        for _ in 0..config.max_pages_per_tick {
            let mut url = reqwest::Url::parse(&format!("{base}/content/search"))
                .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
            {
                let mut p = url.query_pairs_mut();
                p.append_pair("cql", &cql);
                p.append_pair("limit", &limit.to_string());
                p.append_pair("start", &start.to_string());
                p.append_pair("expand", EXPAND);
            }
            let resp = crate::oauth_http::get_json(&ctx.http, url.as_str(), &token, &[]).await?;
            let results = resp.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
            if results.is_empty() {
                break;
            }
            for page in &results {
                emit_page(page, &mut nodes, &mut edges, &mut user_seen);
            }
            let size = resp.get("size").and_then(Value::as_u64).unwrap_or(0) as usize;
            start += size.max(results.len());
            if size < limit && results.len() < limit {
                break;
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "cloud_id": cloud_id })),
            tables: vec![
                TableRows { table: "confluence_nodes".into(), rows: nodes },
                TableRows { table: "confluence_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "confluence".into(),
                node_table: "confluence_nodes".into(),
                edge_table: "confluence_edges".into(),
            }),
        })
    }
}

fn emit_page(
    page: &Value,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    let id = page.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return;
    }
    let page_id = format!("conf:page:{id}");
    nodes.push(json!({
        "id": page_id, "labels": ["Page"],
        "name": page.get("title").and_then(Value::as_str).unwrap_or("Untitled"),
        "status": page.get("status").cloned().unwrap_or(Value::Null),
        "version": page.get("version").and_then(|v| v.get("number")).cloned().unwrap_or(Value::Null),
        "vendor": "confluence",
    }));

    // Space containment.
    if let Some(skey) = page.get("space").and_then(|s| s.get("key")).and_then(Value::as_str) {
        let space_id = format!("conf:space:{skey}");
        nodes.push(json!({
            "id": space_id, "labels": ["Space"],
            "name": page.get("space").and_then(|s| s.get("name")).and_then(Value::as_str).unwrap_or(skey),
            "vendor": "confluence",
        }));
        edges.push(json!({
            "id": format!("e:contains:{space_id}::{page_id}"),
            "src": space_id, "dst": page_id, "type": "CONTAINS",
        }));
    }

    // Immediate parent = last ancestor.
    if let Some(parent) = page
        .get("ancestors")
        .and_then(Value::as_array)
        .and_then(|a| a.last())
        .and_then(|p| p.get("id"))
        .and_then(Value::as_str)
    {
        let parent_id = format!("conf:page:{parent}");
        edges.push(json!({
            "id": format!("e:child_of:{page_id}::{parent_id}"),
            "src": page_id, "dst": parent_id, "type": "CHILD_OF",
        }));
    }

    // Author (history.createdBy) + last editor (version.by).
    let authors = [
        (page.get("history").and_then(|h| h.get("createdBy")), "CREATED_BY"),
        (page.get("version").and_then(|v| v.get("by")), "LAST_EDITED_BY"),
    ];
    for (who, rel) in authors {
        if let Some(u) = who {
            if let Some(uid) = u.get("accountId").and_then(Value::as_str) {
                let user_id = format!("conf:user:{uid}");
                if user_seen.insert(user_id.clone()) {
                    nodes.push(json!({
                        "id": user_id, "labels": ["User"],
                        "name": u.get("displayName").and_then(Value::as_str).unwrap_or(uid),
                        "vendor": "confluence",
                    }));
                }
                edges.push(json!({
                    "id": format!("e:{}:{page_id}::{user_id}", rel.to_lowercase()),
                    "src": page_id, "dst": user_id, "type": rel,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_page_graph() {
        let page = json!({
            "id": "123", "title": "Runbook", "status": "current",
            "space": { "key": "ENG", "name": "Engineering" },
            "version": { "number": 4, "by": { "accountId": "u2", "displayName": "Bob" } },
            "ancestors": [{ "id": "100" }, { "id": "110" }],
            "history": { "createdBy": { "accountId": "u1", "displayName": "Ada" } }
        });
        let (mut n, mut e, mut seen) = (vec![], vec![], HashSet::new());
        emit_page(&page, &mut n, &mut e, &mut seen);
        let types: Vec<&str> = e.iter().filter_map(|x| x["type"].as_str()).collect();
        assert!(types.contains(&"CONTAINS"));
        assert!(types.contains(&"CREATED_BY"));
        assert!(types.contains(&"LAST_EDITED_BY"));
        // immediate parent is the last ancestor (110)
        assert!(e.iter().any(|x| x["type"] == "CHILD_OF" && x["dst"] == "conf:page:110"));
    }
}
