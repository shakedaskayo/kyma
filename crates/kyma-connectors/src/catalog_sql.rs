//! Direct SQL helpers against PostgresCatalog::pool().
//!
//! Connector scheduler + runner read/write a handful of connector-
//! specific rows that don't warrant growing the Catalog trait. This
//! module is the one place those SQL statements live.

use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ConnectorRow {
    pub id: Uuid,
    pub name: String,
    pub type_id: String,
    pub target_database: String,
    pub target_table: String,
    pub config_jsonb: serde_json::Value,
    pub schedule_ms: i64,
    pub drive_model: String,
    pub enabled: bool,
}

/// Create a connector row (used from admin API + test setup).
#[allow(clippy::too_many_arguments)]
pub async fn create_connector_direct(
    pool: &PgPool,
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
           (name, type, target_database, target_table, config_jsonb,
            schedule_ms, drive_model)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
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

/// List periodic, enabled connectors due for a tick.
pub async fn list_due_periodic(pool: &PgPool) -> Result<Vec<ConnectorRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled, last_run_at
         FROM connectors
         WHERE enabled = TRUE AND drive_model = 'periodic'
           AND (last_run_at IS NULL
                OR last_run_at < now() - (schedule_ms || ' milliseconds')::interval)",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(ConnectorRow {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                type_id: r.try_get("type")?,
                target_database: r.try_get("target_database")?,
                target_table: r.try_get("target_table")?,
                config_jsonb: r.try_get("config_jsonb")?,
                schedule_ms: r.try_get("schedule_ms")?,
                drive_model: r.try_get("drive_model")?,
                enabled: r.try_get("enabled")?,
            })
        })
        .collect()
}

pub async fn load_connector(pool: &PgPool, id: Uuid) -> Result<Option<ConnectorRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled
         FROM connectors WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    Ok(Some(ConnectorRow {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        type_id: r.try_get("type")?,
        target_database: r.try_get("target_database")?,
        target_table: r.try_get("target_table")?,
        config_jsonb: r.try_get("config_jsonb")?,
        schedule_ms: r.try_get("schedule_ms")?,
        drive_model: r.try_get("drive_model")?,
        enabled: r.try_get("enabled")?,
    }))
}

pub async fn load_cursor(
    pool: &PgPool,
    connector_id: Uuid,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT cursor_jsonb FROM connector_cursors WHERE connector_id = $1")
            .bind(connector_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| if v.is_null() { None } else { Some(v) }))
}

pub async fn upsert_cursor(
    pool: &PgPool,
    connector_id: Uuid,
    cursor: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_cursors (connector_id, cursor_jsonb)
         VALUES ($1, $2)
         ON CONFLICT (connector_id)
         DO UPDATE SET cursor_jsonb = EXCLUDED.cursor_jsonb, updated_at = now()",
    )
    .bind(connector_id)
    .bind(cursor)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_success(
    pool: &PgPool,
    connector_id: Uuid,
    rows_ingested: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_success_at = now(),
             last_error = NULL,
             last_rows_ingested = $2,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
    .bind(rows_ingested)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_failure(
    pool: &PgPool,
    connector_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_error = $2,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn disable_connector(
    pool: &PgPool,
    connector_id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET enabled = FALSE, disabled_reason = $2, updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
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
    connector_id: Uuid,
    scheduled_for_ms: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO background_tasks (kind, payload, priority)
         VALUES ('connector_tick',
                 jsonb_build_object(
                     'connector_id', $1::text,
                     'scheduled_for', $2::text),
                 0)
         ON CONFLICT DO NOTHING",
    )
    .bind(connector_id)
    .bind(scheduled_for_ms)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
