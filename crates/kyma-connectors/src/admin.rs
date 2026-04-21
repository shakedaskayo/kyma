//! HTTP admin API — /v1/connectors CRUD.

use crate::catalog_sql;
use crate::registry::ConnectorRegistry;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kyma_catalog::PostgresCatalog;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminState {
    pub catalog: Arc<PostgresCatalog>,
    pub registry: Arc<ConnectorRegistry>,
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/v1/connectors", post(create).get(list))
        .route(
            "/v1/connectors/:id",
            get(get_one).patch(patch_one).delete(delete_one),
        )
        .route("/v1/connectors/:id/pause", post(pause))
        .route("/v1/connectors/:id/resume", post(resume))
        .route("/v1/connectors/:id/trigger", post(trigger))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateReq {
    name: String,
    #[serde(rename = "type")]
    type_id: String,
    target_database: String,
    target_table: String,
    schedule_ms: i64,
    config: serde_json::Value,
}

#[derive(Serialize)]
struct IdResp {
    id: Uuid,
}

async fn create(State(s): State<AdminState>, Json(req): Json<CreateReq>) -> impl IntoResponse {
    let Some(c) = s.registry.lookup(&req.type_id) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unknown type {}", req.type_id),
            })),
        )
            .into_response();
    };
    if let Err(e) = c.validate_config(&req.config) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.0 })),
        )
            .into_response();
    }
    if !(100..=86_400_000).contains(&req.schedule_ms) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "schedule_ms must be in [100, 86400000]",
            })),
        )
            .into_response();
    }
    let res = catalog_sql::create_connector_direct(
        s.catalog.pool(),
        &req.name,
        &req.type_id,
        &req.target_database,
        &req.target_table,
        req.config,
        req.schedule_ms,
        "periodic",
    )
    .await;
    match res {
        Ok(id) => (StatusCode::CREATED, Json(IdResp { id })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn list(State(s): State<AdminState>) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, name, type, enabled FROM connectors ORDER BY name",
    )
    .fetch_all(s.catalog.pool())
    .await;
    match rows {
        Ok(rows) => {
            let items: Vec<_> = rows
                .into_iter()
                .map(|(id, name, type_id, enabled)| {
                    serde_json::json!({
                        "id": id,
                        "name": name,
                        "type": type_id,
                        "enabled": enabled,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_one(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match catalog_sql::load_connector(s.catalog.pool(), id).await {
        Ok(Some(c)) => {
            let scrubbed = scrub_secrets(c.config_jsonb.clone());
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": c.id,
                    "name": c.name,
                    "type": c.type_id,
                    "target_database": c.target_database,
                    "target_table": c.target_table,
                    "schedule_ms": c.schedule_ms,
                    "drive_model": c.drive_model,
                    "enabled": c.enabled,
                    "disabled_reason": c.disabled_reason,
                    "last_run_at": c.last_run_at,
                    "last_success_at": c.last_success_at,
                    "last_error": c.last_error,
                    "last_rows_ingested": c.last_rows_ingested,
                    "config": scrubbed,
                })),
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

fn scrub_secrets(mut v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    fn looks_secret(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n.contains("token") || n.contains("password") || n.contains("secret") || n.contains("key")
    }
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, vv) in m.iter_mut() {
                    match vv {
                        Value::String(s) => {
                            if looks_secret(k) && !s.starts_with("$env:") {
                                *s = "***".into();
                            }
                        }
                        _ => walk(vv),
                    }
                }
            }
            Value::Array(a) => {
                for vv in a.iter_mut() {
                    walk(vv);
                }
            }
            _ => {}
        }
    }
    walk(&mut v);
    v
}

#[derive(Deserialize)]
struct PatchReq {
    name: Option<String>,
    schedule_ms: Option<i64>,
    enabled: Option<bool>,
    config: Option<serde_json::Value>,
}

async fn patch_one(
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchReq>,
) -> impl IntoResponse {
    if let Some(sched_ms) = req.schedule_ms {
        if !(100..=86_400_000).contains(&sched_ms) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "schedule_ms must be in [100, 86400000]",
                })),
            )
                .into_response();
        }
    }
    if let Some(cfg) = &req.config {
        let Some(c) = catalog_sql::load_connector(s.catalog.pool(), id)
            .await
            .ok()
            .flatten()
        else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Some(impl_) = s.registry.lookup(&c.type_id) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown type {}", c.type_id) })),
            )
                .into_response();
        };
        if let Err(e) = impl_.validate_config(cfg) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.0 })),
            )
                .into_response();
        }
    }
    let res = sqlx::query(
        "UPDATE connectors SET
             name = COALESCE($2, name),
             schedule_ms = COALESCE($3, schedule_ms),
             enabled = COALESCE($4, enabled),
             config_jsonb = COALESCE($5, config_jsonb),
             disabled_reason = CASE WHEN $4 = TRUE THEN NULL ELSE disabled_reason END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.schedule_ms)
    .bind(req.enabled)
    .bind(req.config.as_ref())
    .execute(s.catalog.pool())
    .await;
    match res {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn delete_one(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let res = sqlx::query("DELETE FROM connectors WHERE id = $1")
        .bind(id)
        .execute(s.catalog.pool())
        .await;
    match res {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn pause(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = FALSE, disabled_reason = 'manual',
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(s.catalog.pool())
    .await;
    StatusCode::NO_CONTENT
}

async fn resume(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = TRUE, disabled_reason = NULL,
         updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(s.catalog.pool())
    .await;
    StatusCode::NO_CONTENT
}

async fn trigger(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = catalog_sql::enqueue_tick(s.catalog.pool(), id, now_ms).await;
    StatusCode::ACCEPTED
}
