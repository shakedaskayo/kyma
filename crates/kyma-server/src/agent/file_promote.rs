//! File-candidate promotion pipeline (E6).
//!
//! A scheduled deterministic dreaming worker (sibling of [`super::ci_correlate::CiCorrelator`])
//! that resolves contributed/scraped candidate `File` nodes to their **live
//! upstream repo** `File` nodes in the `github` code graph, and records the
//! match as a cross-graph `SAME_AS` edge — stitching "the files an agent read /
//! the files on disk" to "the repo + its CI + its memories" in one graph.
//!
//! The `SAME_AS` edge doubles as the promotion marker: a candidate that already
//! has one is skipped on the next tick. Candidates with no upstream match (a
//! file outside any synced repo) stay in the candidate layer.

use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use kyma_core::tenant::TenantId;
use kyma_memory::file_candidates::FILE_CANDIDATES_DB;
use kyma_memory::MemoryWriter;

use super::tools::{execute_sql, SharedToolCtx};

const GITHUB_NODE_TABLE: &str = "github_nodes";

/// The live `github` graph node id for a repo file. Mirrors
/// `kyma_connectors::github::transform::file_id` (`file:{owner}/{repo}:{path}`),
/// which is how the connector keys File nodes — so a contributed file under a
/// known repo maps to the exact same node.
fn github_file_id(repo: &str, path: &str) -> String {
    format!("file:{repo}:{path}")
}

/// Pull `(repo, path)` out of a candidate File node's provenance JSON (stored as
/// a string). `None` when the file has no repo (can't resolve upstream).
fn repo_and_path(provenance: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(provenance).ok()?;
    let repo = v.get("repo").and_then(Value::as_str)?;
    let path = v.get("path").and_then(Value::as_str)?;
    if repo.is_empty() || path.is_empty() {
        return None;
    }
    Some((repo.to_string(), path.to_string()))
}

fn sql_lit(s: &str) -> String {
    s.replace('\'', "''")
}

/// Scheduled candidate→live promotion worker.
pub struct FilePromoter {
    shared: SharedToolCtx,
    pool: PgPool,
    tenant: TenantId,
    pub poll_interval: Duration,
    /// Max candidates resolved per tick.
    pub batch: usize,
}

impl FilePromoter {
    pub fn new(shared: SharedToolCtx, pool: PgPool, tenant: TenantId) -> Self {
        Self {
            shared,
            pool,
            tenant,
            poll_interval: Duration::from_secs(300),
            batch: 200,
        }
    }

    pub async fn run(self, shutdown: impl std::future::Future<Output = ()> + Send) {
        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // consume the immediate first tick
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                _ = ticker.tick() => {
                    if let Err(e) = self.tick().await {
                        tracing::warn!(error = %e, "file-promote tick failed");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> anyhow::Result<()> {
        // Unresolved candidate File nodes: latest version, no SAME_AS edge yet.
        let sql = format!(
            "WITH files AS (SELECT id, provenance, \
                row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn \
                FROM memory_nodes WHERE labels = 'File') \
             SELECT f.id AS id, f.provenance AS provenance FROM files f \
             WHERE f.rn = 1 AND f.id NOT IN (SELECT src FROM memory_edges WHERE type = 'SAME_AS') \
             LIMIT {}",
            self.batch
        );
        let res = execute_sql(&self.shared, FILE_CANDIDATES_DB, &sql, self.batch).await;
        if res.get("error").is_some() {
            return Ok(()); // candidate graph not created yet — nothing to do
        }
        let candidates = res.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
        if candidates.is_empty() {
            return Ok(());
        }

        let dbs = self.shared.catalog.list_databases().await.unwrap_or_default();
        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO memory_pipeline_runs (id, tenant_id, kind, status, started_at) \
             VALUES ($1, $2, 'file_promote', 'running', $3)",
        )
        .bind(run_id)
        .bind(self.tenant.as_uuid())
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        match self.promote(&candidates, &dbs).await {
            Ok((scanned, promoted)) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='success', finished_at=$2, \
                     events_scanned=$3, memories_written=$4, mode='deterministic' WHERE id=$1",
                )
                .bind(run_id)
                .bind(Utc::now())
                .bind(scanned)
                .bind(promoted)
                .execute(&self.pool)
                .await?;
            }
            Err(e) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='error', finished_at=$2, error=$3 WHERE id=$1",
                )
                .bind(run_id)
                .bind(Utc::now())
                .bind(e.to_string())
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Resolve each candidate to a live github File node and write a `SAME_AS`
    /// edge. Returns `(scanned, promoted)`.
    async fn promote(&self, candidates: &[Value], dbs: &[String]) -> anyhow::Result<(i64, i64)> {
        let embed = kyma_memory::shared_embedding()
            .await
            .map_err(|e| anyhow::anyhow!("embedding backend: {e}"))?;
        let writer = MemoryWriter::new(self.shared.catalog.clone(), self.shared.format.clone(), embed)
            .with_database(FILE_CANDIDATES_DB);

        let scanned = candidates.len() as i64;
        let mut promoted = 0i64;
        for c in candidates {
            let id = c.get("id").and_then(Value::as_str).unwrap_or("");
            let prov = c.get("provenance").and_then(Value::as_str).unwrap_or("");
            let Some((repo, path)) = repo_and_path(prov) else {
                continue;
            };
            let gid = github_file_id(&repo, &path);

            // Find which database holds the live github File node.
            let mut found_db: Option<String> = None;
            for db in dbs {
                let q = format!(
                    "SELECT id FROM {GITHUB_NODE_TABLE} WHERE id = '{}' LIMIT 1",
                    sql_lit(&gid)
                );
                let r = execute_sql(&self.shared, db, &q, 1).await;
                if r.get("rows").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
                    found_db = Some(db.clone());
                    break;
                }
            }
            let Some(db) = found_db else {
                continue; // no upstream match — stays a candidate
            };

            // Cross-graph SAME_AS edge: candidate File → live github File. The
            // target_namespace points the unified canvas at the github graph.
            let realm = repo.clone();
            let target_ns = format!("{db}/github");
            if writer
                .link(id, &gid, "SAME_AS", &realm, Some(&target_ns))
                .await
                .is_ok()
            {
                promoted += 1;
            }
        }
        Ok((scanned, promoted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_file_id_matches_connector_scheme() {
        assert_eq!(
            github_file_id("acme/app", "src/main.rs"),
            "file:acme/app:src/main.rs"
        );
    }

    #[test]
    fn repo_and_path_parsed_from_provenance() {
        let prov = r#"{"source":"file_contribution","stage":"candidate","repo":"acme/app","path":"src/main.rs"}"#;
        assert_eq!(
            repo_and_path(prov),
            Some(("acme/app".to_string(), "src/main.rs".to_string()))
        );
        // No repo → can't resolve upstream.
        assert_eq!(repo_and_path(r#"{"path":"x.rs"}"#), None);
        assert_eq!(repo_and_path("not json"), None);
    }
}
