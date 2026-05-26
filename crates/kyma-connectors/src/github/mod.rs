//! GitHub metadata + structural code graph connector.
//!
//! Ingests repository metadata (repos, branches, pull requests, issues,
//! contributors) **and** the structural code graph (Repo→Dir→File `CONTAINS`
//! + file-level `IMPORTS` via tree-sitter) into a property-graph via the
//! GitHub REST API using a Personal Access Token (PAT).
//!
//! ## Graph shape
//! - Nodes → `github_nodes` (4 columns: id, labels, name, props)
//! - Edges → `github_edges` (5 columns: id, src, dst, type, props)
//! - Graph → registered as `"github"` with `GraphSpec::with_defaults`

pub mod admin;
pub mod client;
pub mod config;
pub mod cursor;
pub mod parse;
pub mod transform;

pub use admin::github_repos_router;

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashSet;

use crate::types::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use client::{GithubClient, StopReason};
use config::GithubConfig;
use cursor::Cursor;
use transform::{FileImports, RawRecord, code_to_graph, resolve_imports, to_graph};

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

        // Collect default-branch HEAD SHAs per repo (needed for code graph).
        let mut repo_head_shas: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

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
            let repo_json = if config.modules.repos {
                match gh.get_repo(owner, name).await {
                    Ok(repo) => {
                        let j = repo.clone();
                        records.push(RawRecord::Repo(repo));
                        Some(j)
                    }
                    Err(ConnectorError::Transient(e)) => {
                        return Err(ConnectorError::Transient(format!(
                            "get_repo {repo_slug}: {e}"
                        )));
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "get_repo failed; skipping");
                        None
                    }
                }
            } else {
                None
            };

            // 2. Branches
            if config.modules.branches {
                match gh.list_branches(owner, name, max_pages).await {
                    Ok(branches) => {
                        // Capture the default branch SHA for code graph
                        if let Some(ref rj) = repo_json {
                            let default_branch = rj["default_branch"].as_str().unwrap_or("main");
                            for b in &branches {
                                if b["name"].as_str() == Some(default_branch) {
                                    if let Some(sha) = b["commit"]["sha"].as_str() {
                                        repo_head_shas.insert(repo_slug.clone(), sha.to_string());
                                    }
                                }
                            }
                        }
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
            let mut pulls_rate_limited = false;
            if config.modules.pulls {
                match gh
                    .list_pulls(owner, name, repo_cur.pulls_since, max_pages)
                    .await
                {
                    Ok((pulls, stop)) => {
                        if stop == StopReason::RateLimited {
                            tracing::warn!(
                                repo = %repo_slug,
                                "pulls fetch hit rate-limit floor; cursor will not advance this tick"
                            );
                            pulls_rate_limited = true;
                        }
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
            let mut issues_rate_limited = false;
            if config.modules.issues {
                match gh
                    .list_issues(owner, name, repo_cur.issues_since, max_pages)
                    .await
                {
                    Ok((issues, stop)) => {
                        if stop == StopReason::RateLimited {
                            tracing::warn!(
                                repo = %repo_slug,
                                "issues fetch hit rate-limit floor; cursor will not advance this tick"
                            );
                            issues_rate_limited = true;
                        }
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

            // Update metadata cursor with the latest timestamps seen.
            // When a resource was rate-limited mid-page, do NOT advance that
            // resource's cursor — leave it at the previous value so the next
            // tick re-fetches from the same point and no items are skipped.
            cur.update_repo(
                repo_slug,
                if pulls_rate_limited { repo_cur.pulls_since } else { latest_pr_ts },
                if issues_rate_limited { repo_cur.issues_since } else { latest_issue_ts },
            );
        }

        // ── Transform metadata records ────────────────────────────────────────
        let (meta_tables, hint) = to_graph(&records);
        let mut nodes: Vec<serde_json::Value> = meta_tables
            .iter()
            .find(|t| t.table == transform::NODE_TABLE)
            .map(|t| t.rows.clone())
            .unwrap_or_default();
        let mut edges: Vec<serde_json::Value> = meta_tables
            .iter()
            .find(|t| t.table == transform::EDGE_TABLE)
            .map(|t| t.rows.clone())
            .unwrap_or_default();

        // ── Skip re-emitting unchanged full-refetch metadata ─────────────────
        // repo/user/branch are re-fetched in full every poll, but the node/edge
        // tables are append-only. Gate those rows by a content signature so a
        // steady-state tick (nothing changed) writes zero rows instead of
        // duplicating them. Pulls/issues are cursor-incremental and pass through.
        let sig = transform::refetch_signature(&nodes, &edges);
        if cur.refetch_signature.as_deref() == Some(sig.as_str()) {
            let before = nodes.len() + edges.len();
            nodes.retain(|n| !transform::is_refetch_node(n));
            edges.retain(|e| !transform::is_refetch_edge(e));
            tracing::debug!(
                dropped = before - (nodes.len() + edges.len()),
                "github: full-refetch metadata unchanged; skipping re-emit"
            );
        } else {
            cur.refetch_signature = Some(sig);
        }

        // ── Code graph (B2) ───────────────────────────────────────────────────
        if config.modules.codebase {
            for repo_slug in &config.repos {
                let parts: Vec<&str> = repo_slug.splitn(2, '/').collect();
                if parts.len() != 2 {
                    continue;
                }
                let owner = parts[0];
                let name = parts[1];

                // Get the current HEAD SHA.
                let Some(head_sha) = repo_head_shas.get(repo_slug) else {
                    tracing::debug!(
                        repo = %repo_slug,
                        "no HEAD SHA available; skipping code graph"
                    );
                    continue;
                };

                // Skip if nothing changed since last ingest.
                let repo_cur = cur.for_repo(repo_slug);
                if repo_cur.tree_sha_ingested.as_deref() == Some(head_sha.as_str()) {
                    tracing::debug!(
                        repo = %repo_slug,
                        sha = %head_sha,
                        "HEAD SHA unchanged; skipping code graph re-ingest"
                    );
                    continue;
                }

                // Fetch the recursive git tree.
                let tree_entries = match gh.get_tree(owner, name, head_sha).await {
                    Ok(entries) => entries,
                    Err(ConnectorError::Transient(e)) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "get_tree failed; skipping");
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(repo = %repo_slug, error = %e, "get_tree permanent error; skipping");
                        continue;
                    }
                };

                // Build the set of all file paths for import resolution.
                let all_paths: HashSet<String> = tree_entries
                    .iter()
                    .filter(|e| e.entry_type == "blob")
                    .map(|e| e.path.clone())
                    .collect();

                // Build glob set for exclusions.
                let glob_set = build_glob_set(&config.code.exclude_globs);

                // Filter + fetch + parse files.
                let blob_entries: Vec<_> = tree_entries
                    .iter()
                    .filter(|e| {
                        e.entry_type == "blob"
                            && e.size <= config.code.max_file_bytes
                            && !is_excluded(&e.path, &glob_set)
                            && parse::Lang::from_path(&e.path)
                                .map(|l| config.code.languages.contains(&l.name().to_string()))
                                .unwrap_or(false)
                    })
                    .take(config.code.max_files_per_tick)
                    .collect();

                let files_cap = blob_entries.len();
                let mut file_imports: Vec<FileImports> = Vec::new();

                for entry in &blob_entries {
                    let lang = match parse::Lang::from_path(&entry.path) {
                        Some(l) => l,
                        None => continue,
                    };

                    // Fetch the blob asynchronously; parse imports on spawn_blocking
                    // (tree-sitter is sync/CPU) below.
                    let gh2 = gh.clone();
                    let owner2 = owner.to_string();
                    let name2 = name.to_string();
                    let sha2 = entry.sha.clone();
                    let max_bytes = config.code.max_file_bytes;

                    let content = match gh2.get_blob(&owner2, &name2, &sha2, max_bytes).await {
                        Ok(Some(c)) => c,
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::debug!(path = %entry.path, error = %e, "get_blob failed; skipping");
                            continue;
                        }
                    };

                    // Parse imports on a blocking thread (tree-sitter is sync/CPU).
                    let content2 = content.clone();
                    let raw_imports = tokio::task::spawn_blocking(move || {
                        parse::extract_imports(lang, &content2)
                    })
                    .await
                    .unwrap_or_default();

                    let resolved = resolve_imports(lang.name(), &entry.path, &raw_imports, &all_paths);

                    file_imports.push(FileImports {
                        path: entry.path.clone(),
                        size: entry.size,
                        language: lang.name().to_string(),
                        imports: resolved,
                    });
                }

                // Merge code graph into existing nodes/edges.
                let (code_tables, _) = code_to_graph(owner, name, &file_imports, nodes, edges);
                nodes = code_tables
                    .iter()
                    .find(|t| t.table == transform::NODE_TABLE)
                    .map(|t| t.rows.clone())
                    .unwrap_or_default();
                edges = code_tables
                    .iter()
                    .find(|t| t.table == transform::EDGE_TABLE)
                    .map(|t| t.rows.clone())
                    .unwrap_or_default();

                // Update code cursor.
                cur.update_code_cursor(repo_slug, head_sha.clone(), files_cap);
            }
        }

        // ── Reassemble final TableRows ────────────────────────────────────────
        let tables = vec![
            crate::types::TableRows {
                table: transform::NODE_TABLE.to_string(),
                rows: nodes,
            },
            crate::types::TableRows {
                table: transform::EDGE_TABLE.to_string(),
                rows: edges,
            },
        ];

        Ok(ConnectorRun {
            rows: vec![],
            new_cursor: Some(cur.to_value()),
            tables,
            graph: Some(hint),
        })
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Build a `GlobSet` from a list of glob patterns. Errors are silently ignored
/// (a failed pattern simply doesn't exclude anything).
fn build_glob_set(patterns: &[String]) -> Option<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    for pat in patterns {
        if let Ok(g) = globset::Glob::new(pat) {
            builder.add(g);
        }
    }
    builder.build().ok()
}

/// Return `true` if `path` matches any glob in `set`.
fn is_excluded(path: &str, set: &Option<globset::GlobSet>) -> bool {
    match set {
        Some(gs) => gs.is_match(path),
        None => false,
    }
}

// Re-export so callers can get the Arc<dyn SecretStore>-based router builder.
pub use admin::github_repos_router as repos_router;
