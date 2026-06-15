//! Admin user-management HTTP surface — `/v1/admin/users`.
//!
//! Mounted behind `require_role_middleware` at [`crate::auth::Role::Admin`], so
//! every route requires an admin bearer token (true network RBAC — the token,
//! not raw Postgres access, is the credential). Passwords are argon2-hashed
//! here, server-side, via [`crate::auth::passwords`]; clients only ever send
//! plaintext over the connection (same as `/v1/auth/signup`).
//!
//! A **last-admin guard** prevents lockout: the final admin cannot be deleted
//! or demoted.
//!
//! # Routes
//! - `GET    /v1/admin/users`            — list users.
//! - `POST   /v1/admin/users`            — `{username, password, role}` → create.
//! - `PATCH  /v1/admin/users/:username`  — `{role?, password?}` → update.
//! - `DELETE /v1/admin/users/:username`  — delete.

use crate::auth::{passwords, Role};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, put},
    Json, Router,
};
use kyma_core::catalog::Catalog;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Minimum password length — matches the first-run signup rule.
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Clone)]
pub struct AdminState {
    pub catalog: Arc<dyn Catalog>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

fn err(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

/// Would removing or demoting this user leave the system with zero admins?
///
/// `new_role` is `None` for a delete and `Some(role)` for a role change. Pure
/// so it can be unit-tested without a database.
fn would_remove_last_admin(current_role: &str, new_role: Option<&str>, admin_count: usize) -> bool {
    let losing_an_admin = current_role == "admin" && new_role.map_or(true, |r| r != "admin");
    losing_an_admin && admin_count <= 1
}

/// Count admins in the default tenant (for the last-admin guard).
async fn admin_count(catalog: &dyn Catalog) -> Result<usize, Response> {
    let users = catalog
        .list_users()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()))?;
    Ok(users.iter().filter(|u| u.role == "admin").count())
}

async fn list_users_handler(State(state): State<AdminState>) -> Response {
    match state.catalog.list_users().await {
        Ok(users) => (StatusCode::OK, Json(users)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    }
}

async fn create_user_handler(
    State(state): State<AdminState>,
    Json(body): Json<CreateUserRequest>,
) -> Response {
    let username = body.username.trim();
    if username.is_empty() {
        return err(StatusCode::BAD_REQUEST, "invalid", "username is required");
    }
    if Role::parse(&body.role).is_none() {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "role must be one of: read, write, admin",
        );
    }
    if body.password.len() < MIN_PASSWORD_LEN {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid",
            "password must be at least 8 characters",
        );
    }
    match state.catalog.get_user_with_hash(username).await {
        Ok(Some(_)) => return err(StatusCode::CONFLICT, "exists", "user already exists"),
        Ok(None) => {}
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    }
    let phc = match passwords::hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "hash", &e),
    };
    match state.catalog.create_user(username, &phc, &body.role).await {
        Ok(user) => (StatusCode::CREATED, Json(user)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    }
}

async fn update_user_handler(
    State(state): State<AdminState>,
    Path(username): Path<String>,
    Json(body): Json<UpdateUserRequest>,
) -> Response {
    let (user, _hash) = match state.catalog.get_user_with_hash(&username).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", "user not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    };

    if body.role.is_none() && body.password.is_none() {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid",
            "nothing to update (provide role and/or password)",
        );
    }

    if let Some(role) = &body.role {
        if Role::parse(role).is_none() {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_role",
                "role must be one of: read, write, admin",
            );
        }
        if user.role == "admin" && role != "admin" {
            let count = match admin_count(state.catalog.as_ref()).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            if would_remove_last_admin(&user.role, Some(role), count) {
                return err(
                    StatusCode::CONFLICT,
                    "last_admin",
                    "cannot demote the only admin",
                );
            }
        }
        if let Err(e) = state.catalog.set_user_role(&username, role).await {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string());
        }
    }

    if let Some(password) = &body.password {
        if password.len() < MIN_PASSWORD_LEN {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid",
                "password must be at least 8 characters",
            );
        }
        let phc = match passwords::hash_password(password) {
            Ok(h) => h,
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "hash", &e),
        };
        if let Err(e) = state.catalog.set_user_password(&username, &phc).await {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string());
        }
    }

    match state.catalog.get_user_with_hash(&username).await {
        Ok(Some((user, _))) => (StatusCode::OK, Json(user)).into_response(),
        _ => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn delete_user_handler(
    State(state): State<AdminState>,
    Path(username): Path<String>,
) -> Response {
    let (user, _) = match state.catalog.get_user_with_hash(&username).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", "user not found"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    };
    if user.role == "admin" {
        let count = match admin_count(state.catalog.as_ref()).await {
            Ok(c) => c,
            Err(resp) => return resp,
        };
        if would_remove_last_admin(&user.role, None, count) {
            return err(
                StatusCode::CONFLICT,
                "last_admin",
                "cannot delete the only admin",
            );
        }
    }
    match state.catalog.delete_user(&username).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "not_found", "user not found"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    }
}

// --- per-tenant quotas (S2.6 ops hardening) ---

#[derive(Debug, Deserialize)]
pub struct UpsertQuotaRequest {
    /// Max in-flight `/v1/query` + `/v1/search` for the tenant (null = no override).
    #[serde(default)]
    pub max_query_concurrent: Option<u32>,
    /// Max in-flight agent runs for the tenant (null = no override).
    #[serde(default)]
    pub max_agent_concurrent: Option<u32>,
}

async fn list_quotas_handler(State(state): State<AdminState>) -> Response {
    match state.catalog.list_tenant_quotas().await {
        Ok(quotas) => {
            let body: Vec<_> = quotas
                .into_iter()
                .map(|q| {
                    json!({
                        "tenant": q.tenant.to_string(),
                        "max_query_concurrent": q.max_query_concurrent,
                        "max_agent_concurrent": q.max_agent_concurrent,
                        "updated_at": q.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!({ "quotas": body }))).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string()),
    }
}

async fn upsert_quota_handler(
    State(state): State<AdminState>,
    Path(tenant): Path<String>,
    Json(body): Json<UpsertQuotaRequest>,
) -> Response {
    let tenant_id = match tenant.parse::<kyma_core::tenant::TenantId>() {
        Ok(t) => t,
        Err(_) => {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_tenant",
                "tenant must be a UUID",
            )
        }
    };
    let quota = kyma_core::catalog::TenantQuota {
        tenant: tenant_id,
        max_query_concurrent: body.max_query_concurrent,
        max_agent_concurrent: body.max_agent_concurrent,
        updated_at: chrono::Utc::now(),
    };
    if let Err(e) = state.catalog.upsert_tenant_quota(&quota).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "catalog", &e.to_string());
    }
    // Refresh the in-RAM cache so the new limit applies immediately (rather than
    // waiting for the next periodic refresh).
    crate::quota_cache::refresh(&state.catalog).await;
    (
        StatusCode::OK,
        Json(json!({
            "tenant": tenant_id.to_string(),
            "max_query_concurrent": quota.max_query_concurrent,
            "max_agent_concurrent": quota.max_agent_concurrent,
        })),
    )
        .into_response()
}

/// Build the `/v1/admin/users` + `/v1/admin/tenant-quotas` router. Mount behind
/// `require_role_middleware` at [`Role::Admin`].
pub fn admin_users_router(catalog: Arc<dyn Catalog>) -> Router {
    Router::new()
        .route(
            "/v1/admin/users",
            get(list_users_handler).post(create_user_handler),
        )
        .route(
            "/v1/admin/users/:username",
            patch(update_user_handler).delete(delete_user_handler),
        )
        .route("/v1/admin/tenant-quotas", get(list_quotas_handler))
        .route("/v1/admin/tenant-quotas/:tenant", put(upsert_quota_handler))
        .with_state(AdminState { catalog })
}

#[cfg(test)]
mod tests {
    use super::would_remove_last_admin;

    #[test]
    fn delete_sole_admin_is_blocked() {
        // new_role = None => delete.
        assert!(would_remove_last_admin("admin", None, 1));
    }

    #[test]
    fn delete_admin_when_others_exist_is_allowed() {
        assert!(!would_remove_last_admin("admin", None, 2));
    }

    #[test]
    fn deleting_non_admin_is_always_allowed() {
        assert!(!would_remove_last_admin("write", None, 1));
        assert!(!would_remove_last_admin("read", None, 0));
    }

    #[test]
    fn demoting_sole_admin_is_blocked() {
        assert!(would_remove_last_admin("admin", Some("write"), 1));
        assert!(would_remove_last_admin("admin", Some("read"), 1));
    }

    #[test]
    fn demoting_admin_when_others_exist_is_allowed() {
        assert!(!would_remove_last_admin("admin", Some("write"), 3));
    }

    #[test]
    fn keeping_admin_role_is_never_a_demotion() {
        assert!(!would_remove_last_admin("admin", Some("admin"), 1));
    }
}
