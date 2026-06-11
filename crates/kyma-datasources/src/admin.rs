//! HTTP admin API — /v1/connectors CRUD.

use crate::catalog_sql;
use crate::registry::ConnectorRegistry;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use kyma_catalog::PostgresCatalog;
use kyma_core::tenant::TenantId;
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
        .route("/v1/connectors/catalog", get(catalog))
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
    /// Graph-style connectors that emit multiple tables via `ConnectorRun::tables`
    /// may omit `target_table` (or pass an empty string); the connector's
    /// `GraphHint` dictates the actual tables. Single-table connectors
    /// (e.g. Prometheus) should still supply it.
    #[serde(default)]
    target_table: String,
    schedule_ms: i64,
    config: serde_json::Value,
}

#[derive(Serialize)]
struct IdResp {
    id: Uuid,
}

async fn create(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Json(req): Json<CreateReq>,
) -> impl IntoResponse {
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
        tenant,
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

/// `GET /v1/connectors/catalog` — the vendor-agnostic catalog that drives the
/// connectors UI. Merges registered connectors (self-described, status
/// `"available"`) with the static `coming_soon` list (registered wins on a
/// `type_id` collision), sorted available-first then by category and label.
async fn catalog(State(s): State<AdminState>) -> impl IntoResponse {
    let mut entries = s.registry.catalog();
    let have: std::collections::HashSet<String> =
        entries.iter().map(|e| e.type_id.clone()).collect();
    for cs in crate::catalog::coming_soon() {
        if !have.contains(&cs.type_id) {
            entries.push(cs);
        }
    }
    entries.sort_by(|a, b| {
        let av = (a.status != "available", a.category.clone(), a.label.clone());
        let bv = (b.status != "available", b.category.clone(), b.label.clone());
        av.cmp(&bv)
    });
    // For OAuth connectors, surface the provider slug + scopes the UI needs to
    // drive the connect flow (`POST /v1/oauth/{provider}/start`). Injected here
    // rather than carried on every CatalogEntry so non-OAuth connectors stay
    // untouched.
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            let mut v = serde_json::to_value(&e).unwrap_or(serde_json::Value::Null);
            if e.auth_mode == "oauth" {
                if let serde_json::Value::Object(ref mut m) = v {
                    let provider = crate::oauth::provider_for_connector(&e.type_id);
                    if !provider.is_empty() {
                        m.insert("oauth_provider".into(), serde_json::Value::String(provider.to_string()));
                        let scopes = crate::oauth::scopes_for_connector(&e.type_id);
                        if !scopes.is_empty() {
                            m.insert(
                                "oauth_scopes".into(),
                                serde_json::Value::Array(
                                    scopes.into_iter().map(serde_json::Value::String).collect(),
                                ),
                            );
                        }
                    }
                }
            }
            v
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response()
}

async fn list(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, name, type, enabled FROM connectors
         WHERE tenant_id = $1
         ORDER BY name",
    )
    .bind(tenant.as_uuid())
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

async fn get_one(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match catalog_sql::load_connector(s.catalog.pool(), tenant, id).await {
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
    Extension(tenant): Extension<TenantId>,
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
        let Some(c) = catalog_sql::load_connector(s.catalog.pool(), tenant, id)
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
             name = COALESCE($3, name),
             schedule_ms = COALESCE($4, schedule_ms),
             enabled = COALESCE($5, enabled),
             config_jsonb = COALESCE($6, config_jsonb),
             disabled_reason = CASE WHEN $5 = TRUE THEN NULL ELSE disabled_reason END,
             updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
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

async fn delete_one(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let res = sqlx::query("DELETE FROM connectors WHERE tenant_id = $1 AND id = $2")
        .bind(tenant.as_uuid())
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

async fn pause(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = FALSE, disabled_reason = 'manual',
         updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(id)
    .execute(s.catalog.pool())
    .await;
    StatusCode::NO_CONTENT
}

async fn resume(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = TRUE, disabled_reason = NULL,
         updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(id)
    .execute(s.catalog.pool())
    .await;
    StatusCode::NO_CONTENT
}

async fn trigger(
    Extension(tenant): Extension<TenantId>,
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let now_ms = chrono::Utc::now().timestamp_millis();
    // Align the manual trigger to the scheduler's time-bucket so it dedups
    // (ON CONFLICT DO NOTHING) with any pending/just-run scheduled tick. Without
    // this a trigger enqueues a *separate* task that the SKIP-LOCKED workers run
    // concurrently with the scheduled tick — both re-parse + re-emit the code
    // graph before either persists the cursor (read-dedup hides it, but the
    // append-only tables grow). One task per bucket keeps ingestion idempotent.
    let schedule_ms: i64 = sqlx::query_scalar(
        "SELECT schedule_ms FROM connectors WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tenant.as_uuid())
    .bind(id)
    .fetch_optional(s.catalog.pool())
    .await
    .ok()
    .flatten()
    .unwrap_or(0);
    let key = if schedule_ms > 0 { (now_ms / schedule_ms) * schedule_ms } else { now_ms };
    let _ = catalog_sql::enqueue_tick(s.catalog.pool(), tenant, id, key).await;
    StatusCode::ACCEPTED
}
