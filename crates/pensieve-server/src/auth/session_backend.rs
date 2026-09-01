//! Session-based auth backend: validates bearer tokens against the
//! `api_tokens` catalog table, with a fallback to [`EnvAuthBackend`] so that
//! static `PENSIEVE_AUTH_TOKENS` API keys keep working alongside session tokens
//! issued by `POST /v1/auth/login`.
//!
//! # Authentication flow
//!
//! 1. Try `EnvAuthBackend::authenticate(token)` — fast in-memory lookup.
//!    If the token is a known static key, return the resolved principal.
//! 2. On [`AuthError::UnknownToken`], hash the presented token with SHA-256
//!    and call `catalog.lookup_api_token(&hash)`. On success, map the
//!    [`TokenPrincipal`] to a [`Principal`].
//! 3. Any other error from env_backend is propagated as-is.

use super::backend::{AuthBackend, AuthError, Principal, Role};
use super::env_backend::EnvAuthBackend;
use async_trait::async_trait;
use pensieve_core::catalog::Catalog;
use std::sync::Arc;

/// SHA-256 hash of a bearer token string.
///
/// Used for both session token storage and lookup. High-entropy tokens
/// (≥128 bits of CSPRNG entropy) are safe to store as raw SHA-256 without
/// a salt. Do NOT use this for password hashing — that needs argon2.
pub fn hash_token(token: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().to_vec()
}

/// Bearer-token authenticator that chains the env static tokens with the
/// Postgres-backed `api_tokens` session token store.
pub struct SessionAuthBackend {
    catalog: Arc<dyn Catalog>,
    env: EnvAuthBackend,
    /// Cached at construction time: `true` if there is at least one user row.
    /// Used by `enabled()` so callers know auth is live even when
    /// `PENSIEVE_AUTH_TOKENS` is empty.
    users_exist: bool,
}

impl SessionAuthBackend {
    /// Create a new backend.
    ///
    /// * `catalog` – used for [`Catalog::lookup_api_token`].
    /// * `env` – static token fallback (from `PENSIEVE_AUTH_TOKENS`).
    /// * `users_exist` – pass `count_users() > 0`; determines `enabled()`.
    pub fn new(catalog: Arc<dyn Catalog>, env: EnvAuthBackend, users_exist: bool) -> Self {
        Self {
            catalog,
            env,
            users_exist,
        }
    }
}

#[async_trait]
impl AuthBackend for SessionAuthBackend {
    fn enabled(&self) -> bool {
        self.env.enabled() || self.users_exist
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        // 1. Fast path: env static tokens.
        match self.env.authenticate(token).await {
            Ok(p) => return Ok(p),
            Err(AuthError::UnknownToken) => {}
            Err(other) => return Err(other),
        }

        // 2. Slow path: SHA-256 lookup in api_tokens.
        let hash = hash_token(token);
        let result = self
            .catalog
            .lookup_api_token(&hash)
            .await
            .map_err(|e| AuthError::Backend(e.to_string()))?;

        match result {
            Some(tp) => Ok(Principal {
                tenant: tp.tenant,
                role: Role::parse(&tp.role).unwrap_or(Role::Read),
                subject: tp.subject,
                allowed_databases: None,
                allowed_realms: None,
            }),
            None => Err(AuthError::UnknownToken),
        }
    }
}
