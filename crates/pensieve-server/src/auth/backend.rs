//! Core auth abstractions: the [`AuthBackend`] trait, [`Principal`], [`Role`],
//! and [`AuthError`]. Backend implementations live alongside this module
//! (e.g. [`super::env_backend::EnvAuthBackend`]).

use async_trait::async_trait;
use pensieve_core::tenant::TenantId;

/// Minimum role needed to access a handler.
///
/// Ordered: `Read < Write < Admin`. A higher role implicitly grants the
/// access of any lower role.
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

/// The authenticated caller, resolved from a bearer token by an
/// [`AuthBackend`]. Inserted into the request extensions by
/// [`super::middleware::require_role_middleware`].
#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant: TenantId,
    pub role: Role,
    pub subject: Option<String>,
    /// Databases this principal may touch. `None` = unrestricted (the default
    /// for all pre-existing backends; only OIDC claims populate this).
    pub allowed_databases: Option<Vec<String>>,
    /// Memory realms this principal may read/write. `None` = unrestricted (the
    /// default for all pre-existing backends and the auth-disabled synthetic
    /// principal; only an OIDC `pensieve_realms` claim populates this). See
    /// [`super::scope::RealmScope`] / [`super::scope::intersect_realms`].
    pub allowed_realms: Option<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing Authorization: Bearer <token>")]
    MissingToken,
    #[error("unknown token")]
    UnknownToken,
    #[error("auth backend error: {0}")]
    Backend(String),
}

/// A pluggable bearer-token authenticator.
///
/// Implementations may resolve tokens from in-memory env config
/// (self-hosted) or a Postgres `api_tokens` table (cloud control plane).
#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    /// `false` ⇒ auth is disabled and the middleware should pass through.
    fn enabled(&self) -> bool;

    /// Resolve a presented token to a [`Principal`], or return an
    /// [`AuthError`].
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError>;
}
