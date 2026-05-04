//! Self-hosted auth backend: bearer tokens loaded from the
//! `KYMA_AUTH_TOKENS` env var. Empty/unset ⇒ auth disabled.
//!
//! Format: comma-separated list of `token:role` pairs, e.g.
//! ```text
//! KYMA_AUTH_TOKENS=alice-tok:admin,bob-tok:write,reader-tok:read
//! ```

use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use kyma_core::tenant::DEFAULT_TENANT;
use std::collections::HashMap;
use std::sync::Arc;

/// Parsed env-var auth configuration. Cheap to clone (Arc inside).
#[derive(Clone, Default)]
pub struct EnvAuthBackend {
    inner: Arc<EnvInner>,
}

#[derive(Default)]
struct EnvInner {
    /// Empty map ⇒ auth disabled.
    tokens: HashMap<String, Role>,
}

impl EnvAuthBackend {
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
            let Some(role) = Role::parse(role) else {
                continue;
            };
            if !tok.is_empty() {
                tokens.insert(tok.to_owned(), role);
            }
        }
        Self {
            inner: Arc::new(EnvInner { tokens }),
        }
    }
}

#[async_trait]
impl AuthBackend for EnvAuthBackend {
    fn enabled(&self) -> bool {
        !self.inner.tokens.is_empty()
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let role = self
            .inner
            .tokens
            .get(token)
            .copied()
            .ok_or(AuthError::UnknownToken)?;
        Ok(Principal {
            tenant: DEFAULT_TENANT,
            role,
            subject: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_token_role_pairs() {
        let b = EnvAuthBackend::from_str("alice:admin, bob:write, carol:read");
        assert!(b.enabled());
        assert_eq!(b.authenticate("alice").await.unwrap().role, Role::Admin);
        assert_eq!(b.authenticate("bob").await.unwrap().role, Role::Write);
        assert_eq!(b.authenticate("carol").await.unwrap().role, Role::Read);
    }

    #[tokio::test]
    async fn empty_disables_auth() {
        let b = EnvAuthBackend::from_str("");
        assert!(!b.enabled());
    }

    #[tokio::test]
    async fn principal_is_default_tenant() {
        let b = EnvAuthBackend::from_str("alice:admin");
        let p = b.authenticate("alice").await.unwrap();
        assert_eq!(p.tenant, kyma_core::tenant::DEFAULT_TENANT);
    }

    #[tokio::test]
    async fn unknown_token_rejected() {
        let b = EnvAuthBackend::from_str("alice:admin");
        let err = b.authenticate("eve").await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }
}
