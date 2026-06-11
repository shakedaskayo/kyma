//! Direct SQL helpers against PostgresCatalog::pool().
//!
//! DataSource scheduler + runner read/write a handful of data-source-
//! specific rows that don't warrant growing the Catalog trait. This
//! module is the one place those SQL statements live.
//!
//! Every helper is tenant-scoped: callers must pass a [`TenantId`], and
//! mutating helpers bind it on INSERT, while readers/updaters/deleters
//! constrain by `WHERE tenant_id = ...`. The one cluster-global helper is
//! [`list_due_periodic`], which the data source scheduler invokes without a
//! tenant context — it returns each row's `tenant_id` so the caller can
//! propagate it (e.g., to [`enqueue_tick`]).

use kyma_core::tenant::TenantId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DataSourceRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub name: String,
    pub type_id: String,
    pub target_database: String,
    pub target_table: String,
    pub config_jsonb: serde_json::Value,
    pub schedule_ms: i64,
    pub drive_model: String,
    pub enabled: bool,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_success_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub last_rows_ingested: Option<i64>,
    pub disabled_reason: Option<String>,
}

fn row_to_connector(r: sqlx::postgres::PgRow) -> Result<DataSourceRow, sqlx::Error> {
    let tenant_uuid: Uuid = r.try_get("tenant_id")?;
    Ok(DataSourceRow {
        id: r.try_get("id")?,
        tenant_id: TenantId::from_uuid(tenant_uuid),
        name: r.try_get("name")?,
        type_id: r.try_get("type")?,
        target_database: r.try_get("target_database")?,
        target_table: r.try_get("target_table")?,
        config_jsonb: r.try_get("config_jsonb")?,
        schedule_ms: r.try_get("schedule_ms")?,
        drive_model: r.try_get("drive_model")?,
        enabled: r.try_get("enabled")?,
        last_run_at: r.try_get("last_run_at")?,
        last_success_at: r.try_get("last_success_at")?,
        last_error: r.try_get("last_error")?,
        last_rows_ingested: r.try_get("last_rows_ingested")?,
        disabled_reason: r.try_get("disabled_reason")?,
    })
}

/// Create a data source row (used from admin API + test setup).
#[allow(clippy::too_many_arguments)]
pub async fn create_connector_direct(
    pool: &PgPool,
    tenant: TenantId,
    name: &str,
    type_id: &str,
    target_database: &str,
    target_table: &str,
    config: serde_json::Value,
    schedule_ms: i64,
    drive_model: &str,
) -> Result<Uuid, sqlx::Error> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO connectors
           (tenant_id, name, type, target_database, target_table, config_jsonb,
            schedule_ms, drive_model)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id",
    )
    .bind(tenant.as_uuid())
    .bind(name)
    .bind(type_id)
    .bind(target_database)
    .bind(target_table)
    .bind(&config)
    .bind(schedule_ms)
    .bind(drive_model)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List periodic, enabled data sources due for a tick across all tenants.
///
/// Cluster-global: the scheduler runs without a tenant context, so we
/// return every due data source and include each row's `tenant_id` so the
/// caller can thread it through to [`enqueue_tick`].
pub async fn list_due_periodic(pool: &PgPool) -> Result<Vec<DataSourceRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, tenant_id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled,
                last_run_at, last_success_at, last_error,
                last_rows_ingested, disabled_reason
         FROM connectors
         WHERE enabled = TRUE AND drive_model = 'periodic'
           AND (last_run_at IS NULL
                OR last_run_at < now() - (schedule_ms || ' milliseconds')::interval)",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_connector).collect()
}

pub async fn load_connector(
    pool: &PgPool,
    tenant: TenantId,
    id: Uuid,
) -> Result<Option<DataSourceRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, tenant_id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled,
                last_run_at, last_success_at, last_error,
                last_rows_ingested, disabled_reason
         FROM connectors WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    Ok(Some(row_to_connector(r)?))
}

pub async fn load_cursor(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT cursor_jsonb FROM connector_cursors
         WHERE tenant_id = $1 AND connector_id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| if v.is_null() { None } else { Some(v) }))
}

pub async fn upsert_cursor(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    cursor: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_cursors (tenant_id, connector_id, cursor_jsonb)
         VALUES ($1, $2, $3)
         ON CONFLICT (connector_id)
         DO UPDATE SET cursor_jsonb = EXCLUDED.cursor_jsonb, updated_at = now()",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(cursor)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_success(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    rows_ingested: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_success_at = now(),
             last_error = NULL,
             last_rows_ingested = $3,
             updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(rows_ingested)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_failure(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_error = $3,
             updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn disable_connector(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET enabled = FALSE, disabled_reason = $3, updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enqueue a single connector_tick task with the bucketed `scheduled_for`.
/// Race-safe: the partial unique index on `background_tasks` turns duplicate
/// inserts into `Ok(0)` (we use ON CONFLICT DO NOTHING).
pub async fn enqueue_tick(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    scheduled_for_ms: i64,
) -> Result<u64, sqlx::Error> {
    // tenant_id lives both on the row (NOT NULL column) and in the JSON
    // payload so the runner can recover it without a second SELECT.
    let res = sqlx::query(
        "INSERT INTO background_tasks (tenant_id, kind, payload, priority)
         VALUES ($1,
                 'connector_tick',
                 jsonb_build_object(
                     'tenant_id', $1::text,
                     'connector_id', $2::text,
                     'scheduled_for', $3::text),
                 0)
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(scheduled_for_ms)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Enqueue a `datasource_sync` job on the worker fabric (`jobs` table) with
/// the bucketed `scheduled_for` — the fabric successor of [`enqueue_tick`].
/// Race-safe via the partial unique index `jobs_connector_sync_uniq`
/// (ON CONFLICT DO NOTHING → `Ok(0)` on duplicates).
pub async fn enqueue_connector_sync(
    pool: &PgPool,
    tenant: TenantId,
    data_source_id: Uuid,
    scheduled_for_ms: i64,
) -> Result<u64, sqlx::Error> {
    // tenant_id lives both on the row (NOT NULL column) and in the JSON
    // payload so the executor can recover it without a second SELECT.
    let res = sqlx::query(
        "INSERT INTO jobs (tenant_id, kind, payload, priority, req_capabilities)
         VALUES ($1,
                 'connector_sync',
                 jsonb_build_object(
                     'tenant_id', $1::text,
                     'connector_id', $2::text,
                     'scheduled_for', $3::text),
                 0,
                 ARRAY['connector'])
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(data_source_id)
    .bind(scheduled_for_ms)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
