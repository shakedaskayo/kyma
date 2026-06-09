//! The kyma node daemon (`kyma worker run`) — a fabric worker on any compute.
//!
//! A node is more than a job executor: on a developer machine it OWNS local
//! sources (a coding agent's memory files / transcripts — Claude Code, Cursor,
//! Windsurf, Codex, …) that only it can read. The daemon therefore:
//! 1. registers with the control plane (worker token) and heartbeats with
//!    presence (active local coding-agent sessions) + its source inventory,
//! 2. self-enqueues `source_sync` jobs pinned to itself (only the node sees
//!    its local file state) and executes them via the per-agent sync pipeline,
//! 3. long-polls `POST /v1/jobs/claim` for any other kinds the operator
//!    explicitly accepts (`--accept`).
//!
//! Which agents are present is decided by the detector registry in
//! [`crate::agent_sources`] — a cheap fs probe per agent. Adding an agent is
//! one registry entry; only Claude Code has a working ingestion pipeline today.
//!
//! Low-impact by default: a dev-laptop daemon accepts ONLY `source_sync`
//! (one concurrent job). Heavy kinds (dreaming, connector syncs) run on the
//! server's embedded worker unless this node opts in.

use anyhow::{Context, Result};
use async_trait::async_trait;
use kyma_core::fabric::{ClaimedJob, Heartbeat, WorkerRegistration};
use kyma_jobs::{ExecutorRegistry, JobCtx, JobError, JobExecutor, JobQueue, JobRunner};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::agent_sources;

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Control-plane base URL (e.g. `https://kyma.example.com`).
    pub server_url: String,
    /// Worker token (`kyw_…`) minted by `kyma worker create`.
    pub token: String,
    /// Job kinds this node executes. Default: `["source_sync"]`.
    pub accept: Vec<String>,
    pub max_concurrent: usize,
    /// Friendly name override (defaults to the registered worker's name).
    pub name: Option<String>,
}

impl NodeConfig {
    pub fn validate(&self) -> Result<()> {
        let insecure = std::env::var("KYMA_WORKER_INSECURE").ok().as_deref() == Some("1");
        let local = self.server_url.starts_with("http://127.0.0.1")
            || self.server_url.starts_with("http://localhost");
        if self.server_url.starts_with("http://") && !insecure && !local {
            anyhow::bail!(
                "refusing plain http to a non-local control plane (worker tokens + claim \
                 payloads ride this channel). Use https, or set KYMA_WORKER_INSECURE=1."
            );
        }
        if !self.token.starts_with("kyw_") {
            anyhow::bail!("token does not look like a worker token (kyw_…)");
        }
        Ok(())
    }
}

/// HTTP front-end to the control plane's worker/job API.
pub struct RemoteQueue {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl RemoteQueue {
    fn new(cfg: &NodeConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: cfg.server_url.trim_end_matches('/').to_string(),
            token: cfg.token.clone(),
        }
    }

    async fn post(&self, path: &str, body: &Value) -> Result<reqwest::Response> {
        Ok(self
            .http
            .post(format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?)
    }

    pub async fn register(&self, reg: &WorkerRegistration) -> Result<Value> {
        let resp = self
            .post("/v1/workers/register", &serde_json::to_value(reg)?)
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
        if !status.is_success() {
            anyhow::bail!("register failed ({status}): {body}");
        }
        Ok(body)
    }

    pub async fn heartbeat(&self, hb: &Heartbeat) -> Result<Value> {
        let resp = self
            .post("/v1/workers/heartbeat", &serde_json::to_value(hb)?)
            .await?;
        Ok(resp.json().await.unwrap_or_else(|_| json!({})))
    }

    /// Self-enqueue a job pinned to this worker (operator-auth not needed —
    /// uses the admin enqueue surface? No: worker tokens can't hit the
    /// operator router, so source_sync self-enqueue goes through claim-side
    /// dedup instead: see `enqueue_self`).
    pub async fn enqueue_self(&self, job: &Value) -> Result<Option<Uuid>> {
        let resp = self.post("/v1/jobs/self", job).await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
        if !status.is_success() {
            anyhow::bail!("self-enqueue failed ({status}): {body}");
        }
        Ok(body
            .get("job_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok()))
    }
}

#[async_trait]
impl JobQueue for RemoteQueue {
    async fn claim(&self, kinds: &[String], max: usize) -> Result<Vec<ClaimedJob>> {
        let resp = self
            .post(
                "/v1/jobs/claim",
                &json!({ "kinds": kinds, "max": max, "wait_ms": 20_000 }),
            )
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
        if !status.is_success() {
            anyhow::bail!("claim failed ({status}): {body}");
        }
        Ok(serde_json::from_value(
            body.get("jobs").cloned().unwrap_or_else(|| json!([])),
        )?)
    }

    async fn progress(&self, job_id: Uuid, snapshot: Value) -> Result<bool> {
        let resp = self
            .post(&format!("/v1/jobs/{job_id}/progress"), &snapshot)
            .await?;
        Ok(resp.status().is_success())
    }

    async fn complete(&self, job_id: Uuid, result: Value) -> Result<bool> {
        let resp = self
            .post(&format!("/v1/jobs/{job_id}/complete"), &json!({ "result": result }))
            .await?;
        Ok(resp.status().is_success())
    }

    async fn fail(&self, job_id: Uuid, error: &str) -> Result<bool> {
        let resp = self
            .post(&format!("/v1/jobs/{job_id}/fail"), &json!({ "error": error }))
            .await?;
        Ok(resp.status().is_success())
    }

    async fn fail_terminal(&self, job_id: Uuid, error: &str) -> Result<bool> {
        // The HTTP surface folds terminal failures into /fail; max_attempts=1
        // jobs end terminal regardless.
        self.fail(job_id, error).await
    }

    async fn extend_lease(&self, job_id: Uuid, lease_secs: i64) -> Result<bool> {
        let resp = self
            .post(
                &format!("/v1/jobs/{job_id}/lease"),
                &json!({ "lease_secs": lease_secs }),
            )
            .await?;
        Ok(resp.status().is_success())
    }

    async fn link_dreaming_run(&self, _job_id: Uuid, _run_id: Uuid) -> Result<()> {
        Ok(()) // control-plane-side concern; remote dreaming lands later
    }
}

// ── source_sync executor ─────────────────────────────────────────────────────

/// Runs one local sync pass (a coding agent's files → engine/control plane).
/// The job payload's `source` field selects the agent kind; `claude-code`
/// drives the existing `run_sync` machinery, other detected kinds are accepted
/// but no-op until their ingest pipeline lands. Registered only on node
/// daemons — the server's embedded worker has no local sources.
pub struct SourceSyncExecutor;

#[async_trait]
impl JobExecutor for SourceSyncExecutor {
    fn kind(&self) -> &'static str {
        kyma_core::fabric::JOB_SOURCE_SYNC
    }

    async fn run(&self, ctx: &JobCtx, job: &ClaimedJob) -> Result<Value, JobError> {
        // Default to claude-code for back-compat with older payloads.
        let source = job
            .payload
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("claude-code")
            // tolerate the legacy underscore spelling.
            .replace('_', "-");

        match source.as_str() {
            "claude-code" => {
                ctx.progress
                    .push(json!({ "current_phase": "syncing Claude Code sources" }))
                    .await;
                crate::run_sync(crate::SyncOptions {
                    watch: false,
                    dry_run: false,
                    cc_only: false,
                    cloud_only: false,
                    project: None,
                })
                .await
                .map_err(|e| JobError::Transient(e.to_string()))?;
                Ok(json!({ "synced": true, "source": "claude-code" }))
            }
            other => {
                // The seam for future ingest pipelines: the agent is detected
                // and its source_sync job is accepted, but there's no pipeline
                // for it yet, so it cleanly no-ops.
                tracing::info!(source = %other, "ingestion for {other} not yet implemented — skipping");
                ctx.progress
                    .push(json!({ "current_phase": format!("skipping {other} (no ingest pipeline)") }))
                    .await;
                Ok(json!({ "skipped": other }))
            }
        }
    }
}

// ── the daemon ───────────────────────────────────────────────────────────────

pub async fn run_node(cfg: NodeConfig) -> Result<()> {
    cfg.validate()?;
    let hostname = hostname_string();
    let queue = Arc::new(RemoteQueue::new(&cfg));

    // Capabilities: the generic `sources` capability when this node accepts
    // source_sync, plus a per-kind `source:<kind>` for each detected agent
    // (e.g. source:claude-code). Other accepted kinds pass through verbatim.
    let detected = agent_sources::detect_all();
    let mut capabilities: Vec<String> = vec![];
    for k in &cfg.accept {
        match k.as_str() {
            "source_sync" => {
                capabilities.push("sources".to_string());
                for a in &detected {
                    capabilities.push(agent_sources::capability_for(a.kind));
                }
            }
            other => capabilities.push(other.to_string()),
        }
    }

    let reg = WorkerRegistration {
        name: cfg.name.clone().unwrap_or_else(|| format!("node@{hostname}")),
        kind: kyma_core::fabric::WorkerKind::Daemon,
        hostname: Some(hostname.clone()),
        capabilities,
        labels: json!({}),
        sources: agent_sources::discover_sources(),
        max_concurrent: cfg.max_concurrent as i32,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
    };
    let info = queue.register(&reg).await.context("registering with control plane")?;
    println!(
        "registered with {} as {} (accepting: {})",
        cfg.server_url,
        info.get("worker_id").and_then(|v| v.as_str()).unwrap_or("?"),
        cfg.accept.join(", ")
    );

    // Heartbeat task: liveness + presence + refreshed source inventory.
    let hb_queue = queue.clone();
    let heartbeat = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let hb = Heartbeat {
                current_jobs: vec![],
                presence: agent_sources::discover_presence(),
                sources: Some(agent_sources::discover_sources()),
            };
            if let Err(e) = hb_queue.heartbeat(&hb).await {
                eprintln!("heartbeat failed (continuing): {e}");
            }
        }
    });

    // Self-enqueue loop: only this node can see its local file state, so the
    // node itself schedules its source_sync work (deduped server-side by
    // jobs_source_sync_uniq while one is in flight).
    let accepts_source_sync = cfg.accept.iter().any(|k| k == "source_sync");
    let enq_queue = queue.clone();
    let self_enqueue = tokio::spawn(async move {
        if !accepts_source_sync {
            return;
        }
        let poll = std::env::var("KYMA_CC_SYNC_POLL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60u64);
        let mut tick = tokio::time::interval(Duration::from_secs(poll.max(15)));
        loop {
            tick.tick().await;
            // One source_sync per detected agent kind (deduped server-side by
            // jobs_source_sync_uniq while one is in flight). If no agent is
            // detected, fall back to claude-code so a freshly-installed node
            // still tries the default pipeline.
            let detected = agent_sources::detect_all();
            let kinds: Vec<String> = if detected.is_empty() {
                vec!["claude-code".to_string()]
            } else {
                detected.iter().map(|a| a.kind.to_string()).collect()
            };
            for kind in kinds {
                if let Err(e) = enq_queue
                    .enqueue_self(&json!({
                        "kind": "source_sync",
                        "payload": { "source": kind },
                    }))
                    .await
                {
                    eprintln!("source_sync enqueue failed for {kind} (continuing): {e}");
                }
            }
        }
    });

    // Executors this node runs.
    let mut executors = ExecutorRegistry::new();
    if accepts_source_sync {
        executors.register(Arc::new(SourceSyncExecutor));
    }
    // NOTE: remote `dreaming` / `connector_sync` execution lands in a
    // follow-up (needs control-plane-side trace materialization + the HTTP
    // write surface). Accepting them today claims jobs this daemon cannot
    // run, so refuse early.
    for k in &cfg.accept {
        if k != "source_sync" {
            eprintln!(
                "warning: `{k}` is not executable on remote nodes yet — ignoring \
                 (runs on the server's embedded worker)"
            );
        }
    }
    if executors.kinds().is_empty() {
        heartbeat.abort();
        self_enqueue.abort();
        anyhow::bail!("nothing to do: no accepted job kind is executable on this node");
    }

    let lease_secs = 300;
    let runner = JobRunner::new(queue.clone(), executors, lease_secs);
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nshutting down…");
    };
    runner.run(shutdown).await;
    heartbeat.abort();
    self_enqueue.abort();
    Ok(())
}

fn hostname_string() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}
