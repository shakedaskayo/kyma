//! SQLite-backed `JobQueue` for the local embedded fabric worker.
//!
//! Claims, progresses, completes, and fails jobs stored in the SQLite `jobs`
//! table.  The scheduler (DataSourceScheduler) inserts rows;  this queue
//! claims + finalises them.  Lease expiry recovery happens inside `claim`
//! via the same SELECT-then-UPDATE pattern that `PgQueue` uses.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use kyma_jobs::JobQueue;
use kyma_core::fabric::ClaimedJob;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteQueue {
    pool: SqlitePool,
    worker_id: Uuid,
    capabilities: Vec<String>,
    lease_secs: i64,
}

impl SqliteQueue {
    pub fn new(
        pool: SqlitePool,
        worker_id: Uuid,
        capabilities: Vec<String>,
        lease_secs: i64,
    ) -> Self {
        Self {
            pool,
            worker_id,
            capabilities,
            lease_secs,
        }
    }

    pub fn worker_id(&self) -> Uuid {
        self.worker_id
    }
}

#[async_trait]
impl JobQueue for SqliteQueue {
    async fn claim(&self, kinds: &[String], max: usize) -> Result<Vec<ClaimedJob>> {
        let mut out = Vec::new();
        let now_str = chrono::Utc::now().to_rfc3339();
        let expires_str = (chrono::Utc::now()
            + chrono::Duration::seconds(self.lease_secs))
        .to_rfc3339();
        let worker_str = self.worker_id.to_string();

        for _ in 0..max {
            // Try each accepted kind in turn.
            let mut claimed_one = false;
            'kinds: for kind in kinds {
                // First: recover expired leases for this kind.
                let _ = sqlx::query(
                    "UPDATE jobs SET status = 'pending', claimed_by = NULL, claim_expires_at = NULL,
                     updated_at = ?
                     WHERE kind = ? AND status IN ('claimed','running')
                       AND claim_expires_at < ?",
                )
                .bind(&now_str)
                .bind(kind)
                .bind(&now_str)
                .execute(&self.pool)
                .await;

                // Check if this worker's capabilities cover the job's requirements.
                // We pull the oldest pending job and verify req_capabilities inline.
                let row: Option<(String, String, String, String, i64)> = sqlx::query_as(
                    "SELECT id, tenant_id, kind, payload, attempt
                     FROM jobs
                     WHERE kind = ? AND status = 'pending'
                     ORDER BY priority DESC, created_at ASC
                     LIMIT 1",
                )
                .bind(kind)
                .fetch_optional(&self.pool)
                .await?;

                let Some((id_str, tenant_str, job_kind, payload_str, attempt)) = row else {
                    continue 'kinds;
                };

                // Check req_capabilities: load from the row.
                let req_caps: Vec<String> = {
                    let caps_str: Option<String> = sqlx::query_scalar(
                        "SELECT req_capabilities FROM jobs WHERE id = ?",
                    )
                    .bind(&id_str)
                    .fetch_optional(&self.pool)
                    .await?;
                    caps_str
                        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                        .unwrap_or_default()
                };

                // Worker must advertise all required capabilities.
                if !req_caps
                    .iter()
                    .all(|c| self.capabilities.iter().any(|cap| cap == c))
                {
                    continue 'kinds;
                }

                // Claim it: atomic UPDATE WHERE status = 'pending' (handles races).
                let res = sqlx::query(
                    "UPDATE jobs SET status = 'claimed', claimed_by = ?,
                     claim_expires_at = ?, attempt = attempt + 1, updated_at = ?
                     WHERE id = ? AND status = 'pending'",
                )
                .bind(&worker_str)
                .bind(&expires_str)
                .bind(&now_str)
                .bind(&id_str)
                .execute(&self.pool)
                .await?;

                if res.rows_affected() == 0 {
                    // Race — another worker claimed it. Try next.
                    continue 'kinds;
                }

                let job_id = Uuid::parse_str(&id_str).unwrap_or(Uuid::nil());
                let tenant_id: Uuid = tenant_str
                    .parse()
                    .unwrap_or(Uuid::nil());
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
                let max_attempts: i64 = sqlx::query_scalar(
                    "SELECT max_attempts FROM jobs WHERE id = ?",
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await?
                .unwrap_or(3);
                let expire_ts = Utc::now() + chrono::Duration::seconds(self.lease_secs);

                out.push(ClaimedJob {
                    id: job_id,
                    tenant_id,
                    kind: job_kind,
                    payload,
                    attempt: attempt as i32 + 1,
                    max_attempts: max_attempts as i32,
                    claim_expires_at: expire_ts,
                    credentials: None,
                });
                claimed_one = true;
                break 'kinds;
            }
            if !claimed_one {
                break;
            }
        }
        Ok(out)
    }

    async fn progress(&self, job_id: Uuid, snapshot: serde_json::Value) -> Result<bool> {
        let id_str = job_id.to_string();
        let worker_str = self.worker_id.to_string();
        let progress_str = serde_json::to_string(&snapshot)?;
        let now_str = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs SET progress = ?, updated_at = ?
             WHERE id = ? AND claimed_by = ?",
        )
        .bind(&progress_str)
        .bind(&now_str)
        .bind(&id_str)
        .bind(&worker_str)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn complete(&self, job_id: Uuid, result: serde_json::Value) -> Result<bool> {
        let id_str = job_id.to_string();
        let worker_str = self.worker_id.to_string();
        let result_str = serde_json::to_string(&result)?;
        let now_str = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs SET status = 'done', result = ?, updated_at = ?
             WHERE id = ? AND claimed_by = ?",
        )
        .bind(&result_str)
        .bind(&now_str)
        .bind(&id_str)
        .bind(&worker_str)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn fail(&self, job_id: Uuid, error: &str) -> Result<bool> {
        let id_str = job_id.to_string();
        let worker_str = self.worker_id.to_string();
        let now_str = chrono::Utc::now().to_rfc3339();
        // Check if we've exhausted attempts.
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT attempt, max_attempts FROM jobs WHERE id = ?",
        )
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?;
        let terminal = row.map_or(true, |(attempt, max)| attempt >= max);
        let new_status = if terminal { "failed" } else { "pending" };
        let res = sqlx::query(
            "UPDATE jobs SET status = ?, last_error = ?, claimed_by = NULL,
             claim_expires_at = NULL, updated_at = ?
             WHERE id = ? AND claimed_by = ?",
        )
        .bind(new_status)
        .bind(error)
        .bind(&now_str)
        .bind(&id_str)
        .bind(&worker_str)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn fail_terminal(&self, job_id: Uuid, error: &str) -> Result<bool> {
        let id_str = job_id.to_string();
        let worker_str = self.worker_id.to_string();
        let now_str = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs SET status = 'failed', last_error = ?, claimed_by = NULL,
             claim_expires_at = NULL, updated_at = ?
             WHERE id = ? AND claimed_by = ?",
        )
        .bind(error)
        .bind(&now_str)
        .bind(&id_str)
        .bind(&worker_str)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn extend_lease(&self, job_id: Uuid, lease_secs: i64) -> Result<bool> {
        let id_str = job_id.to_string();
        let worker_str = self.worker_id.to_string();
        let new_expires = (chrono::Utc::now() + chrono::Duration::seconds(lease_secs)).to_rfc3339();
        let now_str = chrono::Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE jobs SET claim_expires_at = ?, updated_at = ?
             WHERE id = ? AND claimed_by = ?",
        )
        .bind(&new_expires)
        .bind(&now_str)
        .bind(&id_str)
        .bind(&worker_str)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn link_dreaming_run(&self, _job_id: Uuid, _run_id: Uuid) -> Result<()> {
        Ok(())
    }
}
