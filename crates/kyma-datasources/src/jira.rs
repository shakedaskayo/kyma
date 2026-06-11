//! Jira data source — projects, issues, epics, and their links as a graph.
//!
//! Authenticates with a stored Atlassian OAuth2 token; resolves the site
//! `cloudId` from the token's accessible resources. Fetches the most recently
//! updated issues (bounded) via the Jira Cloud REST API. Uses the same node
//! labels (Issue, User, Project) as the code data sources so cross-vendor views
//! line up. Ids are prefixed `jira:`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use crate::catalog::CatalogEntry;
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const FIELDS: &str =
    "summary,status,assignee,reporter,issuetype,parent,issuelinks,project,updated";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JiraConfig {
    #[serde(default)]
    credential_id: Option<Uuid>,
    /// Atlassian site cloud id (resolved from the token if omitted).
    #[serde(default)]
    cloud_id: Option<String>,
    /// Optional: restrict to these project keys. Empty = all accessible.
    #[serde(default)]
    projects: Vec<String>,
    /// Max `/search` pages fetched per tick (each ≤ 100 issues).
    #[serde(default = "default_max_pages")]
    max_pages_per_tick: usize,
}
fn default_max_pages() -> usize {
    5
}

pub struct JiraConnector;

#[async_trait]
impl DataSource for JiraConnector {
    fn type_id(&self) -> &'static str {
        "jira"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "jira".into(),
            label: "Jira".into(),
            category: "project".into(),
            description: "Projects, issues, epics, assignees, and issue links from Jira Cloud."
                .into(),
            brand: "jira".into(),
            auth_mode: "oauth".into(),
            status: "available".into(),
            default_schedule_ms: 10 * 60_000,
            fields: Vec::new(),
            resource: None,
            default_target_table: Some("jira_nodes".into()),
            config_defaults: None,
            graph_name: Some("jira".into()),
            accepted_credential_kinds: vec!["oauth2".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: JiraConfig =
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
        let config: JiraConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;
        let cid = config
            .credential_id
            .ok_or_else(|| DataSourceError::Config("credential_id required".into()))?;
        let token = crate::oauth::valid_access_token(ctx, cid).await?;

        // cloudId: config → cursor cache → resolve from token.
        let cloud_id = match config
            .cloud_id
            .clone()
            .or_else(|| cursor.and_then(|c| c.get("cloud_id")).and_then(Value::as_str).map(String::from))
        {
            Some(c) => c,
            None => crate::oauth_http::resolve_atlassian_cloud_id(&ctx.http, &token).await?,
        };
        let base = format!("https://api.atlassian.com/ex/jira/{cloud_id}/rest/api/3");

        let jql = if config.projects.is_empty() {
            "ORDER BY updated DESC".to_string()
        } else {
            format!("project in ({}) ORDER BY updated DESC", config.projects.join(","))
        };

        let mut nodes: Vec<Value> = Vec::new();
        let mut edges: Vec<Value> = Vec::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        let mut start_at = 0usize;
        for _ in 0..config.max_pages_per_tick {
            let mut url = reqwest::Url::parse(&format!("{base}/search"))
                .map_err(|e| DataSourceError::Permanent(format!("url: {e}")))?;
            {
                let mut p = url.query_pairs_mut();
                p.append_pair("jql", &jql);
                p.append_pair("startAt", &start_at.to_string());
                p.append_pair("maxResults", "100");
                p.append_pair("fields", FIELDS);
            }
            let resp = crate::oauth_http::get_json(&ctx.http, url.as_str(), &token, &[]).await?;
            let issues = resp.get("issues").and_then(Value::as_array).cloned().unwrap_or_default();
            if issues.is_empty() {
                break;
            }
            for issue in &issues {
                emit_issue(issue, &mut nodes, &mut edges, &mut user_seen);
            }
            start_at += issues.len();
            let total = resp.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
            if start_at >= total {
                break;
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: Some(json!({ "cloud_id": cloud_id })),
            tables: vec![
                TableRows { table: "jira_nodes".into(), rows: nodes },
                TableRows { table: "jira_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "jira".into(),
                node_table: "jira_nodes".into(),
                edge_table: "jira_edges".into(),
            }),
        })
    }
}

fn emit_issue(
    issue: &Value,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    user_seen: &mut HashSet<String>,
) {
    let key = issue.get("key").and_then(Value::as_str).unwrap_or("");
    if key.is_empty() {
        return;
    }
    let f = issue.get("fields").cloned().unwrap_or(Value::Null);
    let issue_id = format!("jira:issue:{key}");
    let summary = f.get("summary").and_then(Value::as_str).unwrap_or("");
    let issuetype = f
        .get("issuetype")
        .and_then(|t| t.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let label = if issuetype.eq_ignore_ascii_case("epic") { "Epic" } else { "Issue" };
    nodes.push(json!({
        "id": issue_id, "labels": [label], "name": format!("{key} {summary}"),
        "status": f.get("status").and_then(|s| s.get("name")).cloned().unwrap_or(Value::Null),
        "issuetype": issuetype,
        "updated": f.get("updated").cloned().unwrap_or(Value::Null),
        "vendor": "jira",
    }));

    // Project containment.
    if let Some(pkey) = f.get("project").and_then(|p| p.get("key")).and_then(Value::as_str) {
        let proj_id = format!("jira:proj:{pkey}");
        nodes.push(json!({
            "id": proj_id, "labels": ["Project"],
            "name": f.get("project").and_then(|p| p.get("name")).and_then(Value::as_str).unwrap_or(pkey),
            "vendor": "jira",
        }));
        edges.push(json!({
            "id": format!("e:has_issue:{proj_id}::{issue_id}"),
            "src": proj_id, "dst": issue_id, "type": "HAS_ISSUE",
        }));
    }

    for (field, rel) in [("assignee", "ASSIGNED_TO"), ("reporter", "REPORTED_BY")] {
        if let Some(u) = f.get(field) {
            if let Some(uid) = u.get("accountId").and_then(Value::as_str) {
                let user_id = format!("jira:user:{uid}");
                if user_seen.insert(user_id.clone()) {
                    nodes.push(json!({
                        "id": user_id, "labels": ["User"],
                        "name": u.get("displayName").and_then(Value::as_str).unwrap_or(uid),
                        "vendor": "jira",
                    }));
                }
                edges.push(json!({
                    "id": format!("e:{}:{issue_id}::{user_id}", rel.to_lowercase()),
                    "src": issue_id, "dst": user_id, "type": rel,
                }));
            }
        }
    }

    // Parent (epic / subtask).
    if let Some(pkey) = f.get("parent").and_then(|p| p.get("key")).and_then(Value::as_str) {
        let parent_id = format!("jira:issue:{pkey}");
        edges.push(json!({
            "id": format!("e:child_of:{issue_id}::{parent_id}"),
            "src": issue_id, "dst": parent_id, "type": "CHILD_OF",
        }));
    }

    // Issue links.
    for link in f.get("issuelinks").and_then(Value::as_array).into_iter().flatten() {
        let link_type = link.get("type").and_then(|t| t.get("name")).and_then(Value::as_str).unwrap_or("");
        let other = link
            .get("outwardIssue")
            .or_else(|| link.get("inwardIssue"))
            .and_then(|i| i.get("key"))
            .and_then(Value::as_str);
        if let Some(other) = other {
            let other_id = format!("jira:issue:{other}");
            edges.push(json!({
                "id": format!("e:links_to:{issue_id}::{other_id}::{link_type}"),
                "src": issue_id, "dst": other_id, "type": "LINKS_TO", "link": link_type,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_issue_graph() {
        let issue = json!({
            "key": "ENG-1",
            "fields": {
                "summary": "Fix bug", "status": { "name": "Open" },
                "issuetype": { "name": "Bug" },
                "assignee": { "accountId": "u1", "displayName": "Ada" },
                "project": { "key": "ENG", "name": "Engineering" },
                "parent": { "key": "ENG-100" },
                "issuelinks": [{ "type": { "name": "blocks" }, "outwardIssue": { "key": "ENG-2" } }]
            }
        });
        let (mut n, mut e, mut seen) = (vec![], vec![], HashSet::new());
        emit_issue(&issue, &mut n, &mut e, &mut seen);
        let types: Vec<&str> = e.iter().filter_map(|x| x["type"].as_str()).collect();
        assert!(types.contains(&"HAS_ISSUE"));
        assert!(types.contains(&"ASSIGNED_TO"));
        assert!(types.contains(&"CHILD_OF"));
        assert!(types.contains(&"LINKS_TO"));
        assert!(n.iter().any(|x| x["id"] == "jira:issue:ENG-1"));
    }
}
