//! Bitbucket Cloud data source — repositories, pull requests, issues, members.
//!
//! Auth: optional basic auth (username + app password). Public repos work
//! tokenless. Metadata-only (no clone, no source parsing). Emits to the
//! `"bitbucket"` graph using the same node labels as the GitHub/GitLab
//! data sources (Repository, PullRequest, Issue, User) so the unified UI lines
//! them up; ids are prefixed `bb:` to avoid collisions.

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

const MAX_REPOS_PER_TICK: usize = 20;
const MAX_PAGES_PER_ENDPOINT: usize = 2;
const PAGE_LEN: usize = 50;
const API_BASE: &str = "https://api.bitbucket.org/2.0";

#[derive(Debug, Deserialize)]
struct BbConfig {
    /// Atlassian account username. Optional — public repos work without auth.
    #[serde(default)]
    username: String,
    /// App password (Bitbucket-issued, not the account password). Optional.
    #[serde(default)]
    app_password: String,
    /// Reference to a stored credential. Accepts either:
    /// - `basic` (username + password) for app-password auth,
    /// - `pat` (single token) for Bitbucket access tokens.
    /// Preferred over inline username/app_password.
    #[serde(default)]
    credential_id: Option<uuid::Uuid>,
    /// Repository slugs as `workspace/repo_slug` — comma or newline separated.
    repos: String,
}

pub struct BitbucketDataSource;

fn parse_repos(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == '\n' || c == ' ')
        .map(|p| p.trim().trim_matches('/').to_string())
        .filter(|p| !p.is_empty() && p.contains('/'))
        .collect()
}

#[async_trait]
impl DataSource for BitbucketDataSource {
    fn type_id(&self) -> &'static str {
        "bitbucket"
    }

    fn catalog(&self) -> CatalogEntry {
        CatalogEntry {
            type_id: "bitbucket".into(),
            label: "Bitbucket".into(),
            category: "code".into(),
            description: "Repositories, branches, pull requests, and issues from Bitbucket \
                          Cloud — metadata graph (no source parsing yet)."
                .into(),
            brand: "bitbucket".into(),
            auth_mode: "pat".into(),
            status: "available".into(),
            drive_model: "periodic".into(),
            default_schedule_ms: 5 * 60_000,
            fields: vec![
                CatalogField {
                    key: "username".into(), label: "Atlassian username".into(),
                    kind: "text".into(), required: false,
                    placeholder: Some("me@example.com".into()),
                    help: Some("Leave blank to read only public repositories.".into()),
                },
                CatalogField {
                    key: "app_password".into(), label: "App password".into(),
                    kind: "secret".into(), required: false,
                    placeholder: Some("Bitbucket app password".into()),
                    help: Some("Issued under Account → Personal settings → App passwords. \
                                Needs `repository:read` (and `issue:read` for issues).".into()),
                },
                CatalogField {
                    key: "repos".into(), label: "Repositories".into(), kind: "text".into(),
                    required: true,
                    placeholder: Some("atlassian/python-bitbucket, atlassian/atlaskit-mk-2".into()),
                    help: Some("Comma-separated `workspace/repo_slug` paths.".into()),
                },
            ],
            resource: None,
            default_target_table: Some("bitbucket_nodes".into()),
            config_defaults: None,
            graph_name: Some("bitbucket".into()),
            accepted_credential_kinds: vec!["basic".into(), "pat".into()],
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let parsed: BbConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        if parse_repos(&parsed.repos).is_empty() {
            return Err(ConfigError("at least one repository (workspace/repo) is required".into()));
        }
        // Inline-auth pairing rule only applies when credential_id isn't used.
        if parsed.credential_id.is_none()
            && (!parsed.username.is_empty()) != (!parsed.app_password.is_empty())
        {
            return Err(ConfigError(
                "provide both username and app_password, or leave both blank for public repos"
                    .into(),
            ));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &Value,
        _cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let parsed: BbConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| DataSourceError::Permanent(format!("bad config: {e}")))?;

        // Resolve auth: credential_id (preferred) → inline username/app_password.
        // For `basic`: build `Basic base64(user:pass)`; for `pat`: build a Bearer.
        use base64::Engine;
        let auth_header: Option<String> = if let Some(cid) = parsed.credential_id {
            use pensieve_core::credentials::CredentialValue;
            let cred = ctx
                .credentials
                .get(ctx.tenant, cid)
                .await
                .map_err(|e| DataSourceError::Permanent(format!("resolve credential {cid}: {e}")))?;
            match cred.value {
                CredentialValue::Basic { username, password } => {
                    let creds = format!("{username}:{password}");
                    let b64 = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
                    Some(format!("Basic {b64}"))
                }
                CredentialValue::Pat { token } => Some(format!("Bearer {token}")),
                other => {
                    return Err(DataSourceError::Permanent(format!(
                        "credential {cid} has kind={}; bitbucket data source requires `basic` or `pat`",
                        other.kind()
                    )));
                }
            }
        } else if !parsed.username.is_empty() {
            let creds = format!("{}:{}", parsed.username, parsed.app_password);
            let b64 = base64::engine::general_purpose::STANDARD.encode(creds.as_bytes());
            Some(format!("Basic {b64}"))
        } else {
            None
        };

        let mut headers = HeaderMap::new();
        if let Some(auth) = auth_header {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                auth.parse().map_err(|_| DataSourceError::Permanent("bad auth header".into()))?,
            );
        }

        let mut nodes = Vec::<Value>::new();
        let mut edges = Vec::<Value>::new();
        let mut user_seen: HashSet<String> = HashSet::new();

        let repos = parse_repos(&parsed.repos);
        let repos = &repos[..repos.len().min(MAX_REPOS_PER_TICK)];

        for slug in repos {
            let repo_id = format!("bb:repo:{slug}");
            // ── repo metadata ──────────────────────────────────────────────────
            let repo_url = format!("{API_BASE}/repositories/{slug}");
            // Tolerate per-repo Permanent failures (404 = repo doesn't exist;
            // 403 = private + bad creds) so one bad target doesn't kill the
            // whole tick — skip the repo and keep going.
            let repo: Value = match get_json(&ctx.http, &repo_url, &headers).await {
                Ok(v) => v,
                Err(DataSourceError::Permanent(_)) => continue,
                Err(e) => return Err(e),
            };
            nodes.push(json!({
                "id": repo_id,
                "labels": ["Repository"],
                "name": repo.get("full_name").and_then(Value::as_str).unwrap_or(slug),
                "description": repo.get("description").cloned().unwrap_or(Value::Null),
                "default_branch": repo.get("mainbranch").and_then(|m| m.get("name")).cloned().unwrap_or(Value::Null),
                "is_private": repo.get("is_private").cloned().unwrap_or(Value::Null),
                "vendor": "bitbucket",
            }));

            // ── pull requests ──────────────────────────────────────────────────
            let pr_url = format!("{API_BASE}/repositories/{slug}/pullrequests");
            for pr in paged(&ctx.http, &pr_url, &headers, "state=OPEN,MERGED,DECLINED,SUPERSEDED").await? {
                let pr_iid = pr.get("id").and_then(Value::as_i64).unwrap_or(0);
                let pr_id = format!("bb:pr:{slug}#{pr_iid}");
                let author_name = pr.get("author").and_then(|a| a.get("nickname").or_else(|| a.get("display_name")))
                    .and_then(Value::as_str).map(str::to_string);
                nodes.push(json!({
                    "id": pr_id,
                    "labels": ["PullRequest"],
                    "name": format!("#{} {}", pr_iid,
                        pr.get("title").and_then(Value::as_str).unwrap_or("")),
                    "iid": pr_iid,
                    "state": pr.get("state").cloned().unwrap_or(Value::Null),
                    "title": pr.get("title").cloned().unwrap_or(Value::Null),
                    "vendor": "bitbucket",
                }));
                edges.push(json!({
                    "id": format!("e:has_pull_request:{repo_id}::{pr_id}"),
                    "src": repo_id, "dst": pr_id, "type": "HAS_PULL_REQUEST",
                }));
                if let Some(username) = author_name {
                    let user_id = format!("bb:user:{username}");
                    if user_seen.insert(user_id.clone()) {
                        nodes.push(json!({
                            "id": user_id, "labels": ["User"],
                            "name": username, "vendor": "bitbucket",
                        }));
                    }
                    edges.push(json!({
                        "id": format!("e:authored:{user_id}::{pr_id}"),
                        "src": user_id, "dst": pr_id, "type": "AUTHORED",
                    }));
                }
            }

            // ── issues (only repos that enabled the issue tracker; tolerate 404) ─
            let issues_url = format!("{API_BASE}/repositories/{slug}/issues");
            match paged(&ctx.http, &issues_url, &headers, "").await {
                Ok(list) => {
                    for issue in list {
                        let iid = issue.get("id").and_then(Value::as_i64).unwrap_or(0);
                        let issue_id = format!("bb:issue:{slug}#{iid}");
                        let author_name = issue.get("reporter").and_then(|a| a.get("nickname").or_else(|| a.get("display_name")))
                            .and_then(Value::as_str).map(str::to_string);
                        nodes.push(json!({
                            "id": issue_id,
                            "labels": ["Issue"],
                            "name": format!("#{} {}", iid,
                                issue.get("title").and_then(Value::as_str).unwrap_or("")),
                            "iid": iid,
                            "state": issue.get("state").cloned().unwrap_or(Value::Null),
                            "title": issue.get("title").cloned().unwrap_or(Value::Null),
                            "vendor": "bitbucket",
                        }));
                        edges.push(json!({
                            "id": format!("e:has_issue:{repo_id}::{issue_id}"),
                            "src": repo_id, "dst": issue_id, "type": "HAS_ISSUE",
                        }));
                        if let Some(username) = author_name {
                            let user_id = format!("bb:user:{username}");
                            if user_seen.insert(user_id.clone()) {
                                nodes.push(json!({
                                    "id": user_id, "labels": ["User"],
                                    "name": username, "vendor": "bitbucket",
                                }));
                            }
                            edges.push(json!({
                                "id": format!("e:authored:{user_id}::{issue_id}"),
                                "src": user_id, "dst": issue_id, "type": "AUTHORED",
                            }));
                        }
                    }
                }
                Err(DataSourceError::Permanent(_)) => {
                    // Issue tracker disabled for this repo → 404 / not-found. Skip.
                }
                Err(e) => return Err(e),
            }
        }

        let nodes = nodes.into_iter().map(crate::graph_row::normalize_node).collect();
        let edges = edges.into_iter().map(crate::graph_row::normalize_edge).collect();

        Ok(DataSourceRun {
            rows: Vec::new(),
            new_cursor: None,
            tables: vec![
                TableRows { table: "bitbucket_nodes".into(), rows: nodes },
                TableRows { table: "bitbucket_edges".into(), rows: edges },
            ],
            graph: Some(GraphHint {
                graph_name: "bitbucket".into(),
                node_table: "bitbucket_nodes".into(),
                edge_table: "bitbucket_edges".into(),
            }),
        })
    }
}

async fn get_json(http: &reqwest::Client, url: &str, headers: &HeaderMap) -> Result<Value, DataSourceError> {
    let res = http
        .get(url)
        .headers(headers.clone())
        .timeout(Duration::from_secs(30))
        .send()
        .await
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
    // Bitbucket 2.0 paginates with `next` URLs; we honour them but cap pages.
    let sep = if base.contains('?') { '&' } else { '?' };
    let mut url = if extra_q.is_empty() {
        format!("{base}{sep}pagelen={PAGE_LEN}")
    } else {
        format!("{base}{sep}{extra_q}&pagelen={PAGE_LEN}")
    };
    let mut out = Vec::<Value>::new();
    for _ in 0..MAX_PAGES_PER_ENDPOINT {
        let v: Value = get_json(http, &url, headers).await?;
        if let Some(arr) = v.get("values").and_then(Value::as_array) {
            out.extend(arr.iter().cloned());
        }
        match v.get("next").and_then(Value::as_str) {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

