//! CI failure-correlation pipeline (E4b) — the proactive-intelligence loop.
//!
//! A scheduled "dreaming" worker (sibling of [`super::memory::MemoryConsolidator`])
//! that periodically scans the `github_job_logs` failure signal (E4a) for
//! **recurring** failures — the same normalised `failure_signature` across
//! multiple runs — and distils each into a durable **incident memory**. The
//! memory carries the repo, failure kind, signature, occurrence count, and the
//! correlated run ids in its provenance, so the agent can recall "what keeps
//! breaking" and navigate to the commits/jobs/logs behind it.
//!
//! Deterministic: needs no LLM. (An engine-driven "proposed fix" pass is a
//! natural follow-up, mirroring `MemoryConsolidator::with_engine`.)

use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use kyma_core::tenant::TenantId;
use kyma_memory::types::MemoryType;
use kyma_memory::{CreateMemory, MemoryWriter};

use super::tools::{execute_sql, SharedToolCtx};

const JOB_LOGS_TABLE: &str = "github_job_logs";

/// SQL that groups failed job-log rows by `(owner, repo, kind, signature)` and
/// keeps only signatures recurring across at least `min_runs` distinct runs
/// since `cutoff` (rfc3339; lexicographic compare is chronological).
fn recurring_failures_sql(cutoff_rfc3339: &str, min_runs: i64) -> String {
    let cutoff = cutoff_rfc3339.replace('\'', "''");
    format!(
        "SELECT owner, repo, failure_kind, failure_signature, \
                count(DISTINCT run_id) AS runs, \
                count(*) AS jobs, \
                min(created_at) AS first_seen, \
                max(created_at) AS last_seen, \
                min(failure_sample) AS sample \
         FROM {JOB_LOGS_TABLE} \
         WHERE failed = true AND failure_signature <> '' AND created_at > '{cutoff}' \
         GROUP BY owner, repo, failure_kind, failure_signature \
         HAVING count(DISTINCT run_id) >= {min_runs} \
         ORDER BY runs DESC"
    )
}

/// One detected recurring-failure incident.
#[derive(Debug, Clone)]
struct Incident {
    owner: String,
    repo: String,
    kind: String,
    signature: String,
    runs: i64,
    first_seen: String,
    last_seen: String,
    sample: String,
}

impl Incident {
    fn from_row(db: &str, r: &Value) -> Option<Self> {
        let _ = db;
        let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let owner = s("owner");
        let repo = s("repo");
        let signature = s("failure_signature");
        if owner.is_empty() || repo.is_empty() || signature.is_empty() {
            return None;
        }
        Some(Self {
            owner,
            repo,
            kind: s("failure_kind"),
            signature,
            runs: r.get("runs").and_then(Value::as_i64).unwrap_or(0),
            first_seen: s("first_seen"),
            last_seen: s("last_seen"),
            sample: s("sample"),
        })
    }

    /// Stable upsert key so re-detecting the same incident updates the memory in
    /// place rather than appending a duplicate.
    fn topic_key(&self) -> String {
        let sig: String = self
            .signature
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .take(80)
            .collect();
        format!("ci-incident:{}/{}:{}:{sig}", self.owner, self.repo, self.kind)
    }

    fn into_memory(self) -> CreateMemory {
        let topic_key = self.topic_key();
        let Incident {
            owner,
            repo,
            kind,
            signature,
            runs,
            first_seen,
            last_seen,
            sample,
        } = self;
        let realm = format!("{owner}/{repo}");
        let content = format!(
            "Recurring CI failure in {owner}/{repo} ({kind}): \"{signature}\" — seen across \
             {runs} runs between {first_seen} and {last_seen}. Sample: {sample}"
        );

        let mut cm = CreateMemory::new(content);
        cm.title = Some(format!("CI incident — {owner}/{repo}: {kind}"));
        cm.memory_type = MemoryType::Learning;
        cm.realm = realm;
        cm.importance = 0.75;
        cm.tags = vec![
            "pipeline:ci_correlate".to_string(),
            "ci:incident".to_string(),
            format!("repo:{owner}/{repo}"),
            format!("kind:{kind}"),
        ];
        cm.topic_key = Some(topic_key);
        cm.provenance = Some(json!({
            "source": "ci_correlate",
            "owner": owner,
            "repo": repo,
            "failure_kind": kind,
            "failure_signature": signature,
            "runs": runs,
            "first_seen": first_seen,
            "last_seen": last_seen,
        }));
        cm
    }
}

/// Scheduled CI failure-correlation worker.
pub struct CiCorrelator {
    shared: SharedToolCtx,
    pool: PgPool,
    tenant: TenantId,
    pub poll_interval: Duration,
    /// Lookback window for "recurring" detection.
    pub window: chrono::Duration,
    /// Minimum distinct runs sharing a signature to count as recurring.
    pub min_runs: i64,
}

impl CiCorrelator {
    pub fn new(shared: SharedToolCtx, pool: PgPool, tenant: TenantId) -> Self {
        Self {
            shared,
            pool,
            tenant,
            poll_interval: Duration::from_secs(300),
            window: chrono::Duration::days(7),
            min_runs: 2,
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
                        tracing::warn!(error = %e, "ci-correlate tick failed");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let now = Utc::now();
        let cutoff = (now - self.window).format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
        let sql = recurring_failures_sql(&cutoff, self.min_runs);

        // The github_job_logs table lives in whichever database the GitHub
        // connector targets — scan every database and union the hits.
        let dbs = self.shared.catalog.list_databases().await.unwrap_or_default();
        let mut incidents: Vec<Incident> = Vec::new();
        for db in &dbs {
            let res = execute_sql(&self.shared, db, &sql, 200).await;
            if res.get("error").is_some() {
                continue; // table absent in this db
            }
            if let Some(rows) = res.get("rows").and_then(Value::as_array) {
                incidents.extend(rows.iter().filter_map(|r| Incident::from_row(db, r)));
            }
        }
        if incidents.is_empty() {
            return Ok(());
        }

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO memory_pipeline_runs (id, tenant_id, kind, status, started_at) \
             VALUES ($1, $2, 'ci_correlate', 'running', $3)",
        )
        .bind(run_id)
        .bind(self.tenant.as_uuid())
        .bind(now)
        .execute(&self.pool)
        .await?;

        match self.write_incidents(incidents).await {
            Ok((scanned, written)) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='success', finished_at=$2, \
                     events_scanned=$3, memories_written=$4, watermark_ts=$5, mode='deterministic' \
                     WHERE id=$1",
                )
                .bind(run_id)
                .bind(Utc::now())
                .bind(scanned)
                .bind(written)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            Err(e) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='error', finished_at=$2, error=$3 \
                     WHERE id=$1",
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

    /// Write/upsert an incident memory per detected recurrence. Returns
    /// `(scanned, written)`.
    async fn write_incidents(&self, incidents: Vec<Incident>) -> anyhow::Result<(i64, i64)> {
        let embed = kyma_memory::shared_embedding()
            .await
            .map_err(|e| anyhow::anyhow!("embedding backend: {e}"))?;
        let writer = MemoryWriter::new(
            self.shared.catalog.clone(),
            self.shared.format.clone(),
            embed,
        );
        let _ = writer.ensure_provisioned().await;

        let scanned = incidents.len() as i64;
        let mut written = 0i64;
        for inc in incidents {
            if writer.save(&inc.into_memory()).await.is_ok() {
                written += 1;
            }
        }
        Ok((scanned, written))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{BooleanArray, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use datafusion::datasource::MemTable;
    use datafusion::prelude::SessionContext;
    use std::sync::Arc;

    /// The recurring-failure aggregation groups by signature and keeps only the
    /// ones spanning >= min_runs distinct runs.
    #[tokio::test]
    async fn recurring_sql_groups_and_thresholds() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("owner", DataType::Utf8, false),
            Field::new("repo", DataType::Utf8, false),
            Field::new("run_id", DataType::Int64, false),
            Field::new("failed", DataType::Boolean, false),
            Field::new("failure_kind", DataType::Utf8, false),
            Field::new("failure_signature", DataType::Utf8, false),
            Field::new("failure_sample", DataType::Utf8, false),
            Field::new("created_at", DataType::Utf8, false),
        ]));
        // Signature "boom" recurs across run 1 + run 2 (recurring); "rare" only
        // in run 3 (not recurring at min_runs=2). A success row is ignored.
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["acme", "acme", "acme", "acme"])),
                Arc::new(StringArray::from(vec!["app", "app", "app", "app"])),
                Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
                Arc::new(BooleanArray::from(vec![true, true, true, false])),
                Arc::new(StringArray::from(vec![
                    "build_error",
                    "build_error",
                    "test_failure",
                    "none",
                ])),
                Arc::new(StringArray::from(vec!["boom", "boom", "rare", ""])),
                Arc::new(StringArray::from(vec!["s1", "s2", "s3", ""])),
                Arc::new(StringArray::from(vec![
                    "2026-06-08T01:00:00.0Z",
                    "2026-06-08T02:00:00.0Z",
                    "2026-06-08T03:00:00.0Z",
                    "2026-06-08T04:00:00.0Z",
                ])),
            ],
        )
        .unwrap();

        let ctx = SessionContext::new();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        ctx.register_table("github_job_logs", Arc::new(table)).unwrap();

        let sql = recurring_failures_sql("2026-06-01T00:00:00.0Z", 2);
        let df = ctx.sql(&sql).await.unwrap();
        let rows = df.collect().await.unwrap();

        let total: usize = rows.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total, 1, "only the recurring signature should be returned");

        let batch = &rows[0];
        let sig = batch
            .column_by_name("failure_signature")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(sig.value(0), "boom");
        let runs = batch
            .column_by_name("runs")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(runs.value(0), 2);
    }
}
