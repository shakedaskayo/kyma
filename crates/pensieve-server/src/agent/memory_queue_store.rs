//! Persistence for the memory approval queue.
//!
//! Two backends behind one enum, mirroring how [`super::memory_settings`]
//! handles server vs local mode:
//!   - **Pg** — the `memory_approval_queue` table (server / worker-fabric mode),
//!     tenant-scoped on every query.
//!   - **Local** — a JSON file next to the local memory-settings file
//!     (`kyma serve` single-binary mode, no Postgres).
//!
//! The store is dumb: it inserts, lists, gets, counts, and flips status. All
//! policy + apply/inverse logic lives in [`super::memory_gate`].

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;
use uuid::Uuid;

use kyma_core::tenant::TenantId;

use super::memory_policy::MemoryOp;
use super::state::AgentState;

/// One row in the approval queue. Shared by both backends and serialized
/// verbatim to the local JSON file + the API responses. Timestamps are RFC3339
/// strings so the shape is identical regardless of backend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueRow {
    pub id: Uuid,
    pub realm: String,
    pub operation: MemoryOp,
    /// `gate` | `post_hoc`.
    pub mode: String,
    /// `pending` | `approved` | `rejected` | `auto_applied` | `rolled_back`.
    pub status: String,
    pub confidence: Option<f32>,
    pub reason: Option<String>,
    /// `dreaming` | `realtime` | `file_promote`.
    pub source: String,
    pub source_run_id: Option<Uuid>,
    /// The deferred (gate) or applied (post_hoc) op descriptor.
    pub payload: Value,
    /// Precomputed undo for post_hoc rows; `None` for gate rows (nothing applied).
    pub inverse: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
    pub resolution_comment: Option<String>,
}

impl QueueRow {
    /// Build a fresh row (new id, `created_at = now`). `mode`/`status` are set by
    /// the caller per disposition: gate ⇒ `("gate","pending")`, post-hoc ⇒
    /// `("post_hoc","auto_applied")`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: MemoryOp,
        realm: impl Into<String>,
        mode: &str,
        status: &str,
        confidence: Option<f32>,
        reason: Option<String>,
        source: &str,
        source_run_id: Option<Uuid>,
        payload: Value,
        inverse: Option<Value>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm: realm.into(),
            operation,
            mode: mode.to_string(),
            status: status.to_string(),
            confidence,
            reason,
            source: source.to_string(),
            source_run_id,
            payload,
            inverse,
            created_at: Utc::now().to_rfc3339(),
            resolved_at: None,
            resolved_by: None,
            resolution_comment: None,
        }
    }
}

/// Filters for [`QueueStore::list`]. `limit == 0` ⇒ the default cap.
#[derive(Debug, Default, Clone)]
pub struct QueueFilter {
    pub status: Option<String>,
    pub source: Option<String>,
    pub realm: Option<String>,
    pub operation: Option<MemoryOp>,
    pub source_run_id: Option<Uuid>,
    pub limit: usize,
}

const DEFAULT_LIMIT: i64 = 200;

/// Pending + post-hoc counts for the nav badge.
#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct QueueCounts {
    pub pending: i64,
    pub post_hoc: i64,
}

/// The two backends. Cheap to construct per request from [`AgentState`].
pub enum QueueStore {
    Pg { pool: PgPool, tenant: TenantId },
    Local { path: PathBuf },
}

/// One process-wide lock guarding local-file read-modify-write. There is only
/// ever one local deployment (single tenant, one file), so a single mutex is
/// sufficient to prevent lost updates between concurrent requests.
static LOCAL_LOCK: Mutex<()> = Mutex::const_new(());

impl QueueStore {
    /// Pick the backend for this server: Postgres when a pool exists, else a
    /// JSON file beside the local memory-settings file, else `None` (no durable
    /// place to queue — the gate degrades to apply-through).
    pub fn from_state(state: &AgentState) -> Option<QueueStore> {
        if let Some(pool) = state.pool.as_ref() {
            return Some(QueueStore::Pg {
                pool: pool.clone(),
                tenant: state.tenant,
            });
        }
        if let Some(p) = state.memory_settings_path.as_ref() {
            let path = p
                .parent()
                .map(|d| d.join("memory-approval-queue.json"))
                .unwrap_or_else(|| PathBuf::from("memory-approval-queue.json"));
            return Some(QueueStore::Local { path });
        }
        None
    }

    pub async fn insert(&self, row: &QueueRow) -> anyhow::Result<()> {
        match self {
            QueueStore::Pg { pool, tenant } => {
                sqlx::query(
                    "INSERT INTO memory_approval_queue \
                     (id, tenant_id, realm, operation, mode, status, confidence, reason, source, \
                      source_run_id, payload, inverse, created_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13::timestamptz)",
                )
                .bind(row.id)
                .bind(tenant.as_uuid())
                .bind(&row.realm)
                .bind(row.operation.as_str())
                .bind(&row.mode)
                .bind(&row.status)
                .bind(row.confidence)
                .bind(&row.reason)
                .bind(&row.source)
                .bind(row.source_run_id)
                .bind(&row.payload)
                .bind(&row.inverse)
                .bind(&row.created_at)
                .execute(pool)
                .await?;
                Ok(())
            }
            QueueStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let mut rows = read_local(path).await?;
                rows.push(row.clone());
                write_local(path, &rows).await
            }
        }
    }

    pub async fn list(&self, f: &QueueFilter) -> anyhow::Result<Vec<QueueRow>> {
        let limit = if f.limit == 0 {
            DEFAULT_LIMIT
        } else {
            f.limit as i64
        };
        match self {
            QueueStore::Pg { pool, tenant } => {
                let rows = sqlx::query(
                    "SELECT id, realm, operation, mode, status, confidence, reason, source, \
                            source_run_id, payload, inverse, created_at, resolved_at, resolved_by, \
                            resolution_comment \
                     FROM memory_approval_queue \
                     WHERE tenant_id = $1 \
                       AND ($2::text IS NULL OR status = $2) \
                       AND ($3::text IS NULL OR source = $3) \
                       AND ($4::text IS NULL OR realm = $4) \
                       AND ($5::text IS NULL OR operation = $5) \
                       AND ($6::uuid IS NULL OR source_run_id = $6) \
                     ORDER BY created_at DESC LIMIT $7",
                )
                .bind(tenant.as_uuid())
                .bind(&f.status)
                .bind(&f.source)
                .bind(&f.realm)
                .bind(f.operation.map(|o| o.as_str()))
                .bind(f.source_run_id)
                .bind(limit)
                .fetch_all(pool)
                .await?;
                Ok(rows.iter().filter_map(pg_to_row).collect())
            }
            QueueStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let mut rows = read_local(path).await?;
                rows.retain(|r| matches_filter(r, f));
                rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                rows.truncate(limit as usize);
                Ok(rows)
            }
        }
    }

    pub async fn get(&self, id: Uuid) -> anyhow::Result<Option<QueueRow>> {
        match self {
            QueueStore::Pg { pool, tenant } => {
                let row = sqlx::query(
                    "SELECT id, realm, operation, mode, status, confidence, reason, source, \
                            source_run_id, payload, inverse, created_at, resolved_at, resolved_by, \
                            resolution_comment \
                     FROM memory_approval_queue WHERE id = $1 AND tenant_id = $2",
                )
                .bind(id)
                .bind(tenant.as_uuid())
                .fetch_optional(pool)
                .await?;
                Ok(row.as_ref().and_then(pg_to_row))
            }
            QueueStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                Ok(read_local(path).await?.into_iter().find(|r| r.id == id))
            }
        }
    }

    pub async fn counts(&self) -> anyhow::Result<QueueCounts> {
        match self {
            QueueStore::Pg { pool, tenant } => {
                let row = sqlx::query(
                    "SELECT \
                       count(*) FILTER (WHERE status='pending')      AS pending, \
                       count(*) FILTER (WHERE status='auto_applied') AS post_hoc \
                     FROM memory_approval_queue WHERE tenant_id = $1",
                )
                .bind(tenant.as_uuid())
                .fetch_one(pool)
                .await?;
                Ok(QueueCounts {
                    pending: row.get::<i64, _>("pending"),
                    post_hoc: row.get::<i64, _>("post_hoc"),
                })
            }
            QueueStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let rows = read_local(path).await?;
                Ok(QueueCounts {
                    pending: rows.iter().filter(|r| r.status == "pending").count() as i64,
                    post_hoc: rows.iter().filter(|r| r.status == "auto_applied").count() as i64,
                })
            }
        }
    }

    /// Flip a row's status + stamp who/when/why. Returns the updated row (for
    /// the gate to act on its payload/inverse), or `None` if the id is unknown.
    pub async fn update_status(
        &self,
        id: Uuid,
        status: &str,
        by: Option<&str>,
        comment: Option<&str>,
    ) -> anyhow::Result<Option<QueueRow>> {
        match self {
            QueueStore::Pg { pool, tenant } => {
                let row = sqlx::query(
                    "UPDATE memory_approval_queue \
                     SET status = $3, resolved_at = now(), resolved_by = $4, resolution_comment = $5 \
                     WHERE id = $1 AND tenant_id = $2 \
                     RETURNING id, realm, operation, mode, status, confidence, reason, source, \
                               source_run_id, payload, inverse, created_at, resolved_at, resolved_by, \
                               resolution_comment",
                )
                .bind(id)
                .bind(tenant.as_uuid())
                .bind(status)
                .bind(by)
                .bind(comment)
                .fetch_optional(pool)
                .await?;
                Ok(row.as_ref().and_then(pg_to_row))
            }
            QueueStore::Local { path } => {
                let _g = LOCAL_LOCK.lock().await;
                let mut rows = read_local(path).await?;
                let mut updated = None;
                for r in rows.iter_mut() {
                    if r.id == id {
                        r.status = status.to_string();
                        r.resolved_at = Some(Utc::now().to_rfc3339());
                        r.resolved_by = by.map(str::to_string);
                        r.resolution_comment = comment.map(str::to_string);
                        updated = Some(r.clone());
                        break;
                    }
                }
                if updated.is_some() {
                    write_local(path, &rows).await?;
                }
                Ok(updated)
            }
        }
    }
}

fn matches_filter(r: &QueueRow, f: &QueueFilter) -> bool {
    f.status.as_ref().map_or(true, |s| &r.status == s)
        && f.source.as_ref().map_or(true, |s| &r.source == s)
        && f.realm.as_ref().map_or(true, |s| &r.realm == s)
        && f.operation.map_or(true, |o| r.operation == o)
        && f.source_run_id.map_or(true, |id| r.source_run_id == Some(id))
}

/// Map a Postgres row to a [`QueueRow`]. Returns `None` if the stored
/// `operation` string is unrecognized (forward-compat: skip rather than panic).
fn pg_to_row(r: &sqlx::postgres::PgRow) -> Option<QueueRow> {
    let op = MemoryOp::parse(r.get::<String, _>("operation").as_str())?;
    Some(QueueRow {
        id: r.get("id"),
        realm: r.get("realm"),
        operation: op,
        mode: r.get("mode"),
        status: r.get("status"),
        confidence: r.get("confidence"),
        reason: r.get("reason"),
        source: r.get("source"),
        source_run_id: r.get("source_run_id"),
        payload: r.get("payload"),
        inverse: r.get("inverse"),
        created_at: r.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
        resolved_at: r
            .get::<Option<DateTime<Utc>>, _>("resolved_at")
            .map(|t| t.to_rfc3339()),
        resolved_by: r.get("resolved_by"),
        resolution_comment: r.get("resolution_comment"),
    })
}

async fn read_local(path: &PathBuf) -> anyhow::Result<Vec<QueueRow>> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) if !raw.trim().is_empty() => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        _ => Ok(Vec::new()),
    }
}

async fn write_local(path: &PathBuf, rows: &[QueueRow]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(path, serde_json::to_string_pretty(rows)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(op: MemoryOp, realm: &str, status: &str) -> QueueRow {
        QueueRow::new(
            op,
            realm,
            "gate",
            status,
            Some(0.4),
            Some("dup".into()),
            "dreaming",
            None,
            json!({"k": "v"}),
            None,
        )
    }

    #[tokio::test]
    async fn local_roundtrip_insert_list_get_update_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::Local {
            path: dir.path().join("nested").join("queue.json"),
        };

        // Empty store.
        assert!(store.list(&QueueFilter::default()).await.unwrap().is_empty());
        assert_eq!(store.counts().await.unwrap().pending, 0);

        // Insert two; list is newest-first.
        let a = row(MemoryOp::Merge, "r1", "pending");
        // ensure distinct created_at ordering
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let b = row(MemoryOp::Invalidate, "r2", "pending");
        store.insert(&a).await.unwrap();
        store.insert(&b).await.unwrap();

        let all = store.list(&QueueFilter::default()).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, b.id, "newest first");

        // Filter by realm + operation.
        let f = QueueFilter {
            realm: Some("r1".into()),
            ..Default::default()
        };
        let only_r1 = store.list(&f).await.unwrap();
        assert_eq!(only_r1.len(), 1);
        assert_eq!(only_r1[0].operation, MemoryOp::Merge);

        // Get by id.
        assert_eq!(store.get(a.id).await.unwrap().unwrap().id, a.id);
        assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());

        // Counts.
        assert_eq!(store.counts().await.unwrap().pending, 2);

        // Update status → approved, stamps resolver.
        let updated = store
            .update_status(a.id, "approved", Some("alice"), Some("ok"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, "approved");
        assert_eq!(updated.resolved_by.as_deref(), Some("alice"));
        assert!(updated.resolved_at.is_some());
        assert_eq!(store.counts().await.unwrap().pending, 1);

        // Filter by status.
        let pending = store
            .list(&QueueFilter {
                status: Some("pending".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, b.id);
    }

    #[tokio::test]
    async fn local_update_unknown_id_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = QueueStore::Local {
            path: dir.path().join("queue.json"),
        };
        assert!(store
            .update_status(Uuid::new_v4(), "approved", None, None)
            .await
            .unwrap()
            .is_none());
    }
}
