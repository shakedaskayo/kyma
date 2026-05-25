//! GitHub metadata connector.
//!
//! Ingests repository metadata (repos, branches, pull requests, issues,
//! contributors) into a property-graph via the GitHub REST API using a
//! Personal Access Token (PAT).
//!
//! Code/AST extraction is intentionally **not** part of this unit — that
//! lands in the next task (B2, tree-sitter).
//!
//! ## Graph shape
//! - Nodes → `github_nodes` (4 columns: id, labels, name, props)
//! - Edges → `github_edges` (5 columns: id, src, dst, type, props)
//! - Graph → registered as `"github"` with `GraphSpec::with_defaults`

pub mod admin;
pub mod client;
pub mod config;
pub mod cursor;
pub mod transform;

pub use admin::github_repos_router;

use async_trait::async_trait;
use chrono::Utc;

use crate::types::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use client::GithubClient;
use config::GithubConfig;
use cursor::Cursor;
use transform::{RawRecord, to_graph};

#[derive(Default, Clone, Debug)]
pub struct GithubConnector;

#[async_trait]
impl Connector for GithubConnector {
    fn type_id(&self) -> &'static str {
        "github"
    }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        let parsed: GithubConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        parsed.validate().map_err(ConfigError)
    }

    async fn run_once(
        &self,
        ctx: &ConnectorCtx,
        cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        let config: GithubConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| ConnectorError::Config(format!("config parse: {e}")))?;

        // Resolve the token (may be "$env:GITHUB_TOKEN" or a literal PAT).
        let token = ctx
            .secrets
            .resolve(&config.token)
            .map_err(|e| ConnectorError::Config(format!("token resolve: {e}")))?;

        let gh = GithubClient::new(ctx.http.clone(), token);
        let mut cur = Cursor::from_value(cursor);

        let mut records: Vec<RawRecord> = Vec::new();
        let max_pages = config.max_pages_per_tick;

        for repo_slug in &config.repos {
            let parts: Vec<&str> = repo_slug.splitn(2, '/').collect();
            if parts.len() != 2 {
                return Err(ConnectorError::Config(format!(
                    "invalid repo slug {repo_slug:?}"
                )));
            }
            let owner = parts[0];
            let name = parts[1];

            // 1. Repo metadata
            if config.modules.repos {
                match gh.get_repo(owner, name).await {
                    Ok(repo) => records.push(RawRecord::Repo(repo)),
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "get_repo {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "get_repo failed; skipping");
                    }
                }
            }

            // 2. Branches
            if config.modules.branches {
                match gh.list_branches(owner, name, max_pages).await {
                    Ok(branches) => {
                        for b in branches {
                            records.push(RawRecord::Branch {
                                owner: owner.to_string(),
                                name: name.to_string(),
                                branch: b,
                            });
                        }
                    }
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "list_branches {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "list_branches failed; skipping");
                    }
                }
            }

            let repo_cur = cur.for_repo(repo_slug);

            // 3. Pull requests
            let mut latest_pr_ts = repo_cur.pulls_since;
            if config.modules.pulls {
                match gh
                    .list_pulls(owner, name, repo_cur.pulls_since, max_pages)
                    .await
                {
                    Ok((pulls, _stop)) => {
                        for p in &pulls {
                            // Track latest updated_at for cursor update.
                            if let Some(ts_str) = p["updated_at"].as_str() {
                                if let Ok(ts) = ts_str.parse::<chrono::DateTime<Utc>>() {
                                    latest_pr_ts = Some(match latest_pr_ts {
                                        Some(existing) if ts > existing => ts,
                                        Some(existing) => existing,
                                        None => ts,
                                    });
                                }
                            }
                            records.push(RawRecord::Pull {
                                owner: owner.to_string(),
                                repo: name.to_string(),
                                pull: p.clone(),
                            });
                        }
                    }
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "list_pulls {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "list_pulls failed; skipping");
                    }
                }
            }

            // 4. Issues
            let mut latest_issue_ts = repo_cur.issues_since;
            if config.modules.issues {
                match gh
                    .list_issues(owner, name, repo_cur.issues_since, max_pages)
                    .await
                {
                    Ok((issues, _stop)) => {
                        for i in &issues {
                            if let Some(ts_str) = i["updated_at"].as_str() {
                                if let Ok(ts) = ts_str.parse::<chrono::DateTime<Utc>>() {
                                    latest_issue_ts = Some(match latest_issue_ts {
                                        Some(existing) if ts > existing => ts,
                                        Some(existing) => existing,
                                        None => ts,
                                    });
                                }
                            }
                            records.push(RawRecord::Issue {
                                owner: owner.to_string(),
                                repo: name.to_string(),
                                issue: i.clone(),
                            });
                        }
                    }
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "list_issues {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "list_issues failed; skipping");
                    }
                }
            }

            // 5. Contributors
            if config.modules.contributors {
                match gh.list_contributors(owner, name, max_pages).await {
                    Ok(contributors) => {
                        for u in contributors {
                            records.push(RawRecord::User(u));
                        }
                    }
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "list_contributors {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "list_contributors failed; skipping");
                    }
                }
            }

            // Update cursor with the latest timestamps seen.
            cur.update_repo(repo_slug, latest_pr_ts, latest_issue_ts);
        }

        let (tables, hint) = to_graph(&records);

        Ok(ConnectorRun {
            rows: vec![],
            new_cursor: Some(cur.to_value()),
            tables,
            graph: Some(hint),
        })
    }
}

// Re-export so callers can get the Arc<dyn SecretStore>-based router builder.
pub use admin::github_repos_router as repos_router;
