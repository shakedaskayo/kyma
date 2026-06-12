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
use tracing::Instrument as _;

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
    let principal = if state.backend.enabled() {
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
            Err(AuthError::UnknownToken | AuthError::MissingToken) => {
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

        principal
    } else {
        // Auth-disabled mode: pretend an Admin principal in the default tenant
        // so downstream extractors see consistent extensions.
        super::backend::Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
            allowed_databases: None,
        }
    };

    let tenant = principal.tenant;
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| req.uri().path().to_string(), |m| m.as_str().to_string());
    let method = req.method().clone();
    let subject = principal.subject.clone();
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(tenant);

    // Skip self-tracing for noise: health checks, metrics scrapes, and the
    // long-lived live-tail socket (its span would only close on disconnect).
    if route == "/health" || route.starts_with("/metrics") || route.starts_with("/v1/explore/live")
    {
        return next.run(req).await;
    }

    let span = tracing::info_span!(
        target: "kyma_telemetry",
        "request",
        otel.name = %format!("{method} {route}"),
        http.method = %method,
        http.route = %route,
        kyma.tenant = %tenant,
        kyma.subject = tracing::field::Empty,
        http.status = tracing::field::Empty,
        // Declared Empty so the later `record()` lands — recording an
        // undeclared field is a silent no-op in `tracing`.
        otel.status_code = tracing::field::Empty,
    );
    if let Some(s) = &subject {
        span.record("kyma.subject", s.as_str());
    }
    let resp = next.run(req).instrument(span.clone()).await;
    span.record("http.status", resp.status().as_u16());
    span.record(
        "otel.status_code",
        if resp.status().is_server_error() { "ERROR" } else { "OK" },
    );
    resp
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
