//! Object-store artifact catalog.
//!
//! Tracking rows for full-file blobs (CI job logs, agent-contributed files,
//! filesystem-watch snapshots) that live on the file layer outside the columnar
//! extents. Like retention, this is Postgres-only — the sqlite/local catalog
//! never runs artifact GC — so these are inherent methods on `PostgresCatalog`
//! rather than `Catalog` trait methods (callers reach them via the same
//! `as_ref_any().downcast_ref::<PostgresCatalog>()` path the retention worker
//! uses).

use chrono::{DateTime, Utc};
use kyma_core::errors::{CatalogError, Result};
use kyma_core::tenant::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::PostgresCatalog;

/// A tracked object-store artifact.
///
/// `id` / `created_at` / `deleted_at` are populated on read and ignored by
/// [`PostgresCatalog::register_artifact`] — the row identity is the
/// `(tenant_id, object_path)` unique key.
#[derive(Debug, Clone)]
pub struct ArtifactRecord {
    pub id: Option<Uuid>,
    pub tenant_id: TenantId,
    pub object_path: String,
    /// Connector / origin, e.g. `"github"`, `"fswatch"`.
    pub source: String,
    /// `"log"` | `"file"` | …
    pub artifact_class: String,
    /// `"database.table"` the blob is referenced from, if any.
    pub table_ref: Option<String>,
    pub connector_id: Option<Uuid>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// `None` = retain forever.
    pub expires_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

fn sql(e: sqlx::Error) -> CatalogError {
    CatalogError::Sql(e.to_string())
}

fn row_to_artifact(r: &sqlx::postgres::PgRow) -> Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: Some(r.try_get("id").map_err(sql)?),
        tenant_id: TenantId::from_uuid(r.try_get("tenant_id").map_err(sql)?),
        object_path: r.try_get("object_path").map_err(sql)?,
        source: r.try_get("source").map_err(sql)?,
        artifact_class: r.try_get("artifact_class").map_err(sql)?,
        table_ref: r.try_get("table_ref").map_err(sql)?,
        connector_id: r.try_get("connector_id").map_err(sql)?,
        size_bytes: r.try_get("size_bytes").map_err(sql)?,
        sha256: r.try_get("sha256").map_err(sql)?,
        created_at: Some(r.try_get("created_at").map_err(sql)?),
        expires_at: r.try_get("expires_at").map_err(sql)?,
        deleted_at: r.try_get("deleted_at").map_err(sql)?,
    })
}

impl PostgresCatalog {
    /// Upsert an artifact tracking row, keyed on `(tenant_id, object_path)`.
    /// Returns the row id. Re-registering the same path (connectors re-tick,
    /// fs-watch re-fires) updates metadata in place — and clears `deleted_at`,
    /// resurrecting a row whose blob was re-captured after expiry.
    pub async fn register_artifact(&self, rec: &ArtifactRecord) -> Result<Uuid> {
        let row: (Uuid,) = sqlx::query_as(
            "INSERT INTO artifacts
                (tenant_id, object_path, source, artifact_class, table_ref,
                 connector_id, size_bytes, sha256, expires_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (tenant_id, object_path) DO UPDATE SET
                source         = EXCLUDED.source,
                artifact_class = EXCLUDED.artifact_class,
                table_ref      = EXCLUDED.table_ref,
                connector_id   = EXCLUDED.connector_id,
                size_bytes     = EXCLUDED.size_bytes,
                sha256         = EXCLUDED.sha256,
                expires_at     = EXCLUDED.expires_at,
                deleted_at     = NULL
             RETURNING id",
        )
        .bind(rec.tenant_id.as_uuid())
        .bind(&rec.object_path)
        .bind(&rec.source)
        .bind(&rec.artifact_class)
        .bind(&rec.table_ref)
        .bind(rec.connector_id)
        .bind(rec.size_bytes)
        .bind(&rec.sha256)
        .bind(rec.expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(sql)?;
        Ok(row.0)
    }

    /// Fetch an artifact by id, scoped to `tenant`. A caller in another tenant
    /// gets `None` even with a valid id.
    pub async fn get_artifact_in_tenant(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<Option<ArtifactRecord>> {
        let row = sqlx::query(
            "SELECT id, tenant_id, object_path, source, artifact_class, table_ref,
                    connector_id, size_bytes, sha256, created_at, expires_at, deleted_at
             FROM artifacts WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(sql)?;
        match row {
            Some(r) => Ok(Some(row_to_artifact(&r)?)),
            None => Ok(None),
        }
    }

    /// Distinct tenants that currently have live (non-deleted) artifacts — the
    /// set the catch-all artifact-graph sync must iterate.
    pub async fn list_artifact_tenants(&self) -> Result<Vec<TenantId>> {
        let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT DISTINCT tenant_id FROM artifacts WHERE deleted_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        Ok(rows.into_iter().map(|(id,)| TenantId::from_uuid(id)).collect())
    }

    /// All live (not soft-deleted) artifacts for `tenant`, newest first. Used by
    /// the catch-all graph sync to materialize artifact nodes.
    pub async fn list_live_artifacts(&self, tenant: TenantId) -> Result<Vec<ArtifactRecord>> {
        let rows = sqlx::query(
            "SELECT id, tenant_id, object_path, source, artifact_class, table_ref,
                    connector_id, size_bytes, sha256, created_at, expires_at, deleted_at
             FROM artifacts
             WHERE tenant_id = $1 AND deleted_at IS NULL
             ORDER BY created_at DESC",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        rows.iter().map(row_to_artifact).collect()
    }

    /// Pass 1: soft-delete live artifacts whose `expires_at` is past `now`.
    /// Returns `(id, object_path)` for each swept row.
    pub async fn soft_delete_expired_artifacts(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "UPDATE artifacts SET deleted_at = $1
             WHERE deleted_at IS NULL
               AND expires_at IS NOT NULL
               AND expires_at < $1
             RETURNING id, object_path",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        Ok(rows)
    }

    /// Pass 2: rows soft-deleted before `before` (grace cutoff). The worker
    /// deletes their blobs, then calls [`PostgresCatalog::delete_artifact_rows`].
    pub async fn artifact_gc_candidates(
        &self,
        before: DateTime<Utc>,
    ) -> Result<Vec<(Uuid, String)>> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, object_path FROM artifacts
             WHERE deleted_at IS NOT NULL AND deleted_at < $1",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        Ok(rows)
    }

    /// Physically remove artifact rows by id (after their blobs are deleted).
    pub async fn delete_artifact_rows(&self, ids: &[Uuid]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query("DELETE FROM artifacts WHERE id = ANY($1)")
            .bind(ids)
            .execute(&self.pool)
            .await
            .map_err(sql)?;
        Ok(())
    }
}
