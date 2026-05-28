//! Saved Discover views: a named scope (set of `db.table` patterns) owned by a
//! user. Scope is the only thing persisted; search text, filters, and time range
//! stay ephemeral (per spec section 4.2).
//!
//! See migration `013_saved_views.sql` for the table shape.

use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// A saved Discover view.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedView {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub owner_subject: String,
    pub name: String,
    pub sources: Vec<String>,
    pub columns: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Payload for [`create`].
#[derive(Clone, Debug, Deserialize)]
pub struct NewSavedView {
    pub name: String,
    pub sources: Vec<String>,
    #[serde(default)]
    pub columns: Option<serde_json::Value>,
}

/// Patch payload for [`update`]. Any `None` field leaves the existing value
/// untouched.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpdateSavedView {
    pub name: Option<String>,
    pub sources: Option<Vec<String>>,
    pub columns: Option<serde_json::Value>,
}

/// Errors surfaced by this module.
#[derive(Debug, thiserror::Error)]
pub enum SavedViewError {
    #[error("saved view not found")]
    NotFound,
    #[error("name conflict")]
    NameConflict,
    #[error("database error: {0}")]
    Db(String),
}

/// Insert a new saved view for `owner` in `tenant_id`.
pub async fn create(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    input: NewSavedView,
) -> Result<SavedView, SavedViewError> {
    let sources_json = serde_json::to_value(&input.sources)
        .map_err(|e| SavedViewError::Db(format!("encode sources: {e}")))?;
    let row = sqlx::query(
        "INSERT INTO saved_views (tenant_id, owner_subject, name, sources_json, columns_json)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, created_at, updated_at",
    )
    .bind(tenant_id)
    .bind(owner)
    .bind(&input.name)
    .bind(&sources_json)
    .bind(&input.columns)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx_error)?;

    let id: Uuid = row
        .try_get("id")
        .map_err(|e| SavedViewError::Db(e.to_string()))?;
    let created_at = row
        .try_get("created_at")
        .map_err(|e| SavedViewError::Db(e.to_string()))?;
    let updated_at = row
        .try_get("updated_at")
        .map_err(|e| SavedViewError::Db(e.to_string()))?;

    Ok(SavedView {
        id,
        tenant_id,
        owner_subject: owner.to_string(),
        name: input.name,
        sources: input.sources,
        columns: input.columns,
        created_at,
        updated_at,
    })
}

/// List saved views for `owner` in `tenant_id`, newest first.
pub async fn list(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
) -> Result<Vec<SavedView>, SavedViewError> {
    let rows = sqlx::query(
        "SELECT id, name, sources_json, columns_json, created_at, updated_at
         FROM saved_views
         WHERE tenant_id = $1 AND owner_subject = $2
         ORDER BY updated_at DESC",
    )
    .bind(tenant_id)
    .bind(owner)
    .fetch_all(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let sources_json: serde_json::Value = r
            .try_get("sources_json")
            .map_err(|e| SavedViewError::Db(e.to_string()))?;
        let sources: Vec<String> = serde_json::from_value(sources_json)
            .map_err(|e| SavedViewError::Db(format!("decode sources: {e}")))?;
        out.push(SavedView {
            id: r
                .try_get("id")
                .map_err(|e| SavedViewError::Db(e.to_string()))?,
            tenant_id,
            owner_subject: owner.to_string(),
            name: r
                .try_get("name")
                .map_err(|e| SavedViewError::Db(e.to_string()))?,
            sources,
            columns: r.try_get("columns_json").ok(),
            created_at: r
                .try_get("created_at")
                .map_err(|e| SavedViewError::Db(e.to_string()))?,
            updated_at: r
                .try_get("updated_at")
                .map_err(|e| SavedViewError::Db(e.to_string()))?,
        });
    }
    Ok(out)
}

/// Fetch a single saved view by id, scoped to `tenant_id` + `owner`.
pub async fn get(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
) -> Result<SavedView, SavedViewError> {
    let row = sqlx::query(
        "SELECT id, name, sources_json, columns_json, created_at, updated_at
         FROM saved_views
         WHERE tenant_id = $1 AND owner_subject = $2 AND id = $3",
    )
    .bind(tenant_id)
    .bind(owner)
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?
    .ok_or(SavedViewError::NotFound)?;

    let sources_json: serde_json::Value = row
        .try_get("sources_json")
        .map_err(|e| SavedViewError::Db(e.to_string()))?;
    let sources: Vec<String> = serde_json::from_value(sources_json)
        .map_err(|e| SavedViewError::Db(format!("decode sources: {e}")))?;

    Ok(SavedView {
        id: row
            .try_get("id")
            .map_err(|e| SavedViewError::Db(e.to_string()))?,
        tenant_id,
        owner_subject: owner.to_string(),
        name: row
            .try_get("name")
            .map_err(|e| SavedViewError::Db(e.to_string()))?,
        sources,
        columns: row.try_get("columns_json").ok(),
        created_at: row
            .try_get("created_at")
            .map_err(|e| SavedViewError::Db(e.to_string()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| SavedViewError::Db(e.to_string()))?,
    })
}

/// Patch-update a saved view. `None` fields are preserved from the existing row.
pub async fn update(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
    input: UpdateSavedView,
) -> Result<SavedView, SavedViewError> {
    let current = get(pool, tenant_id, owner, id).await?;
    let new_name = input.name.unwrap_or(current.name);
    let new_sources = input.sources.unwrap_or(current.sources);
    let new_columns = input.columns.or(current.columns);
    let sources_json = serde_json::to_value(&new_sources)
        .map_err(|e| SavedViewError::Db(format!("encode sources: {e}")))?;

    sqlx::query(
        "UPDATE saved_views
         SET name = $1, sources_json = $2, columns_json = $3, updated_at = NOW()
         WHERE tenant_id = $4 AND owner_subject = $5 AND id = $6",
    )
    .bind(&new_name)
    .bind(&sources_json)
    .bind(&new_columns)
    .bind(tenant_id)
    .bind(owner)
    .bind(id)
    .execute(pool)
    .await
    .map_err(map_sqlx_error)?;

    get(pool, tenant_id, owner, id).await
}

/// Delete a saved view by id.
pub async fn delete(
    pool: &PgPool,
    tenant_id: Uuid,
    owner: &str,
    id: Uuid,
) -> Result<(), SavedViewError> {
    let res = sqlx::query(
        "DELETE FROM saved_views WHERE tenant_id = $1 AND owner_subject = $2 AND id = $3",
    )
    .bind(tenant_id)
    .bind(owner)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| SavedViewError::Db(e.to_string()))?;
    if res.rows_affected() == 0 {
        return Err(SavedViewError::NotFound);
    }
    Ok(())
}

/// Map a sqlx error, surfacing Postgres unique-violation (sqlstate 23505) as
/// [`SavedViewError::NameConflict`].
fn map_sqlx_error(e: sqlx::Error) -> SavedViewError {
    if let Some(db_err) = e.as_database_error() {
        if db_err.code().as_deref() == Some("23505") {
            return SavedViewError::NameConflict;
        }
    }
    SavedViewError::Db(e.to_string())
}
