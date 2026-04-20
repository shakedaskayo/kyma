//! Bearer-token auth stub.
//!
//! # Config
//!
//! Tokens + roles are loaded from `KYMA_AUTH_TOKENS` as a comma-separated
//! list of `token:role` pairs, e.g.:
//!
//! ```text
//! KYMA_AUTH_TOKENS=alice-tok:admin,bob-tok:write,reader-tok:read
//! ```
//!
//! If the env var is empty or unset, **auth is disabled** and every
//! request is allowed. This preserves backwards compatibility for local
//! dev stacks and existing E2E tests.
//!
//! # Role hierarchy
//!
//! `admin` ⊃ `write` ⊃ `read`. A role automatically grants anything beneath
//! it. Endpoints declare the minimum role required.
//!
//! # What's not here (deferred)
//!
//! - Token TTLs (rotation via a separate catalog table lands with real auth).
//! - Per-token scope restrictions (specific tables / databases).
//! - OIDC / JWT — this is a stub.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::sync::Arc;

/// Minimum role needed to access a handler.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum Role {
    Read = 0,
    Write = 1,
    Admin = 2,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "read" => Some(Role::Read),
            "write" => Some(Role::Write),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// Parsed auth configuration. Cheap to clone (Arc inside).
#[derive(Clone, Default)]
pub struct AuthConfig {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    /// Empty map ⇒ auth disabled.
    tokens: HashMap<String, Role>,
}

impl Default for AuthInner {
    fn default() -> Self {
        Self {
            tokens: HashMap::new(),
        }
    }
}

impl AuthConfig {
    /// Load configuration from the `KYMA_AUTH_TOKENS` env var.
    pub fn from_env() -> Self {
        let raw = std::env::var("KYMA_AUTH_TOKENS").unwrap_or_default();
        Self::from_str(&raw)
    }

    /// Parse a `token:role,token:role,...` specification.
    pub fn from_str(raw: &str) -> Self {
        let mut tokens = HashMap::new();
        for pair in raw.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let Some((tok, role)) = pair.split_once(':') else {
                continue;
            };
            let tok = tok.trim();
            let Some(role) = Role::parse(role) else { continue };
            if !tok.is_empty() {
                tokens.insert(tok.to_owned(), role);
            }
        }
        Self {
            inner: Arc::new(AuthInner { tokens }),
        }
    }

    pub fn enabled(&self) -> bool {
        !self.inner.tokens.is_empty()
    }

    fn lookup(&self, token: &str) -> Option<Role> {
        self.inner.tokens.get(token).copied()
    }
}

/// Middleware constructor: `require_role(Role::Write)` returns a layer that
/// rejects requests without a valid token whose role ≥ the required role.
pub async fn require_role_middleware(
    State((config, required)): State<(AuthConfig, Role)>,
    req: Request,
    next: Next,
) -> Response {
    // Auth-disabled mode: pass through.
    if !config.enabled() {
        return next.run(req).await;
    }

    // Extract token.
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);

    let Some(token) = token else {
        return unauthorized("missing Authorization: Bearer <token>");
    };

    let Some(role) = config.lookup(token) else {
        return unauthorized("unknown token");
    };
    if role < required {
        return forbidden(&format!(
            "token role `{role:?}` below required `{required:?}`"
        ));
    }

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
