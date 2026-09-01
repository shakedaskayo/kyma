//! End-to-end OIDC integration tests with a real in-process mock issuer.
//!
//! Each test spins up an ephemeral axum HTTP server that serves:
//!   GET /.well-known/openid-configuration
//!   GET /jwks
//!
//! Tokens are minted with RSA keypairs (same helper approach as the
//! oidc_backend.rs unit tests). No env vars are set — OidcConfig is built
//! directly to avoid races between parallel tests.

use async_trait::async_trait;
use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use jsonwebtoken::{
    encode,
    jwk::JwkSet,
    Algorithm, EncodingKey, Header,
};
use pensieve_server::auth::{AuthBackend, AuthError, OidcAuthBackend, OidcConfig, Principal, Role};
use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts as _, RsaPrivateKey};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Key material helpers (mirrors the approach in oidc_backend.rs unit tests)
// ---------------------------------------------------------------------------

fn make_rsa_key() -> RsaPrivateKey {
    let mut rng = rand::thread_rng();
    RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key")
}

fn rsa_jwk_set(priv_key: &RsaPrivateKey, kid: &str) -> JwkSet {
    let pub_key = priv_key.to_public_key();
    let n = pub_key.n().to_bytes_be();
    let e = pub_key.e().to_bytes_be();

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

// ---------------------------------------------------------------------------
// Mock OIDC issuer (axum server on 127.0.0.1:0)
// ---------------------------------------------------------------------------

/// Shared state for the mock issuer: the JWKS can be swapped at runtime
/// (for the key-rotation test).
type SharedJwks = Arc<RwLock<JwkSet>>;

/// Mock issuer state.
#[derive(Clone)]
struct MockIssuerState {
    issuer_url: String,
    jwks: SharedJwks,
}

async fn discovery_handler(
    State(state): State<MockIssuerState>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "issuer": state.issuer_url,
        "jwks_uri": format!("{}/jwks", state.issuer_url),
    }))
}

async fn jwks_handler(State(state): State<MockIssuerState>) -> Json<JwkSet> {
    let set = state.jwks.read().await.clone();
    Json(set)
}

/// A running mock OIDC issuer.
struct MockIssuer {
    pub issuer_url: String,
    pub jwks: SharedJwks,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl MockIssuer {
    /// Spawn a mock issuer serving `initial_jwks`.
    async fn start(initial_jwks: JwkSet) -> Self {
        let shared = Arc::new(RwLock::new(initial_jwks));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock issuer");
        let port = listener.local_addr().unwrap().port();
        let issuer_url = format!("http://127.0.0.1:{port}");

        let state = MockIssuerState {
            issuer_url: issuer_url.clone(),
            jwks: shared.clone(),
        };

        let router = Router::new()
            .route("/.well-known/openid-configuration", get(discovery_handler))
            .route("/jwks", get(jwks_handler))
            .with_state(state);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await
                .ok();
        });

        Self {
            issuer_url,
            jwks: shared,
            _shutdown: tx,
        }
    }

    /// Replace the served JWKS (for rotation tests).
    async fn swap_jwks(&self, new_set: JwkSet) {
        *self.jwks.write().await = new_set;
    }
}

// ---------------------------------------------------------------------------
// Stub inner backend (disabled — rejects everything)
// ---------------------------------------------------------------------------

struct DisabledInner;

#[async_trait]
impl AuthBackend for DisabledInner {
    fn enabled(&self) -> bool {
        false
    }
    async fn authenticate(&self, _token: &str) -> Result<Principal, AuthError> {
        Err(AuthError::UnknownToken)
    }
}

fn disabled_inner() -> Arc<dyn AuthBackend> {
    Arc::new(DisabledInner)
}

// ---------------------------------------------------------------------------
// Stub inner backend that recognises "opaque-xyz" (enabled — for chaining test)
// ---------------------------------------------------------------------------

struct OpaqueInner;

#[async_trait]
impl AuthBackend for OpaqueInner {
    fn enabled(&self) -> bool {
        true
    }
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if token == "opaque-xyz" {
            Ok(Principal {
                tenant: pensieve_core::tenant::DEFAULT_TENANT,
                role: Role::Read,
                subject: Some("opaque-subject".into()),
                allowed_databases: None,
                allowed_realms: None,
            })
        } else {
            Err(AuthError::UnknownToken)
        }
    }
}

fn opaque_inner() -> Arc<dyn AuthBackend> {
    Arc::new(OpaqueInner)
}

// ---------------------------------------------------------------------------
// Helper: make a standard OidcConfig pointing at the mock issuer
// ---------------------------------------------------------------------------

fn make_cfg(issuer_url: &str) -> OidcConfig {
    OidcConfig {
        issuers: vec![issuer_url.to_owned()],
        audience: "pensieve".into(),
        role_claim: "pensieve_role".into(),
        subject_claim: "sub".into(),
        databases_claim: "pensieve_databases".into(),
        realms_claim: "pensieve_realms".into(),
    }
}

// ---------------------------------------------------------------------------
// Test 1: full_flow_validates_token_and_maps_claims
//
// Exercises REAL discovery + JWKS fetch over loopback HTTP.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_flow_validates_token_and_maps_claims() {
    let key1 = make_rsa_key();
    let jwks1 = rsa_jwk_set(&key1, "it-key-1");

    let issuer = MockIssuer::start(jwks1).await;

    let cfg = make_cfg(&issuer.issuer_url);
    let backend = OidcAuthBackend::new(cfg, disabled_inner());

    let mut hdr = Header::new(Algorithm::RS256);
    hdr.kid = Some("it-key-1".into());

    let claims = serde_json::json!({
        "iss": issuer.issuer_url,
        "aud": "pensieve",
        "sub": "u1",
        "exp": now_secs() + 3600,
        "pensieve_role": "write",
        "pensieve_databases": ["prod"],
    });

    let token = encode(&hdr, &claims, &encoding_key(&key1)).unwrap();
    let p = backend.authenticate(&token).await.expect("authenticate should succeed");

    assert_eq!(p.role, Role::Write, "role should be Write");
    assert_eq!(p.subject, Some("u1".into()), "subject should be u1");
    assert_eq!(
        p.allowed_databases,
        Some(vec!["prod".into()]),
        "allowed_databases should be [prod]"
    );
    assert_eq!(p.tenant, pensieve_core::tenant::DEFAULT_TENANT);
}

// ---------------------------------------------------------------------------
// Test 2: key_rotation_refreshes_jwks_on_unknown_kid
//
// Solve the 30-second throttle: use `with_min_refresh(Duration::ZERO)` to
// disable force-refresh rate limiting entirely (public builder, doc-hidden).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn key_rotation_refreshes_jwks_on_unknown_kid() {
    let key1 = make_rsa_key();
    let jwks1 = rsa_jwk_set(&key1, "it-key-1");

    let issuer = MockIssuer::start(jwks1).await;

    let cfg = make_cfg(&issuer.issuer_url);
    // Disable the 30-second throttle so the forced refresh works immediately.
    let backend = OidcAuthBackend::new(cfg, disabled_inner())
        .with_min_refresh(Duration::ZERO);

    // --- Warm the cache with key1 ---
    let mut hdr1 = Header::new(Algorithm::RS256);
    hdr1.kid = Some("it-key-1".into());

    let claims1 = serde_json::json!({
        "iss": issuer.issuer_url,
        "aud": "pensieve",
        "sub": "warm",
        "exp": now_secs() + 3600,
    });

    let token1 = encode(&hdr1, &claims1, &encoding_key(&key1)).unwrap();
    backend.authenticate(&token1).await.expect("warm-up with key1");

    // --- Rotate to key2 on the mock server ---
    let key2 = make_rsa_key();
    let jwks2 = rsa_jwk_set(&key2, "it-key-2");
    issuer.swap_jwks(jwks2).await;

    // --- Mint a token signed by key2 ---
    let mut hdr2 = Header::new(Algorithm::RS256);
    hdr2.kid = Some("it-key-2".into());

    let claims2 = serde_json::json!({
        "iss": issuer.issuer_url,
        "aud": "pensieve",
        "sub": "rotated",
        "exp": now_secs() + 3600,
    });

    let token2 = encode(&hdr2, &claims2, &encoding_key(&key2)).unwrap();

    // The backend must force-refresh on the unknown kid "it-key-2" and succeed.
    let p = backend.authenticate(&token2).await
        .expect("should succeed after force-refresh with key2");

    assert_eq!(p.subject, Some("rotated".into()));
}

// ---------------------------------------------------------------------------
// Test 3: opaque_token_chains_to_inner_backend
//
// A non-JWT token bypasses OIDC and is delegated to the inner backend.
// Inner is enabled and recognises "opaque-xyz".
// ---------------------------------------------------------------------------

#[tokio::test]
async fn opaque_token_chains_to_inner_backend() {
    // We still need a valid OIDC config (any issuer) — the backend is enabled.
    // We use a dummy URL; the opaque path never fetches it.
    let cfg = OidcConfig {
        issuers: vec!["http://127.0.0.1:1".into()], // unreachable — never hit
        audience: "pensieve".into(),
        role_claim: "pensieve_role".into(),
        subject_claim: "sub".into(),
        databases_claim: "pensieve_databases".into(),
        realms_claim: "pensieve_realms".into(),
    };
    let backend = OidcAuthBackend::new(cfg, opaque_inner());

    let p = backend.authenticate("opaque-xyz").await
        .expect("opaque token should be accepted by inner backend");

    assert_eq!(p.subject, Some("opaque-subject".into()));
    assert_eq!(p.role, Role::Read);
}

// ---------------------------------------------------------------------------
// Test 4: scoped_claims_flow_through_real_http
//
// Token with pensieve_databases ["a", "b"] → allowed_databases Some(["a","b"]).
// Exercises REAL discovery + JWKS fetch (same as test 1, different claims).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scoped_claims_flow_through_real_http() {
    let key = make_rsa_key();
    let jwks = rsa_jwk_set(&key, "it-key-1");

    let issuer = MockIssuer::start(jwks).await;

    let cfg = make_cfg(&issuer.issuer_url);
    let backend = OidcAuthBackend::new(cfg, disabled_inner());

    let mut hdr = Header::new(Algorithm::RS256);
    hdr.kid = Some("it-key-1".into());

    let claims = serde_json::json!({
        "iss": issuer.issuer_url,
        "aud": "pensieve",
        "sub": "scoped-user",
        "exp": now_secs() + 3600,
        "pensieve_databases": ["a", "b"],
    });

    let token = encode(&hdr, &claims, &encoding_key(&key)).unwrap();
    let p = backend.authenticate(&token).await.expect("scoped token should validate");

    assert_eq!(
        p.allowed_databases,
        Some(vec!["a".into(), "b".into()]),
        "allowed_databases should be [a, b]"
    );
    assert_eq!(p.subject, Some("scoped-user".into()));
}
