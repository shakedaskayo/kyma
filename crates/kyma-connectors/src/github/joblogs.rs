//! GitHub Actions job-log capture (E1).
//!
//! For each recent workflow run (since the cursor watermark), enumerate its
//! jobs, fetch each job's full log, **redact secrets**, store the redacted log
//! as an object-store artifact, and emit a `github_job_logs` pointer row that
//! carries the artifact's `object_path` + `sha256` (the retrieval handle).
//!
//! Redaction happens here, before a single byte hits the store — the raw text
//! returned by the API is never persisted.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::artifacts::ArtifactStore;
use crate::github::client::GithubClient;
use crate::github::config::WorkflowOpts;
use crate::github::failure;
use crate::github::transform;
use crate::types::ConnectorError;
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::tenant::TenantId;

/// Flat pointer table for captured job logs (queryable via SQL/KQL; the blob
/// itself lives on the object store at `object_path`).
pub const JOB_LOGS_TABLE: &str = "github_job_logs";

/// Outcome of a single repo's job-log capture.
pub struct CaptureResult {
    /// `github_job_logs` pointer rows.
    pub rows: Vec<Value>,
    /// CI graph nodes (WorkflowRun / Job / LogFile) for `github_nodes`.
    pub nodes: Vec<Value>,
    /// CI graph edges (HAS_RUN / RUN_CONTAINS_JOB / JOB_HAS_LOG / RUN_ON_BRANCH)
    /// for `github_edges`.
    pub edges: Vec<Value>,
    /// Newest run `created_at` processed — advances the cursor watermark.
    pub newest_created: Option<DateTime<Utc>>,
}

/// Object-store key for one job's log. Tenant-prefixed for isolation;
/// deterministic so a re-capture overwrites in place.
pub fn job_log_key(tenant: TenantId, owner: &str, repo: &str, run_id: i64, job_id: i64) -> String {
    format!(
        "artifacts/{}/github/{owner}/{repo}/{run_id}/{job_id}.log.txt",
        tenant.as_uuid()
    )
}

/// Capture job logs for one repo. `since` is the cursor watermark (runs created
/// at or before it are skipped). When `artifacts` is `None` (no store wired),
/// pointer rows are still emitted but blobs are not stored.
pub async fn capture_job_logs(
    gh: &GithubClient,
    artifacts: Option<&Arc<dyn ArtifactStore>>,
    tenant: TenantId,
    owner: &str,
    repo: &str,
    since: Option<DateTime<Utc>>,
    opts: &WorkflowOpts,
) -> Result<CaptureResult, ConnectorError> {
    let (runs, _stop) = gh.list_workflow_runs(owner, repo, opts.max_pages).await?;

    let guard = kyma_redact::global();
    let mut rows: Vec<Value> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut newest: Option<DateTime<Utc>> = since;

    // Select runs created after the watermark, OLDEST-first, capped. Processing
    // oldest-first means a burst larger than the cap is consumed across ticks
    // without ever skipping a run — the watermark only advances past runs we
    // actually processed (advancing it to the newest first would strand the rest).
    let mut new_runs: Vec<(&Value, DateTime<Utc>)> = runs
        .iter()
        .filter_map(|run| {
            if run["id"].as_i64().unwrap_or(0) == 0 {
                return None;
            }
            let created = run["created_at"]
                .as_str()
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())?;
            match since {
                Some(s) if created <= s => None,
                _ => Some((run, created)),
            }
        })
        .collect();
    new_runs.sort_by_key(|(_, c)| *c);
    if new_runs.len() > opts.max_runs_per_tick {
        tracing::warn!(
            owner, repo, new = new_runs.len(), cap = opts.max_runs_per_tick,
            "more new workflow runs than max_runs_per_tick; taking the oldest this tick, the rest next tick"
        );
        new_runs.truncate(opts.max_runs_per_tick);
    }

    for &(run, created) in &new_runs {
        let run_id = run["id"].as_i64().unwrap_or(0);
        newest = Some(match newest {
            Some(n) if n >= created => n,
            _ => created,
        });
        let workflow_name = run["name"].as_str().unwrap_or("").to_string();
        let created_at = run["created_at"].as_str().unwrap_or("").to_string();

        // WorkflowRun node + HAS_RUN / RUN_ON_BRANCH edges (once per run).
        let (run_node, run_edges) = transform::workflow_run_rows(owner, repo, run);
        nodes.push(run_node);
        edges.extend(run_edges);

        let (jobs, _) = gh.list_run_jobs(owner, repo, run_id, opts.max_pages).await?;
        for job in &jobs {
            let job_id = job["id"].as_i64().unwrap_or(0);
            if job_id == 0 {
                continue;
            }
            let job_name = job["name"].as_str().unwrap_or("").to_string();

            // Job node + RUN_CONTAINS_JOB edge — emitted regardless of whether
            // the log is available.
            let (job_node, job_edge) = transform::job_rows(owner, repo, run_id, job);
            nodes.push(job_node);
            edges.push(job_edge);

            // Fetch the raw log; failures (e.g. 404 expired logs) skip this
            // job's log rather than failing the whole tick.
            let raw = match gh.fetch_job_log_text(owner, repo, job_id).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(owner, repo, job_id, error = %e, "job log fetch failed; skipping");
                    continue;
                }
            };

            // Redact BEFORE storing — the raw text is never persisted.
            let (redacted, _findings) = guard.redact_text(&raw);

            // Characterise a failure from the full redacted log (E4a) — the
            // signal the correlation pipeline (E4b) reads off these rows.
            let conclusion = job["conclusion"].as_str().unwrap_or("");
            let fsig = failure::extract_failure_signature(&redacted, conclusion);

            let max = opts.job_log_max_bytes;
            let truncated = redacted.len() > max;
            let stored_str: &str = if truncated {
                let mut cut = max;
                while cut > 0 && !redacted.is_char_boundary(cut) {
                    cut -= 1;
                }
                &redacted[..cut]
            } else {
                &redacted
            };
            let stored_bytes = stored_str.as_bytes().to_vec();
            let sha = kyma_storage::sha256_hex(&stored_bytes);
            let size_bytes = stored_bytes.len() as i64;
            let object_path = job_log_key(tenant, owner, repo, run_id, job_id);

            if let Some(store) = artifacts {
                let record = ArtifactRecord {
                    id: None,
                    tenant_id: tenant,
                    object_path: object_path.clone(),
                    source: "github".into(),
                    artifact_class: "log".into(),
                    table_ref: Some(JOB_LOGS_TABLE.to_string()),
                    connector_id: None,
                    size_bytes,
                    sha256: Some(sha.clone()),
                    created_at: None,
                    expires_at: None,
                    deleted_at: None,
                };
                if let Err(e) = store
                    .put_and_register(record, bytes::Bytes::from(stored_bytes))
                    .await
                {
                    tracing::warn!(owner, repo, job_id, error = %e, "artifact store failed; row emitted without blob");
                }
            }

            // LogFile node + JOB_HAS_LOG edge (carries the object_path handle).
            let (log_node, log_edge) = transform::log_file_rows(
                owner,
                repo,
                job_id,
                &job_name,
                &object_path,
                &sha,
                size_bytes,
                truncated,
            );
            nodes.push(log_node);
            edges.push(log_edge);

            rows.push(json!({
                "run_id": run_id,
                "job_id": job_id,
                "owner": owner,
                "repo": repo,
                "workflow_name": workflow_name,
                "job_name": job["name"].as_str().unwrap_or(""),
                "status": job["status"].as_str().unwrap_or(""),
                "conclusion": conclusion,
                "failed": fsig.failed,
                "failure_kind": fsig.kind,
                "failure_signature": fsig.signature,
                "error_count": fsig.error_count as i64,
                "failure_sample": fsig.sample,
                "object_path": object_path,
                "sha256": sha,
                "size_bytes": size_bytes,
                "truncated": truncated,
                "created_at": created_at,
                "captured_at": Utc::now().to_rfc3339(),
            }));
        }
    }

    Ok(CaptureResult {
        rows,
        nodes,
        edges,
        newest_created: newest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::sync::Mutex;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Test double that records the bytes handed to `put_and_register` so we can
    /// assert what actually gets stored (redacted, never the raw secret).
    #[derive(Default)]
    struct RecordingArtifactStore {
        stored: Mutex<Vec<(String, Bytes)>>,
    }
    #[async_trait::async_trait]
    impl ArtifactStore for RecordingArtifactStore {
        async fn put_and_register(
            &self,
            record: ArtifactRecord,
            bytes: Bytes,
        ) -> anyhow::Result<Uuid> {
            self.stored
                .lock()
                .unwrap()
                .push((record.object_path.clone(), bytes));
            Ok(Uuid::new_v4())
        }
    }

    #[tokio::test]
    async fn capture_redacts_secrets_before_storing() {
        let server = MockServer::start().await;

        // One run.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/actions/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 1,
                "workflow_runs": [
                    { "id": 100, "name": "CI", "status": "completed",
                      "conclusion": "failure", "created_at": "2026-06-08T10:00:00Z" }
                ]
            })))
            .mount(&server)
            .await;

        // One job under the run.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/actions/runs/100/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 1,
                "jobs": [ { "id": 900, "name": "build", "status": "completed", "conclusion": "failure" } ]
            })))
            .mount(&server)
            .await;

        // The job log contains an AWS access-key id (a kyma-redact pattern).
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/actions/jobs/900/logs"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "2026-06-08 build step\nexport AWS_KEY=AKIAIOSFODNN7EXAMPLE\nbuild failed\n",
            ))
            .mount(&server)
            .await;

        let gh = GithubClient::new(reqwest::Client::new(), "tok".into())
            .with_base_url(server.uri());
        // Keep a concrete handle to inspect what was stored; pass the same
        // allocation in as a trait object.
        let recorder = Arc::new(RecordingArtifactStore::default());
        let store: Arc<dyn ArtifactStore> = recorder.clone();
        let tenant = TenantId::from_uuid(
            Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap(),
        );
        let opts = WorkflowOpts::default();

        let res = capture_job_logs(&gh, Some(&store), tenant, "acme", "app", None, &opts)
            .await
            .unwrap();

        // One pointer row, watermark advanced to the run's created_at.
        assert_eq!(res.rows.len(), 1);
        assert_eq!(res.rows[0]["run_id"], 100);
        assert_eq!(res.rows[0]["job_id"], 900);
        assert_eq!(res.rows[0]["conclusion"], "failure");
        // Failure signature extracted (E4a): the failed conclusion + "build
        // failed" log line yield a non-empty signature.
        assert_eq!(res.rows[0]["failed"], true);
        assert!(
            res.rows[0]["failure_signature"].as_str().unwrap().contains("build failed"),
            "expected a failure signature, got: {}",
            res.rows[0]["failure_signature"]
        );
        assert!(res.newest_created.is_some());
        assert_eq!(
            res.rows[0]["object_path"],
            job_log_key(tenant, "acme", "app", 100, 900)
        );

        // The stored blob is redacted: the secret is gone, the marker is present.
        let stored = recorder.stored.lock().unwrap();
        assert_eq!(stored.len(), 1, "exactly one blob stored");
        let (key, bytes) = &stored[0];
        assert_eq!(key, &job_log_key(tenant, "acme", "app", 100, 900));
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains("AKIAIOSFODNN7EXAMPLE"),
            "raw secret must not be stored; got:\n{text}"
        );
        assert!(
            text.contains("[REDACTED:aws-key-id]"),
            "redaction marker expected; got:\n{text}"
        );
        assert!(text.contains("build failed"), "non-secret content preserved");

        // ── CI graph subgraph (E2) ──
        // 1 run + 1 job + 1 log = 3 nodes; HAS_RUN + RUN_CONTAINS_JOB +
        // JOB_HAS_LOG = 3 edges (no RUN_ON_BRANCH — the mock run has no branch).
        let label = |n: &Value| n["labels"].as_str().unwrap_or("").to_string();
        let id = |n: &Value| n["id"].as_str().unwrap_or("").to_string();
        assert_eq!(res.nodes.len(), 3, "run+job+log nodes");
        assert!(res.nodes.iter().any(|n| label(n) == "WorkflowRun" && id(n) == "run:acme/app#100"));
        assert!(res.nodes.iter().any(|n| label(n) == "Job" && id(n) == "job:acme/app#900"));
        let log_node = res
            .nodes
            .iter()
            .find(|n| label(n) == "LogFile" && id(n) == "log:acme/app#900")
            .expect("LogFile node");
        assert!(
            log_node["props"].as_str().unwrap().contains(&job_log_key(tenant, "acme", "app", 100, 900)),
            "LogFile node carries the object_path retrieval handle"
        );

        let etype = |e: &Value| e["type"].as_str().unwrap_or("").to_string();
        let etypes: Vec<String> = res.edges.iter().map(etype).collect();
        assert!(etypes.contains(&"HAS_RUN".to_string()));
        assert!(etypes.contains(&"RUN_CONTAINS_JOB".to_string()));
        assert!(etypes.contains(&"JOB_HAS_LOG".to_string()));
    }
}
