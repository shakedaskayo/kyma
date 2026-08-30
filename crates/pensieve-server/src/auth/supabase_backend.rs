//! Supabase Auth backend: validates Supabase-issued JWTs (asymmetric via the
//! project JWKS, or HS256 via the legacy shared secret) and JIT-provisions
//! kyma users from them.
//!
//! # Authentication flow
//!
//! 1. JWT-shaped tokens (`eyJ…` with two dots) are validated as Supabase
//!    access tokens: signature (JWKS `kid` lookup with rate-limited refetch
//!    on rotation, or `KYMA_SUPABASE_JWT_SECRET` for HS256), `exp` (60s
//!    leeway), `aud`, and `iss = <url>/auth/v1`.
//! 2. The kyma role is resolved kyma-side: `KYMA_ADMIN_EMAILS` allowlist →
//!    Admin, else a parseable `app_metadata.role` claim, else
//!    `KYMA_SUPABASE_DEFAULT_ROLE` (Read). Supabase's own `role` claim is the
//!    Postgres RLS role (`authenticated`) and is deliberately ignored.
//! 3. Valid principals are JIT-upserted into the catalog `users` table keyed
//!    by `(auth_provider='supabase', external_id=sub)` — rate-limited per
//!    subject so steady-state requests skip the write.
//! 4. Everything else (opaque static keys, kyma session/API tokens) falls
//!    through to the wrapped [`SessionAuthBackend`], so CLI/MCP/CI clients
//!    keep working unchanged.

use super::backend::{AuthBackend, AuthError, Principal, Role};
use super::session_backend::SessionAuthBackend;
use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use kyma_core::catalog::Catalog;
use kyma_core::tenant::DEFAULT_TENANT;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Minimum interval between JWKS refetches triggered by unknown `kid`s, and
/// between JIT upserts for the same subject.
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Supabase project connection + authorization policy.
#[derive(Debug, Clone)]
pub struct SupabaseAuthConfig {
    /// Project base URL, e.g. `https://<ref>.supabase.co` (no trailing `/`).
    pub url: String,
    /// Legacy HS256 shared secret. When unset, validation uses the project
    /// JWKS at `<url>/auth/v1/.well-known/jwks.json` (preferred — fully
    /// automatable, supports key rotation).
    pub jwt_secret: Option<String>,
    /// Expected `aud` claim (Supabase user tokens use `authenticated`).
    pub audience: String,
    /// Emails granted `Role::Admin` (case-insensitive).
    pub admin_emails: Vec<String>,
    /// When non-empty, only emails under these domains may authenticate —
    /// guards JIT provisioning on Supabase projects with open signup.
    pub allowed_email_domains: Vec<String>,
    /// Role for users matching neither the admin allowlist nor a usable
    /// `app_metadata.role` claim.
    pub default_role: Role,
}

impl SupabaseAuthConfig {
    /// Load from `KYMA_SUPABASE_*` / `KYMA_ADMIN_EMAILS` /
    /// `KYMA_ALLOWED_EMAIL_DOMAINS` env vars. `None` when
    /// `KYMA_SUPABASE_URL` is unset.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("KYMA_SUPABASE_URL").ok()?;
        let csv = |key: &str| -> Vec<String> {
            std::env::var(key)
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        };
        Some(Self {
            url: url.trim_end_matches('/').to_string(),
            jwt_secret: std::env::var("KYMA_SUPABASE_JWT_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            audience: std::env::var("KYMA_SUPABASE_JWT_AUD")
                .unwrap_or_else(|_| "authenticated".to_string()),
            admin_emails: csv("KYMA_ADMIN_EMAILS"),
            allowed_email_domains: csv("KYMA_ALLOWED_EMAIL_DOMAINS"),
            default_role: std::env::var("KYMA_SUPABASE_DEFAULT_ROLE")
                .ok()
                .and_then(|v| Role::parse(&v))
                .unwrap_or(Role::Read),
        })
    }
}

/// The claims we read off a validated Supabase access token. `aud`/`iss`/
/// `exp` are enforced by [`Validation`], not deserialized here.
#[derive(serde::Deserialize)]
struct Claims {
    sub: String,
    email: Option<String>,
    #[serde(default)]
    app_metadata: serde_json::Value,
}

#[derive(Default)]
struct JwksCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

/// Bearer-token authenticator for Supabase-issued JWTs, falling back to the
/// wrapped [`SessionAuthBackend`] for opaque kyma tokens.
pub struct SupabaseAuthBackend {
    config: SupabaseAuthConfig,
    catalog: Arc<dyn Catalog>,
    inner: SessionAuthBackend,
    http: reqwest::Client,
    jwks: RwLock<JwksCache>,
    /// `sub → last JIT upsert` — skips the catalog write on hot paths.
    upserted: RwLock<HashMap<String, Instant>>,
}

/// Supabase access tokens are compact JWS: base64url(`{"alg"…`) is always
/// `eyJ…` and there are exactly two dot separators. kyma's own session/API
/// tokens are opaque CSPRNG strings and never match.
fn is_jwt_shaped(token: &str) -> bool {
    token.starts_with("eyJ") && token.bytes().filter(|b| *b == b'.').count() == 2
}

impl SupabaseAuthBackend {
    pub fn new(
        config: SupabaseAuthConfig,
        catalog: Arc<dyn Catalog>,
        inner: SessionAuthBackend,
    ) -> Self {
        Self {
            config,
            catalog,
            inner,
            http: reqwest::Client::new(),
            jwks: RwLock::new(JwksCache::default()),
            upserted: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve the decoding key for a token header: shared secret for HS*
    /// algorithms (when configured), else JWKS lookup by `kid`.
    async fn decoding_key(
        &self,
        header: &jsonwebtoken::Header,
    ) -> Result<DecodingKey, AuthError> {
        if let (Some(secret), Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512) =
            (&self.config.jwt_secret, header.alg)
        {
            return Ok(DecodingKey::from_secret(secret.as_bytes()));
        }
        let kid = header.kid.as_deref().ok_or(AuthError::UnknownToken)?;
        self.jwks_key(kid).await
    }

    /// JWKS `kid → key` lookup with a rate-limited refetch on miss, so key
    /// rotation is picked up without letting bad tokens hammer the endpoint.
    async fn jwks_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(key) = self.jwks.read().await.keys.get(kid) {
            return Ok(key.clone());
        }
        let mut cache = self.jwks.write().await;
        // Re-check under the write lock (another task may have refreshed).
        if let Some(key) = cache.keys.get(kid) {
            return Ok(key.clone());
        }
        let due = cache
            .fetched_at
            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL);
        if due {
            let url = format!("{}/auth/v1/.well-known/jwks.json", self.config.url);
            let set: jsonwebtoken::jwk::JwkSet = self
                .http
                .get(&url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| AuthError::Backend(format!("jwks fetch failed: {e}")))?
                .json()
                .await
                .map_err(|e| AuthError::Backend(format!("jwks parse failed: {e}")))?;
            let mut keys = HashMap::new();
            for jwk in &set.keys {
                if let (Some(id), Ok(key)) =
                    (jwk.common.key_id.clone(), DecodingKey::from_jwk(jwk))
                {
                    keys.insert(id, key);
                }
            }
            cache.keys = keys;
            cache.fetched_at = Some(Instant::now());
        }
        cache.keys.get(kid).cloned().ok_or(AuthError::UnknownToken)
    }

    /// Enforce the email-domain allowlist, then resolve the kyma role:
    /// admin allowlist → `app_metadata.role` → configured default.
    fn resolve_role(&self, email: Option<&str>, claims: &Claims) -> Result<Role, AuthError> {
        if !self.config.allowed_email_domains.is_empty() {
            let allowed = email
                .and_then(|e| e.rsplit_once('@'))
                .map(|(_, domain)| domain.to_lowercase())
                .is_some_and(|d| self.config.allowed_email_domains.contains(&d));
            if !allowed {
                return Err(AuthError::UnknownToken);
            }
        }
        if let Some(email) = email {
            if self
                .config
                .admin_emails
                .iter()
                .any(|a| a.eq_ignore_ascii_case(email))
            {
                return Ok(Role::Admin);
            }
        }
        if let Some(role) = claims
            .app_metadata
            .get("role")
            .and_then(|v| v.as_str())
            .and_then(Role::parse)
        {
            return Ok(role);
        }
        Ok(self.config.default_role)
    }

    async fn authenticate_jwt(&self, token: &str) -> Result<Principal, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::UnknownToken)?;
        let key = self.decoding_key(&header).await?;

        let mut validation = Validation::new(header.alg);
        validation.leeway = 60;
        validation.set_audience(&[&self.config.audience]);
        validation.set_issuer(&[format!("{}/auth/v1", self.config.url)]);
        let data =
            decode::<Claims>(token, &key, &validation).map_err(|_| AuthError::UnknownToken)?;
        let claims = data.claims;

        let role = self.resolve_role(claims.email.as_deref(), &claims)?;
        let username = claims.email.clone().unwrap_or_else(|| claims.sub.clone());

        // JIT provision/refresh, rate-limited per subject.
        let due = {
            let seen = self.upserted.read().await;
            seen.get(&claims.sub)
                .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL)
        };
        if due {
            let role_str = match role {
                Role::Admin => "admin",
                Role::Write => "write",
                Role::Read => "read",
            };
            self.catalog
                .upsert_external_user_in_tenant(
                    DEFAULT_TENANT,
                    "supabase",
                    &claims.sub,
                    &username,
                    role_str,
                )
                .await
                .map_err(|e| AuthError::Backend(e.to_string()))?;
            self.upserted
                .write()
                .await
                .insert(claims.sub.clone(), Instant::now());
        }

        Ok(Principal {
            tenant: DEFAULT_TENANT,
            role,
            subject: Some(username),
            // Supabase tokens are unrestricted; per-database scoping rides on
            // the generic-OIDC backend's claim mapping when that path is used.
            allowed_databases: None,
            allowed_realms: None,
        })
    }
}

#[async_trait]
impl AuthBackend for SupabaseAuthBackend {
    /// Configuring Supabase auth always turns auth on.
    fn enabled(&self) -> bool {
        true
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if is_jwt_shaped(token) {
            match self.authenticate_jwt(token).await {
                Ok(p) => return Ok(p),
                // A static/env token could in principle be JWT-shaped — give
                // the inner backend a chance before rejecting.
                Err(AuthError::UnknownToken) => {}
                Err(other) => return Err(other),
            }
        }
        self.inner.authenticate(token).await
    }
}
