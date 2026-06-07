//! Postgres-backed worker/job fabric store — the control plane's persistence
//! for the distributed context engine.
//!
//! Workers (the server's embedded worker, or remote `kyma worker run`
//! daemons) register here and claim jobs under a lease, mirroring the
//! `background_tasks` `FOR UPDATE SKIP LOCKED` machinery but with placement:
//! a job can be pinned to one worker (`affinity_worker_id`, e.g. source_sync
//! on the node that owns the files) and/or require capabilities the claiming
//! worker must advertise (`req_capabilities`, e.g. `claude-cli` for remote
//! dreaming).

use anyhow::Result;
use chrono::{DateTime, Utc};
use kyma_core::fabric::{
    ClaimedJob, EnqueueJob, Heartbeat, JobStatus, JobView, PresenceSession, WorkerKind,
    WorkerRegistration, WorkerStatus, WorkerView,
};
use kyma_core::tenant::TenantId;
use serde_json::Value as Json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Authenticated worker identity resolved from a worker token (or assumed by
/// the embedded worker in-process).
#[derive(Debug, Clone)]
pub struct WorkerAuth {
    pub worker_id: Uuid,
    pub tenant: TenantId,
    pub capabilities: Vec<String>,
    pub status: WorkerStatus,
}

#[derive(Clone)]
pub struct PgFabricStore {
    pool: PgPool,
}

impl PgFabricStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ── workers ─────────────────────────────────────────────────────────────

    /// Admin issuance: create a daemon worker row with its token hash.
    /// The plaintext token is minted by the caller and shown exactly once.
    pub async fn create_worker(
        &self,
        tenant: TenantId,
        name: &str,
        token_hash: &str,
        capabilities: &[String],
        labels: Json,
    ) -> Result<Uuid> {
        let row = sqlx::query(
            "INSERT INTO workers (tenant_id, name, kind, token_hash, capabilities, labels)
             VALUES ($1, $2, 'daemon', $3, $4, $5)
             RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(name)
        .bind(token_hash)
        .bind(capabilities)
        .bind(&labels)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    /// Token → worker identity. Only non-offline workers may authenticate;
    /// revocation clears `token_hash`, so revoked tokens stop matching.
    pub async fn authenticate_worker(&self, token_hash: &str) -> Result<Option<WorkerAuth>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, capabilities, status
             FROM workers WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(WorkerAuth {
            worker_id: row.get("id"),
            tenant: TenantId::from_uuid(row.get("tenant_id")),
            capabilities: row.get("capabilities"),
            status: parse_worker_status(row.get::<String, _>("status").as_str()),
        }))
    }

    /// Worker connect: refresh the registration fields and come online.
    /// The identity (row) was created at issuance; this is idempotent.
    pub async fn register_worker(&self, worker_id: Uuid, reg: &WorkerRegistration) -> Result<()> {
        sqlx::query(
            "UPDATE workers
             SET hostname = $2, capabilities = $3, labels = $4, sources = $5,
                 max_concurrent = $6, version = $7, status = 'online',
                 last_heartbeat = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(worker_id)
        .bind(&reg.hostname)
        .bind(&reg.capabilities)
        .bind(&reg.labels)
        .bind(&reg.sources)
        .bind(reg.max_concurrent)
        .bind(&reg.version)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The server's in-process worker: idempotent on (tenant, name), no token.
    pub async fn upsert_embedded_worker(
        &self,
        tenant: TenantId,
        reg: &WorkerRegistration,
    ) -> Result<Uuid> {
        let row = sqlx::query(
            "INSERT INTO workers (tenant_id, name, hostname, kind, capabilities, labels,
                                  sources, max_concurrent, version, status, last_heartbeat)
             VALUES ($1, $2, $3, 'embedded', $4, $5, $6, $7, $8, 'online', now())
             ON CONFLICT (tenant_id, name) DO UPDATE
             SET hostname = EXCLUDED.hostname,
                 capabilities = EXCLUDED.capabilities,
                 labels = EXCLUDED.labels,
                 sources = EXCLUDED.sources,
                 max_concurrent = EXCLUDED.max_concurrent,
                 version = EXCLUDED.version,
                 status = 'online',
                 last_heartbeat = now(),
                 updated_at = now()
             RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(&reg.name)
        .bind(&reg.hostname)
        .bind(&reg.capabilities)
        .bind(&reg.labels)
        .bind(&reg.sources)
        .bind(reg.max_concurrent)
        .bind(&reg.version)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    /// Heartbeat: bump liveness + presence; revive an `offline` worker to
    /// `online` (a `draining` worker stays draining so it keeps winding down).
    /// Returns the post-heartbeat status so callers can relay a drain request.
    pub async fn touch_heartbeat(&self, worker_id: Uuid, hb: &Heartbeat) -> Result<WorkerStatus> {
        let presence = serde_json::to_value(&hb.presence)?;
        let row = sqlx::query(
            "UPDATE workers
             SET last_heartbeat = now(),
                 presence = $2,
                 sources = COALESCE($3, sources),
                 status = CASE WHEN status = 'offline' THEN 'online' ELSE status END,
                 updated_at = now()
             WHERE id = $1
             RETURNING status",
        )
        .bind(worker_id)
        .bind(&presence)
        .bind(&hb.sources)
        .fetch_one(&self.pool)
        .await?;
        Ok(parse_worker_status(row.get::<String, _>("status").as_str()))
    }

    pub async fn set_worker_status(&self, worker_id: Uuid, status: WorkerStatus) -> Result<()> {
        sqlx::query("UPDATE workers SET status = $2, updated_at = now() WHERE id = $1")
            .bind(worker_id)
            .bind(status.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Revoke: token stops matching and the worker is marked offline.
    pub async fn revoke_worker(&self, tenant: TenantId, worker_id: Uuid) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE workers SET token_hash = NULL, status = 'offline', updated_at = now()
             WHERE id = $1 AND tenant_id = $2",
        )
        .bind(worker_id)
        .bind(tenant.as_uuid())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Node discovery: every worker in the tenant with status, capabilities,
    /// sources, and presence.
    pub async fn list_workers(&self, tenant: TenantId) -> Result<Vec<WorkerView>> {
        let rows = sqlx::query(
            "SELECT id, name, hostname, kind, capabilities, labels, sources, status,
                    max_concurrent, version, presence, last_heartbeat, created_at
             FROM workers WHERE tenant_id = $1
             ORDER BY kind, name",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(worker_view_from_row).collect()
    }

    // ── jobs ────────────────────────────────────────────────────────────────

    /// Enqueue a job. Returns `None` when an idempotency index skipped the
    /// insert (an equivalent job is already in flight).
    pub async fn enqueue_job(&self, tenant: TenantId, job: &EnqueueJob) -> Result<Option<Uuid>> {
        let row = sqlx::query(
            "INSERT INTO jobs (tenant_id, kind, payload, priority, affinity_worker_id,
                               req_capabilities, label_selector, max_attempts)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT DO NOTHING
             RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(&job.kind)
        .bind(&job.payload)
        .bind(job.priority)
        .bind(job.affinity_worker_id)
        .bind(&job.req_capabilities)
        .bind(&job.label_selector)
        .bind(job.max_attempts)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.get("id")))
    }

    /// Placement-aware atomic claim, mirroring `claim_task`:
    /// - only kinds this worker executes, capabilities satisfied (`@>`),
    /// - pinned to this worker OR unpinned (pinned-first ordering),
    /// - expired leases reclaimed when attempts remain,
    /// - `tenant = None` is the embedded worker's all-tenant mode; remote
    ///   daemons are always scoped to their token's tenant.
    pub async fn claim_job(
        &self,
        worker_id: Uuid,
        tenant: Option<TenantId>,
        capabilities: &[String],
        kinds: &[String],
        lease_secs: i64,
    ) -> Result<Option<ClaimedJob>> {
        let row = sqlx::query(
            "WITH next AS (
                SELECT id FROM jobs
                WHERE kind = ANY($4)
                  AND ($5::uuid IS NULL OR tenant_id = $5)
                  AND (affinity_worker_id = $1 OR affinity_worker_id IS NULL)
                  AND ($3::text[] @> req_capabilities)
                  AND (status = 'pending'
                       OR (status IN ('claimed','running')
                           AND claim_expires_at < now()
                           AND attempt < max_attempts))
                ORDER BY (affinity_worker_id = $1) DESC NULLS LAST,
                         priority DESC, created_at
                LIMIT 1
                FOR UPDATE SKIP LOCKED
             )
             UPDATE jobs j
             SET status = 'claimed',
                 claimed_by = $1,
                 claim_expires_at = now() + ($2 || ' seconds')::interval,
                 attempt = j.attempt + 1,
                 updated_at = now()
             FROM next
             WHERE j.id = next.id
             RETURNING j.id, j.tenant_id, j.kind, j.payload, j.attempt, j.max_attempts,
                       j.claim_expires_at",
        )
        .bind(worker_id)
        .bind(lease_secs.to_string())
        .bind(capabilities)
        .bind(kinds)
        .bind(tenant.map(|t| t.as_uuid()))
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(ClaimedJob {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            kind: row.get("kind"),
            payload: row.get("payload"),
            attempt: row.get("attempt"),
            max_attempts: row.get("max_attempts"),
            claim_expires_at: row.get("claim_expires_at"),
            credentials: None,
        }))
    }

    /// Replace the live progress snapshot. Guarded by `claimed_by` so a worker
    /// that lost its lease cannot scribble over the new owner's progress.
    pub async fn job_progress(&self, job_id: Uuid, worker_id: Uuid, snapshot: Json) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs
             SET progress = $3,
                 status = CASE WHEN status = 'claimed' THEN 'running' ELSE status END,
                 updated_at = now()
             WHERE id = $1 AND claimed_by = $2 AND status IN ('claimed','running')",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(&snapshot)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn complete_job(&self, job_id: Uuid, worker_id: Uuid, result: Json) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs
             SET status = 'done', result = $3, claim_expires_at = NULL, updated_at = now()
             WHERE id = $1 AND claimed_by = $2 AND status IN ('claimed','running')",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(&result)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Fail: back to `pending` while attempts remain, else terminal `failed`
    /// (mirrors `fail_task`).
    pub async fn fail_job(&self, job_id: Uuid, worker_id: Uuid, error: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs
             SET status = CASE WHEN attempt >= max_attempts THEN 'failed' ELSE 'pending' END,
                 claimed_by = CASE WHEN attempt >= max_attempts THEN claimed_by ELSE NULL END,
                 claim_expires_at = NULL,
                 last_error = $3,
                 updated_at = now()
             WHERE id = $1 AND claimed_by = $2 AND status IN ('claimed','running')",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Terminal failure regardless of remaining attempts (non-retryable
    /// executor errors, e.g. broken job payload).
    pub async fn fail_job_terminal(
        &self,
        job_id: Uuid,
        worker_id: Uuid,
        error: &str,
    ) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs
             SET status = 'failed', claim_expires_at = NULL, last_error = $3, updated_at = now()
             WHERE id = $1 AND claimed_by = $2 AND status IN ('claimed','running')",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn extend_lease(&self, job_id: Uuid, worker_id: Uuid, lease_secs: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE jobs
             SET claim_expires_at = now() + ($3 || ' seconds')::interval, updated_at = now()
             WHERE id = $1 AND claimed_by = $2 AND status IN ('claimed','running')",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(lease_secs.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Set the dreaming-run linkage on a job (executor calls this once it has
    /// minted the run row).
    pub async fn link_dreaming_run(&self, job_id: Uuid, run_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE jobs SET dreaming_run_id = $2, updated_at = now() WHERE id = $1")
            .bind(job_id)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Queue self-heal: requeue expired leases with attempts left, fail the
    /// exhausted ones, and mark silent workers offline. Returns
    /// (jobs_requeued, jobs_failed, workers_offlined).
    pub async fn sweep_stale(&self, worker_offline_after_secs: i64) -> Result<(u64, u64, u64)> {
        let requeued = sqlx::query(
            "UPDATE jobs
             SET status = 'pending', claimed_by = NULL, claim_expires_at = NULL,
                 last_error = COALESCE(last_error, '') || ' [recovered: stale lease]',
                 updated_at = now()
             WHERE status IN ('claimed','running') AND claim_expires_at < now()
               AND attempt < max_attempts",
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        let failed = sqlx::query(
            "UPDATE jobs
             SET status = 'failed', claim_expires_at = NULL,
                 last_error = COALESCE(last_error, '') || ' [failed: lease expired, attempts exhausted]',
                 updated_at = now()
             WHERE status IN ('claimed','running') AND claim_expires_at < now()
               AND attempt >= max_attempts",
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        let offlined = sqlx::query(
            "UPDATE workers
             SET status = 'offline', updated_at = now()
             WHERE status IN ('online','draining')
               AND (last_heartbeat IS NULL
                    OR last_heartbeat < now() - ($1 || ' seconds')::interval)",
        )
        .bind(worker_offline_after_secs.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok((requeued, failed, offlined))
    }

    pub async fn get_job(&self, tenant: TenantId, job_id: Uuid) -> Result<Option<JobView>> {
        let row = sqlx::query(&format!("{JOB_VIEW_SELECT} WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(job_view_from_row).transpose()
    }

    pub async fn list_jobs(
        &self,
        tenant: TenantId,
        kind: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JobView>> {
        let rows = sqlx::query(&format!(
            "{JOB_VIEW_SELECT}
             WHERE tenant_id = $1
               AND ($2::text IS NULL OR kind = $2)
               AND ($3::text IS NULL OR status = $3)
             ORDER BY created_at DESC
             LIMIT $4"
        ))
        .bind(tenant.as_uuid())
        .bind(kind)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(job_view_from_row).collect()
    }
}

const JOB_VIEW_SELECT: &str = "SELECT id, kind, status, payload, priority, affinity_worker_id,
        req_capabilities, claimed_by, claim_expires_at, attempt, max_attempts,
        progress, result, dreaming_run_id, last_error, created_at, updated_at
 FROM jobs";

fn parse_worker_status(s: &str) -> WorkerStatus {
    match s {
        "online" => WorkerStatus::Online,
        "draining" => WorkerStatus::Draining,
        _ => WorkerStatus::Offline,
    }
}

fn parse_job_status(s: &str) -> JobStatus {
    match s {
        "pending" => JobStatus::Pending,
        "claimed" => JobStatus::Claimed,
        "running" => JobStatus::Running,
        "done" => JobStatus::Done,
        "canceled" => JobStatus::Canceled,
        _ => JobStatus::Failed,
    }
}

fn worker_view_from_row(row: sqlx::postgres::PgRow) -> Result<WorkerView> {
    let kind = match row.get::<String, _>("kind").as_str() {
        "embedded" => WorkerKind::Embedded,
        _ => WorkerKind::Daemon,
    };
    let presence: Json = row.get("presence");
    let presence: Vec<PresenceSession> = serde_json::from_value(presence).unwrap_or_default();
    Ok(WorkerView {
        id: row.get("id"),
        name: row.get("name"),
        hostname: row.get("hostname"),
        kind,
        capabilities: row.get("capabilities"),
        labels: row.get("labels"),
        sources: row.get("sources"),
        status: parse_worker_status(row.get::<String, _>("status").as_str()),
        max_concurrent: row.get("max_concurrent"),
        version: row.get("version"),
        presence,
        last_heartbeat: row.get::<Option<DateTime<Utc>>, _>("last_heartbeat"),
        created_at: row.get("created_at"),
    })
}

fn job_view_from_row(row: sqlx::postgres::PgRow) -> Result<JobView> {
    Ok(JobView {
        id: row.get("id"),
        kind: row.get("kind"),
        status: parse_job_status(row.get::<String, _>("status").as_str()),
        payload: row.get("payload"),
        priority: row.get("priority"),
        affinity_worker_id: row.get("affinity_worker_id"),
        req_capabilities: row.get("req_capabilities"),
        claimed_by: row.get("claimed_by"),
        claim_expires_at: row.get("claim_expires_at"),
        attempt: row.get("attempt"),
        max_attempts: row.get("max_attempts"),
        progress: row.get("progress"),
        result: row.get("result"),
        dreaming_run_id: row.get("dreaming_run_id"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
