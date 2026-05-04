//! Axum middleware that enforces a minimum [`Role`] using a pluggable
//! [`AuthBackend`]. On success, inserts the resolved [`super::backend::Principal`]
//! and its [`kyma_core::tenant::TenantId`] into the request extensions so
//! downstream handlers can scope work to that tenant.

use super::backend::{AuthBackend, AuthError, Role};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// Per-route state for [`require_role_middleware`].
#[derive(Clone)]
pub struct AuthLayerState {
    pub backend: Arc<dyn AuthBackend>,
    pub required: Role,
}

pub async fn require_role_middleware(
    State(state): State<AuthLayerState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !state.backend.enabled() {
        // Auth-disabled mode: pretend an Admin principal in the default tenant
        // so downstream extractors see consistent extensions.
        req.extensions_mut().insert(super::backend::Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
        });
        req.extensions_mut()
            .insert(kyma_core::tenant::DEFAULT_TENANT);
        return next.run(req).await;
    }

    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(token) = token else {
        return unauthorized("missing Authorization: Bearer <token>");
    };

    let principal = match state.backend.authenticate(token).await {
        Ok(p) => p,
        Err(AuthError::UnknownToken) | Err(AuthError::MissingToken) => {
            return unauthorized("unknown token");
        }
        Err(AuthError::Backend(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("auth backend: {e}"),
            )
                .into_response();
        }
    };

    if principal.role < state.required {
        return forbidden(&format!(
            "token role `{:?}` below required `{:?}`",
            principal.role, state.required
        ));
    }

    let tenant = principal.tenant;
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(tenant);
    next.run(req).await
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, r#"Bearer realm="kyma""#)],
        msg.to_owned(),
    )
        .into_response()
}

fn forbidden(msg: &str) -> Response {
    (StatusCode::FORBIDDEN, msg.to_owned()).into_response()
}
