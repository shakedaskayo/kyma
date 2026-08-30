//! GitLab data source — projects, merge requests, issues, and members metadata.
//!
//! Uses GitLab REST v4 with optional `PRIVATE-TOKEN` auth (public projects work
//! tokenless). Metadata-only by design: no clone, no source parsing. Emits
//! graph rows under a stable `"gitlab"` graph with the same node labels as the
//! GitHub graph (Repository, PullRequest, Issue, User) so cross-vendor views in
//! the UI compare apples-to-apples; ids are prefixed `gl:` so they don't
//! collide with GitHub ids.

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{
    ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun, GraphHint, TableRows,
};

const MAX_PROJECTS_PER_TICK: usize = 20;
const MAX_PAGES_PER_ENDPOINT: usize = 2; // bounded for v1; cursor paging is future work
const PER_PAGE: usize = 50;

#[derive(Debug, Deserialize)]
struct GlConfig {
    /// Personal access token. Optional — public projects work without one.
    #[serde(default)]
    token: String,
    /// Reference to a stored credential of kind `pat`. Preferred over inline
    /// `token`; rotation propagates and the secret is encrypted at rest.
    #[serde(default)]
    credential_id: Option<uuid::Uuid>,
    /// API base URL, e.g. `https://gitlab.com/api/v4`. Defaults to the public host.
    #[serde(default = "default_api_url")]
    api_url: String,
    /// Project paths (e.g. `"gitlab-org/gitlab"`) — comma or newline separated.
    projects: String,
}
fn default_api_url() -> String {
    "https://gitlab.com/api/v4".into()
}

pub struct GitlabDataSource;

fn parse_projects(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == '\n' || c == ' ')
        .map(|p| p.trim().trim_matches('/').to_string())
        .filter(|p| !p.is_empty() && p.contains('/'))
        .collect()
}

#[async_trait]
impl DataSource for GitlabDataSource {
    fn type_id(&self) -> &'static str {
        "gitlab"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "gitlab".into(),
            label: "GitLab".into(),
            category: "code".into(),
            description: "Projects, branches, merge requests, issues, and members from \
                          GitLab — metadata graph (no source parsing yet)."
                .into(),
            brand: "gitlab".into(),
            auth_mode: "pat".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 5 * 60_000,
            fields: vec![
                CatalogField {
                    key: "token".into(), label: "Personal access token".into(),
                    kind: "secret".into(), required: false,
                    placeholder: Some("glpat-…".into()),
                    help: Some("PAT with `read_api` scope. Leave blank for public projects.".into()),
                },
                CatalogField {
                    key: "projects".into(), label: "Projects".into(), kind: "text".into(),
                    required: true,
                    placeholder: Some("gitlab-org/gitlab-foss, gitlab-org/gitlab-runner".into()),
                    help: Some("Comma-separated project paths.".into()),
                },
                CatalogField {
                    key: "api_url".into(), label: "API base URL".into(), kind: "text".into(),
                    required: false,
                    placeholder: Some("https://gitlab.com/api/v4".into()),
                    help: Some("For self-hosted GitLab. Leave blank for gitlab.com.".into()),
                },
            ],
            resource: None,
            default_target_table: Some("gitlab_nodes".into()),
            config_defaults: Some(json!({ "api_url": default_api_url() })),
            graph_name: Some("gitlab".into()),
            accepted_credential_kinds: vec!["pat".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: GlConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parse_projects(&parsed.projects).is_empty() {
            return Err(ConfigError("at least one project (namespace/path) is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        _cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let parsed: GlConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;

        // Resolve auth token — credential_id (preferred) → inline token → none.
        let token: String = if let Some(cid) = parsed.credential_id {
            use pensieve_core::credentials::CredentialValue;
            let cred = ctx
                .credentials
                .get(ctx.tenant, cid)
                .await
                .map_err(|e| DataSourceError::Permanent(format!("resolve credential {cid}: {e}")))?;
            match cred.value {
                CredentialValue::Pat { token } => token,
                other => {
                    return Err(DataSourceError::Permanent(format!(
                        "credential {cid} has kind={}; gitlab data source requires `pat`",
                        other.kind()
                    )));
                }
            }
        } else {
            parsed.token.clone()
        };

        let mut headers = HeaderMap::new();
        if !token.is_empty() {
            headers.insert(
                "PRIVATE-TOKEN",
                token.parse().map_err(|_| DataSourceError::Permanent("bad token".into()))?,
            );
        }
        let api = parsed.api_url.trim_end_matches('/').to_string();
        let mut nodes = Vec::<Value>::new();
        let mut edges = Vec::<Value>::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        let projects = parse_projects(&parsed.projects);
        let projects = &projects[..projects.len().min(MAX_PROJECTS_PER_TICK)];

        for path in projects {
            let proj_id = format!("gl:project:{path}");
            // ── project metadata ───────────────────────────────────────────────
            let url = format!("{api}/projects/{}", urlenc(path));
            let proj: Value = get_json(&ctx.http, &url, &headers).await?;
            nodes.push(json!({
                "id": proj_id,
                "labels": ["Repository"],
                "name": proj.get("path_with_namespace").and_then(Value::as_str).unwrap_or(path),
                "description": proj.get("description").cloned().unwrap_or(Value::Null),
                "default_branch": proj.get("default_branch").cloned().unwrap_or(Value::Null),
                "visibility": proj.get("visibility").cloned().unwrap_or(Value::Null),
                "vendor": "gitlab",
            }));

            // ── merge requests ─────────────────────────────────────────────────
            for mr in paged(&ctx.http, &format!("{api}/projects/{}/merge_requests", urlenc(path)),
                            &headers, "state=all").await? {
                let iid = mr.get("iid").and_then(Value::as_i64).unwrap_or(0);
                let mr_id = format!("gl:mr:{path}#{iid}");
                let author_name = mr.get("author").and_then(|a| a.get("username"))
                    .and_then(Value::as_str).map(str::to_string);
                nodes.push(json!({
                    "id": mr_id,
                    "labels": ["PullRequest"],
                    "name": format!("!{} {}", iid,
                        mr.get("title").and_then(Value::as_str).unwrap_or("")),
                    "iid": iid,
                    "state": mr.get("state").cloned().unwrap_or(Value::Null),
                    "title": mr.get("title").cloned().unwrap_or(Value::Null),
                    "vendor": "gitlab",
                }));
                edges.push(json!({
                    "id": format!("e:has_pull_request:{proj_id}::{mr_id}"),
                    "src": proj_id, "dst": mr_id, "type": "HAS_PULL_REQUEST",
                }));
                if let Some(username) = author_name {
                    let user_id = format!("gl:user:{username}");
                    if user_seen.insert(user_id.clone()) {
                        nodes.push(json!({
                            "id": user_id, "labels": ["User"],
                            "name": username, "vendor": "gitlab",
                        }));
                    }
                    edges.push(json!({
                        "id": format!("e:authored:{user_id}::{mr_id}"),
                        "src": user_id, "dst": mr_id, "type": "AUTHORED",
                    }));
                }
            }

            // ── issues ─────────────────────────────────────────────────────────
            for issue in paged(&ctx.http, &format!("{api}/projects/{}/issues", urlenc(path)),
                                &headers, "state=all").await? {
                let iid = issue.get("iid").and_then(Value::as_i64).unwrap_or(0);
                let issue_id = format!("gl:issue:{path}#{iid}");
                let author_name = issue.get("author").and_then(|a| a.get("username"))
                    .and_then(Value::as_str).map(str::to_string);
                nodes.push(json!({
                    "id": issue_id,
                    "labels": ["Issue"],
                    "name": format!("#{} {}", iid,
                        issue.get("title").and_then(Value::as_str).unwrap_or("")),
                    "iid": iid,
                    "state": issue.get("state").cloned().unwrap_or(Value::Null),
                    "title": issue.get("title").cloned().unwrap_or(Value::Null),
                    "vendor": "gitlab",
                }));
                edges.push(json!({
                    "id": format!("e:has_issue:{proj_id}::{issue_id}"),
                    "src": proj_id, "dst": issue_id, "type": "HAS_ISSUE",
                }));
                if let Some(username) = author_name {
                    let user_id = format!("gl:user:{username}");
                    if user_seen.insert(user_id.clone()) {
                        nodes.push(json!({
                            "id": user_id, "labels": ["User"],
                            "name": username, "vendor": "gitlab",
                        }));
                    }
                    edges.push(json!({
                        "id": format!("e:authored:{user_id}::{issue_id}"),
                        "src": user_id, "dst": issue_id, "type": "AUTHORED",
                    }));
                }
            }

            // ── members (skip silently on 401/403/404 — public projects
            // commonly hide the member list without auth) ─────────────────────
            match paged(&ctx.http, &format!("{api}/projects/{}/members/all", urlenc(path)),
                         &headers, "").await {
                Ok(members) => for m in members {
                    let username = m.get("username").and_then(Value::as_str).map(str::to_string);
                    if let Some(username) = username {
                        let user_id = format!("gl:user:{username}");
                        if user_seen.insert(user_id.clone()) {
                            nodes.push(json!({
                                "id": user_id, "labels": ["User"],
                                "name": username, "vendor": "gitlab",
                            }));
                        }
                        edges.push(json!({
                            "id": format!("e:member_of:{user_id}::{proj_id}"),
                            "src": user_id, "dst": proj_id, "type": "MEMBER_OF",
                        }));
                    }
                },
                Err(DataSourceError::Permanent(_)) => { /* gated endpoint; skip */ }
                Err(e) => return Err(e),
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: None,
            tables: vec![
                TableRows { table: "gitlab_nodes".into(), rows: nodes },
                TableRows { table: "gitlab_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "gitlab".into(),
                node_table: "gitlab_nodes".into(),
                edge_table: "gitlab_edges".into(),
            }),
        })
    }
}

fn urlenc(s: &str) -> String {
    // Path-segment URL encoding (slashes become %2F as GitLab expects).
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}

async fn get_json(http: &reqwest::Client, url: &str, headers: &HeaderMap) -> Result<Value, DataSourceError> {
    let res = http.get(url).headers(headers.clone())
        .timeout(Duration::from_secs(30))
        .send().await
        .map_err(|e| DataSourceError::Transient(format!("GET {url}: {e}")))?;
    let status = res.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(DataSourceError::Transient(format!("GET {url} → {status}")));
    }
    if !status.is_success() {
        return Err(DataSourceError::Permanent(format!("GET {url} → {status}")));
    }
    res.json().await.map_err(|e| DataSourceError::Transient(format!("parse {url}: {e}")))
}

async fn paged(
    http: &reqwest::Client,
    base: &str,
    headers: &HeaderMap,
    extra_q: &str,
) -> Result<Vec<Value>, DataSourceError> {
    let mut out = Vec::<Value>::new();
    for page in 1..=MAX_PAGES_PER_ENDPOINT {
        let sep = if base.contains('?') { '&' } else { '?' };
        let q = if extra_q.is_empty() {
            format!("{base}{sep}per_page={PER_PAGE}&page={page}")
        } else {
            format!("{base}{sep}{extra_q}&per_page={PER_PAGE}&page={page}")
        };
        let v: Value = get_json(http, &q, headers).await?;
        let Some(arr) = v.as_array() else { break };
        if arr.is_empty() {
            break;
        }
        let got = arr.len();
        out.extend(arr.iter().cloned());
        if got < PER_PAGE {
            break;
        }
    }
    Ok(out)
}
