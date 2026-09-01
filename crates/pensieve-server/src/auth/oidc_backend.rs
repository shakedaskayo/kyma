//! Generic OIDC bearer-token validation. Configured with one or more trusted
//! issuers; JWKS are fetched via OIDC discovery and cached, refreshing when
//! an unknown `kid` is seen (rate-limited). Tokens that do not look like JWTs
//! fall through to the wrapped inner backend, so native Pensieve tokens keep
//! working unchanged.

use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use jsonwebtoken::{
    decode, decode_header,
    jwk::{Jwk, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

/// Config parsed from env (see `from_env`).
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuers: Vec<String>,
    pub audience: String,
    pub role_claim: String,      // default "pensieve_role"
    pub subject_claim: String,   // default "sub"
    pub databases_claim: String, // default "pensieve_databases"
    pub realms_claim: String,    // default "pensieve_realms"
}

impl OidcConfig {
    /// `None` when `PENSIEVE_OIDC_ISSUERS` is unset/empty (OIDC disabled).
    pub fn from_env() -> Option<Self> {
        let issuers: Vec<String> = std::env::var("PENSIEVE_OIDC_ISSUERS")
            .ok()?
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if issuers.is_empty() {
            return None;
        }
        let audience = std::env::var("PENSIEVE_OIDC_AUDIENCE").unwrap_or_else(|_| "pensieve".into());
        Some(Self {
            issuers,
            audience,
            role_claim: std::env::var("PENSIEVE_OIDC_ROLE_CLAIM")
                .unwrap_or_else(|_| "pensieve_role".into()),
            subject_claim: std::env::var("PENSIEVE_OIDC_SUBJECT_CLAIM")
                .unwrap_or_else(|_| "sub".into()),
            databases_claim: std::env::var("PENSIEVE_OIDC_DATABASES_CLAIM")
                .unwrap_or_else(|_| "pensieve_databases".into()),
            realms_claim: std::env::var("PENSIEVE_OIDC_REALMS_CLAIM")
                .unwrap_or_else(|_| "pensieve_realms".into()),
        })
    }
}

struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
    /// Last time we attempted a refresh for an unknown kid, to rate-limit.
    last_refresh_attempt: Option<Instant>,
}

/// Validates JWTs from configured OIDC issuers; delegates non-JWT tokens to
/// `inner`. Auth is "enabled" if OIDC is configured (inner may add native tokens).
pub struct OidcAuthBackend {
    cfg: OidcConfig,
    inner: Arc<dyn AuthBackend>,
    http: reqwest::Client,
    /// issuer -> cached JWKS
    jwks: RwLock<HashMap<String, CachedJwks>>,
    /// Minimum time between forced JWKS refreshes for an unknown kid.
    /// Defaults to [`JWKS_MIN_REFRESH`]. Override via [`Self::with_min_refresh`]
    /// in tests to bypass the 30-second throttle.
    min_refresh: Duration,
}

const JWKS_TTL: Duration = Duration::from_secs(3600);
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30);

impl OidcAuthBackend {
    /// Create a new backend wrapping `inner` for non-JWT tokens.
    pub fn new(cfg: OidcConfig, inner: Arc<dyn AuthBackend>) -> Self {
        Self {
            cfg,
            inner,
            http: reqwest::Client::new(),
            jwks: RwLock::new(HashMap::new()),
            min_refresh: JWKS_MIN_REFRESH,
        }
    }

    /// Override the minimum time between forced JWKS refreshes.
    ///
    /// Intended for integration tests that need to exercise key rotation without
    /// waiting 30 seconds. Pass `Duration::ZERO` to disable throttling entirely.
    #[doc(hidden)]
    pub fn with_min_refresh(mut self, d: Duration) -> Self {
        self.min_refresh = d;
        self
    }

    /// Returns true if the string looks like a three-segment base64url JWT.
    fn looks_like_jwt(token: &str) -> bool {
        token.split('.').count() == 3
    }

    /// Returns an error if `url` uses http:// with a non-loopback host.
    fn require_https_for_non_loopback(url: &str) -> Result<(), AuthError> {
        // Only enforce on http:// URLs.
        if !url.starts_with("http://") {
            return Ok(());
        }
        // Extract the host portion from http://host[:port]/...
        let rest = &url["http://".len()..];
        let host = rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("");
        let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
        if loopback {
            return Ok(());
        }
        Err(AuthError::Backend(format!(
            "oidc issuer must use https (got http for non-loopback host: {host})"
        )))
    }

    /// Fetch and cache the JWKS for `issuer`.
    ///
    /// If `force` is true, a refresh is attempted even if the TTL has not
    /// expired, subject to `JWKS_MIN_REFRESH` rate-limiting.
    async fn jwks_for(
        &self,
        issuer: &str,
        force: bool,
    ) -> Result<JwkSet, AuthError> {
        // Enforce HTTPS for non-loopback issuers before any network call.
        Self::require_https_for_non_loopback(issuer)?;

        // --- fast path: read lock ---
        {
            let guard = self.jwks.read().await;
            if let Some(cached) = guard.get(issuer) {
                let expired = cached.fetched_at.elapsed() > JWKS_TTL;
                if !expired && !force {
                    return Ok(cached.set.clone());
                }
                // Rate-limit forced refreshes.
                if force {
                    if let Some(last) = cached.last_refresh_attempt {
                        if last.elapsed() < self.min_refresh {
                            // Return stale rather than hammering the IdP.
                            return Ok(cached.set.clone());
                        }
                    }
                }
            }
        }

        // --- slow path: fetch then write lock ---
        let discovery_url = format!("{}/.well-known/openid-configuration", issuer);
        let discovery: serde_json::Value = self
            .http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| AuthError::Backend(format!("OIDC discovery fetch failed: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Backend(format!("OIDC discovery parse failed: {e}")))?;

        let jwks_uri = discovery
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::Backend("OIDC discovery missing jwks_uri".into()))?
            .to_owned();

        // Also enforce HTTPS for the jwks_uri from the discovery document.
        Self::require_https_for_non_loopback(&jwks_uri)?;

        let set: JwkSet = self
            .http
            .get(&jwks_uri)
            .send()
            .await
            .map_err(|e| AuthError::Backend(format!("JWKS fetch failed: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Backend(format!("JWKS parse failed: {e}")))?;

        let mut guard = self.jwks.write().await;
        guard.insert(
            issuer.to_owned(),
            CachedJwks {
                set: set.clone(),
                fetched_at: Instant::now(),
                last_refresh_attempt: if force { Some(Instant::now()) } else { None },
            },
        );
        Ok(set)
    }

    /// Validate a JWT string and map claims to a [`Principal`].
    async fn validate_jwt(&self, token: &str) -> Result<Principal, AuthError> {
        // Step 1: decode the header to get `kid` and `alg`.
        let header = decode_header(token).map_err(|_| AuthError::UnknownToken)?;

        // Step 2: peek at the payload (second segment) to find `iss` without
        // full validation yet — we need `iss` to select the right JWKS.
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(AuthError::UnknownToken);
        }
        let payload_bytes =
            base64_url_decode(parts[1]).map_err(|_| AuthError::UnknownToken)?;
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).map_err(|_| AuthError::UnknownToken)?;

        let raw_iss = payload
            .get("iss")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::UnknownToken)?;

        // Normalize for the allowlist check (strip trailing slash), but keep
        // the raw value to use in set_issuer so jsonwebtoken's literal
        // comparison matches what the token actually carries.
        let normalized_iss = raw_iss.trim_end_matches('/').to_owned();
        if !self.cfg.issuers.iter().any(|i| *i == normalized_iss) {
            return Err(AuthError::UnknownToken);
        }
        // The JWKS cache is keyed by normalized issuer.
        let iss = normalized_iss;

        // Step 3: find the signing key in the JWKS, refreshing once if kid is
        // unknown.
        let kid = header.kid.as_deref().unwrap_or("");
        let jwk = self.find_jwk(&iss, kid, false).await?;

        // Step 4: build the decoding key and validate.
        let decoding_key = DecodingKey::from_jwk(&jwk)
            .map_err(|e| AuthError::Backend(format!("DecodingKey from JWK: {e}")))?;

        // Use the algorithm from the token header, but only allow asymmetric
        // algorithms (RSA / EC families). Symmetric algs (HS*) are rejected
        // because they require sharing the secret and are not used by OIDC IdPs.
        let alg = header.alg;
        let allowed = [
            Algorithm::RS256,
            Algorithm::RS384,
            Algorithm::RS512,
            Algorithm::ES256,
            Algorithm::ES384,
        ];
        if !allowed.contains(&alg) {
            return Err(AuthError::UnknownToken);
        }

        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.cfg.audience]);
        // Use the token's literal (un-normalized) iss so that an Auth0-style
        // trailing slash in the token doesn't cause a spurious mismatch.
        // The security boundary is the allowlist check above (normalized).
        validation.set_issuer(&[raw_iss]);
        // jsonwebtoken defaults validate_nbf=false; we enforce it.
        validation.validate_nbf = true;

        let token_data =
            decode::<serde_json::Value>(token, &decoding_key, &validation)
                .map_err(|_| AuthError::UnknownToken)?;

        let claims = token_data.claims;

        // Step 5: map claims -> Principal.
        let role = claims
            .get(&self.cfg.role_claim)
            .and_then(|v| v.as_str())
            .and_then(Role::parse)
            .unwrap_or(Role::Read);

        let subject = claims
            .get(&self.cfg.subject_claim)
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        let allowed_databases = claims
            .get(&self.cfg.databases_claim)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            });

        // Realms claim: absent → None (unrestricted). Present but malformed
        // (not a JSON array, or any element not a non-empty string) → REJECT
        // the token (fail closed). This deliberately differs from the
        // databases claim above, which silently maps a non-array to None — a
        // fail-open footgun we do not replicate for a security boundary.
        let allowed_realms = match claims.get(&self.cfg.realms_claim) {
            None => None,
            Some(serde_json::Value::Array(arr)) => {
                let mut out = Vec::with_capacity(arr.len());
                for x in arr {
                    match x.as_str().map(str::trim) {
                        Some(s) if !s.is_empty() => out.push(s.to_owned()),
                        _ => {
                            tracing::warn!(
                                claim = %self.cfg.realms_claim,
                                "rejecting token: realms claim element is not a non-empty string"
                            );
                            return Err(AuthError::UnknownToken);
                        }
                    }
                }
                Some(out)
            }
            Some(_) => {
                tracing::warn!(
                    claim = %self.cfg.realms_claim,
                    "rejecting token: realms claim is not a JSON array"
                );
                return Err(AuthError::UnknownToken);
            }
        };

        // NOTE: multi-tenant claim mapping (e.g. a `pensieve_tenant` claim that
        // picks the TenantId) is a cloud-track follow-up. Self-hosted always
        // uses DEFAULT_TENANT.
        Ok(Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role,
            subject,
            allowed_databases,
            allowed_realms,
        })
    }

    /// Find the JWK by kid, optionally forcing a cache refresh on first miss.
    async fn find_jwk(
        &self,
        issuer: &str,
        kid: &str,
        already_refreshed: bool,
    ) -> Result<Jwk, AuthError> {
        let set = self.jwks_for(issuer, already_refreshed).await?;
        let found = set.keys.iter().find(|k| {
            k.common
                .key_id
                .as_deref()
                .map_or(kid.is_empty(), |id| id == kid)
        });
        match found {
            Some(k) => Ok(k.clone()),
            None if !already_refreshed => {
                // Unknown kid — try a forced refresh once.
                let set2 = self.jwks_for(issuer, true).await?;
                set2.keys
                    .into_iter()
                    .find(|k| {
                        k.common
                            .key_id
                            .as_deref()
                            .map_or(kid.is_empty(), |id| id == kid)
                    })
                    .ok_or(AuthError::UnknownToken)
            }
            None => Err(AuthError::UnknownToken),
        }
    }

    /// Test helper: inject a pre-built JwkSet into the cache, bypassing network.
    #[cfg(test)]
    pub(crate) async fn inject_jwks(&self, issuer: &str, set: JwkSet) {
        let mut guard = self.jwks.write().await;
        guard.insert(
            issuer.to_owned(),
            CachedJwks {
                set,
                fetched_at: Instant::now(),
                last_refresh_attempt: None,
            },
        );
    }
}

/// Decode a base64url segment (no padding required).
fn base64_url_decode(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.decode(input)
}

#[async_trait]
impl AuthBackend for OidcAuthBackend {
    fn enabled(&self) -> bool {
        true
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if Self::looks_like_jwt(token) {
            match self.validate_jwt(token).await {
                Ok(p) => return Ok(p),
                Err(e) => {
                    // Only fall through to inner if inner is enabled.
                    if self.inner.enabled() {
                        return self.inner.authenticate(token).await;
                    }
                    return Err(e);
                }
            }
        }
        // Non-JWT: delegate to inner.
        self.inner.authenticate(token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rsa::{
        pkcs1::EncodeRsaPrivateKey,
        traits::PublicKeyParts as _,
        RsaPrivateKey,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    // ---------------------------------------------------------------------------
    // Test key material (generated once per test run)
    // ---------------------------------------------------------------------------

    fn make_rsa_key() -> RsaPrivateKey {
        let mut rng = rand::thread_rng();
        RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key")
    }

    /// Build a JwkSet from an RSA private key's public components.
    fn rsa_jwk_set(priv_key: &RsaPrivateKey, kid: &str) -> JwkSet {
        let pub_key = priv_key.to_public_key();
        let n = pub_key.n().to_bytes_be();
        let e = pub_key.e().to_bytes_be();

        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let n_b64 = URL_SAFE_NO_PAD.encode(&n);
        let e_b64 = URL_SAFE_NO_PAD.encode(&e);

        let jwk_json = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "kid": kid,
                "alg": "RS256",
                "n": n_b64,
                "e": e_b64,
            }]
        });
        serde_json::from_value(jwk_json).expect("build JwkSet")
    }

    fn encoding_key(priv_key: &RsaPrivateKey) -> EncodingKey {
        let pem = priv_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .expect("to PEM");
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("EncodingKey")
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn make_cfg(issuer: &str) -> OidcConfig {
        OidcConfig {
            issuers: vec![issuer.to_owned()],
            audience: "pensieve".into(),
            role_claim: "pensieve_role".into(),
            subject_claim: "sub".into(),
            databases_claim: "pensieve_databases".into(),
            realms_claim: "pensieve_realms".into(),
        }
    }

    /// Stub inner backend that recognises exactly one opaque token.
    struct StubInner {
        enabled: bool,
        token: String,
        principal: Principal,
    }

    #[async_trait]
    impl AuthBackend for StubInner {
        fn enabled(&self) -> bool {
            self.enabled
        }
        async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
            if token == self.token {
                Ok(self.principal.clone())
            } else {
                Err(AuthError::UnknownToken)
            }
        }
    }

    fn stub_inner(enabled: bool) -> Arc<dyn AuthBackend> {
        Arc::new(StubInner {
            enabled,
            token: "opaque-1".into(),
            principal: Principal {
                tenant: pensieve_core::tenant::DEFAULT_TENANT,
                role: Role::Read,
                subject: Some("stub-subject".into()),
                allowed_databases: None,
                allowed_realms: None,
            },
        })
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn validates_rs256_jwt_and_maps_claims() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "user-1",
            "pensieve_role": "write",
            "pensieve_databases": ["prod"],
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let p = backend.authenticate(&token).await.unwrap();

        assert_eq!(p.role, Role::Write);
        assert_eq!(p.subject, Some("user-1".into()));
        assert_eq!(p.allowed_databases, Some(vec!["prod".into()]));
        assert_eq!(p.tenant, pensieve_core::tenant::DEFAULT_TENANT);
    }

    #[tokio::test]
    async fn missing_role_claim_defaults_to_read() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "user-2",
            // no pensieve_role
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let p = backend.authenticate(&token).await.unwrap();
        assert_eq!(p.role, Role::Read);
    }

    #[tokio::test]
    async fn missing_databases_claim_means_unrestricted() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "user-3",
            // no pensieve_databases
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let p = backend.authenticate(&token).await.unwrap();
        assert!(p.allowed_databases.is_none(), "should be unrestricted");
    }

    // -----------------------------------------------------------------------
    // Fix 1: nbf validation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_not_yet_valid() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 7200,
            "nbf": now_secs() + 3600, // not valid yet
            "sub": "future-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::UnknownToken),
            "expected UnknownToken for nbf in future, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Fix 2: Issuer trailing-slash interop
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn accepts_issuer_with_trailing_slash_in_token() {
        // Config has no trailing slash; token iss has trailing slash.
        let priv_key = make_rsa_key();
        let issuer_config = "https://issuer.test";
        let issuer_token = "https://issuer.test/"; // trailing slash in token
        let cfg = make_cfg(issuer_config);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        // Cache under the normalized (no slash) key because from_env / make_cfg normalizes.
        backend
            .inject_jwks(issuer_config, rsa_jwk_set(&priv_key, "key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-1".into());

        let claims = serde_json::json!({
            "iss": issuer_token,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "slash-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let p = backend.authenticate(&token).await.unwrap();
        assert_eq!(p.subject, Some("slash-user".into()));
    }

    #[tokio::test]
    async fn accepts_trailing_slash_in_config() {
        // Regression guard: even if config had trailing slash (from_env strips it),
        // token without slash must still be accepted.
        let priv_key = make_rsa_key();
        // Build OidcConfig manually with already-stripped issuer (mirrors from_env behavior).
        let issuer_config = "https://issuer.test";
        let issuer_token = "https://issuer.test"; // no trailing slash
        let cfg = make_cfg(issuer_config);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer_config, rsa_jwk_set(&priv_key, "key-2"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-2".into());

        let claims = serde_json::json!({
            "iss": issuer_token,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "noslash-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let p = backend.authenticate(&token).await.unwrap();
        assert_eq!(p.subject, Some("noslash-user".into()));
    }

    // -----------------------------------------------------------------------
    // Fix 3: HTTPS enforcement for non-loopback JWKS
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_http_issuer_for_non_loopback() {
        // http:// with a non-loopback host must fail before any network call.
        let priv_key = make_rsa_key();
        let issuer = "http://idp.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        // Do NOT inject JWKS — the scheme check should fire before any fetch.
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "http-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::Backend(ref msg) if msg.contains("https")),
            "expected Backend error mentioning https, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Fix 4: Bad-signature test
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_bad_signature() {
        // Sign token with key_a but serve JwkSet with key_b under the same kid.
        let key_a = make_rsa_key();
        let key_b = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        // Inject key_b's public key under kid "shared-kid"
        backend
            .inject_jwks(issuer, rsa_jwk_set(&key_b, "shared-kid"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("shared-kid".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "bad-sig-user",
        });

        // Sign with key_a — JWKS has key_b → signature mismatch
        let token = encode(&header, &claims, &encoding_key(&key_a)).unwrap();
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::UnknownToken),
            "expected UnknownToken for bad signature, got {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_expired() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() - 7200,  // expired well past any leeway
            "sub": "user-expired",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::UnknownToken),
            "expected UnknownToken, got {err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_wrong_audience() {
        let priv_key = make_rsa_key();
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "wrong-audience",
            "exp": now_secs() + 3600,
            "sub": "user-aud",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }

    #[tokio::test]
    async fn rejects_wrong_issuer() {
        let priv_key = make_rsa_key();
        // Configure backend to trust "https://trusted.example.com" but token
        // carries "https://evil.example.com".
        let trusted = "https://trusted.example.com";
        let evil = "https://evil.example.com";
        let cfg = make_cfg(trusted);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        // Only inject JWKS for trusted — evil never gets to network lookup.
        backend
            .inject_jwks(trusted, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": evil,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "evil-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        // The iss check happens before any JWKS/network call.
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }

    #[tokio::test]
    async fn rejects_unknown_kid() {
        // Token signed with kid "other" while JWKS only has "test-key-1".
        // The force-refresh path will attempt network to an unreachable address,
        // which will map to AuthError::Backend or AuthError::UnknownToken.
        // We assert only that the result is an error.
        let priv_key = make_rsa_key();
        // Use 127.0.0.1:1 — TCP connect will fail immediately (no listener).
        let issuer = "http://127.0.0.1:1";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));
        // Inject a JWK with kid "test-key-1" only.
        backend
            .inject_jwks(issuer, rsa_jwk_set(&priv_key, "test-key-1"))
            .await;

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("other".into()); // not in JWKS

        let claims = serde_json::json!({
            "iss": issuer,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "some-user",
        });

        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();
        // The force-refresh will try http://127.0.0.1:1/.well-known/... which
        // fails fast with a connection error → Backend error, or the stale
        // cache is returned and kid is still missing → UnknownToken.
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::UnknownToken | AuthError::Backend(_)),
            "expected error for unknown kid, got {err:?}"
        );
    }

    #[tokio::test]
    async fn non_jwt_token_falls_through_to_inner() {
        let issuer = "https://test.example.com";
        let cfg = make_cfg(issuer);
        let backend = OidcAuthBackend::new(cfg, stub_inner(true));

        // "opaque-1" has no dots → not a JWT → delegated to inner
        let p = backend.authenticate("opaque-1").await.unwrap();
        assert_eq!(p.subject, Some("stub-subject".into()));
    }

    #[tokio::test]
    async fn jwt_failing_oidc_falls_through_when_inner_enabled() {
        // A valid-looking JWT from an untrusted issuer; inner is enabled.
        // The OIDC validation returns UnknownToken; we fall through to inner,
        // which also rejects it (unknown token).
        let priv_key = make_rsa_key();
        let evil = "https://evil.example.com";
        let trusted = "https://trusted.example.com";
        let cfg = make_cfg(trusted);
        let backend = OidcAuthBackend::new(cfg, stub_inner(true));

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": evil,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "evil",
        });
        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();

        // JWT with wrong issuer → falls through to inner; inner rejects it.
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }

    #[tokio::test]
    async fn jwt_failing_oidc_errors_when_inner_disabled() {
        let priv_key = make_rsa_key();
        let evil = "https://evil.example.com";
        let trusted = "https://trusted.example.com";
        let cfg = make_cfg(trusted);
        let backend = OidcAuthBackend::new(cfg, stub_inner(false));

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".into());

        let claims = serde_json::json!({
            "iss": evil,
            "aud": "pensieve",
            "exp": now_secs() + 3600,
            "sub": "evil",
        });
        let token = encode(&header, &claims, &encoding_key(&priv_key)).unwrap();

        // JWT with wrong issuer, inner disabled → direct error (no fallback).
        let err = backend.authenticate(&token).await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }
}
