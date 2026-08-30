//! SupabaseAuthBackend contract tests.
//!
//! Tokens are hand-minted with `jsonwebtoken` (HS256 with a shared secret,
//! RS256 against a throwaway test keypair served as a JWKS via wiremock) so
//! every validation rule — signature, exp, aud, iss — is exercised against
//! real JWT mechanics, no Supabase account needed.

use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_server::auth::{
    AuthBackend, EnvAuthBackend, Role, SessionAuthBackend, SupabaseAuthBackend,
    SupabaseAuthConfig,
};
use std::sync::Arc;

/// Throwaway RSA keypair used ONLY by these tests (never deployed anywhere).
const TEST_RSA_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEAr8YQkr3LZNh1L7nIIXkUzHlIkwOUVINbz9r7VlfLv4yDzWaR
D2Bk1OsrvPf0naEM5z92HQE4ELwzHaXST7bOPY6nBQE6c0Rh52Cec+SEwFWUe05P
QQ0lNchAP50aJo1GPCVIJWhLPjybpIZCcC6clrWo2qonHglR4u6SNpBEsJWPQkMU
P8ZZNwMT9M1BrfYdCn9mKOWl9AHo+sVAODC/aLGIx1PoMa4RoMY0tZvR/mKgPzbQ
PC1GmbgH4h7m8qc4Kr/TBOMqEgKqER7Q2itWEcs9UYxRFXj9Jo6EkhJBaeJOmcWY
gl7irQTRfQ4QHw8SR5nkzAZ0D41ahjIgpBoaewIDAQABAoIBAQCvjcNZvAObefE4
tHWksNjMC8onQujvq66UH6LtLozJiB7Pk8QHtn+ehC7P8lo24PYRNDnUaDZFyKHI
16gAg5TiuEop3nsxSrf5rm8zUqYfrpm4uZLAZs+mMpWws0i9/lWKlrXC3rJfu4q7
vHg4sOUmRNMbadvdzPMjEqGnq1lffqgsyY7dAPKRM0AiXkBB4Xh3U8cSuUyr4hkC
235qvfkv0KinoJPyQ6djxJdB+RiQWl9/qpHgNwyQIeU9xA9EK6klQMeU0+br4DWg
hshNbCn3j/G4i79+6pgrQ0YS8pF6XVTXiZFQ48UyWukICEBb5jIotAOVit4isCfr
LlHglQlhAoGBAN8ULFUJrEEIeTSt6JC0sCHlinMfw29li1DXzel72AAOO0QnMHjf
XrkRvVfyBTWb8L0DM+o0Wqs3yk/aOa2Pz0HiOpn+2AnddFfzabIcAitJiceZJQQH
wa/JJy3hmsZ/4Zgxe8zpCNXghugl8ZAAFp/bgBuCbp9wna6l3cEd2vHLAoGBAMm2
uT7590GWffIrQI9qhhYUro5cx05l0vd4LMtgBCSnud6LjbBeDHW92LF/ValMFN/z
7wYzN2l/9sOE3nc3LTfxYbn1O1K0wcDjBSQ7sWuBAgFcGKAeVXhTI6XvUbmK4s69
Hjn1zDgaEVmasJmW4NTkW6nYFncCwvcfzTezvqQRAoGAL7OinzSITwe+01L0ziy1
FSp+Zou+QM3X8puS/oBq+egRKEuxA8fP+4cdk/a+wm3sFp7etRAo6z/s1RJ3DvQX
f6Eeottp2wIt5Li6O0nd9N+uxK2syqXV9v7uj9MUQ6oI1YCPVovmRcXTU0T52K8M
J3bKeBd2DEYKkdQKDUeTD+0CgYBW/uiMIbCi5+3vyPmyIOYtlcPnAFqxFDdVpc3j
9Mg0quX99kAopZdIHJXdj6Z5OqfyIrme+e3XIWpizuZHklN9Qiy8z+hC9lRuBTtN
cjVFwUEFJxwzyoFgQLMqOLoNhLnnIidsJfdq5ss+0vmBdFIJX2etK9Ycg+NkQ6H1
eR8qkQKBgQDHtguru+FT1f49nDAsXrl6NrthDnX7MnZkAzKbrRcRmfiarFMfv7wN
5ddwPTfJcmdXIvN/MQcaYmyvxOkkeMXV0yMg0qyxSWu80QBuUqutLXNyAZZ9JeD0
3+8j77hg4ND/xLi83THtky4LhyEOEawWYs10XLs/kqoWwGkWKuoqvw==
-----END RSA PRIVATE KEY-----"#;

const TEST_RSA_N: &str = "r8YQkr3LZNh1L7nIIXkUzHlIkwOUVINbz9r7VlfLv4yDzWaRD2Bk1OsrvPf0naEM5z92HQE4ELwzHaXST7bOPY6nBQE6c0Rh52Cec-SEwFWUe05PQQ0lNchAP50aJo1GPCVIJWhLPjybpIZCcC6clrWo2qonHglR4u6SNpBEsJWPQkMUP8ZZNwMT9M1BrfYdCn9mKOWl9AHo-sVAODC_aLGIx1PoMa4RoMY0tZvR_mKgPzbQPC1GmbgH4h7m8qc4Kr_TBOMqEgKqER7Q2itWEcs9UYxRFXj9Jo6EkhJBaeJOmcWYgl7irQTRfQ4QHw8SR5nkzAZ0D41ahjIgpBoaew";

const SECRET: &str = "test-jwt-secret";
const URL: &str = "https://proj.supabase.co";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Mint an HS256 token with explicit claims (full control for negatives).
fn hs256(claims: &serde_json::Value) -> String {
    use jsonwebtoken::{encode, EncodingKey, Header};
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

fn valid_claims(email: &str) -> serde_json::Value {
    serde_json::json!({
        "sub": format!("sub-{email}"),
        "email": email,
        "aud": "authenticated",
        "iss": format!("{URL}/auth/v1"),
        "exp": now() + 3600,
        "role": "authenticated",
    })
}

fn config(url: &str) -> SupabaseAuthConfig {
    SupabaseAuthConfig {
        url: url.to_string(),
        jwt_secret: Some(SECRET.to_string()),
        audience: "authenticated".to_string(),
        admin_emails: vec!["root@corp.com".to_string()],
        allowed_email_domains: vec![],
        default_role: Role::Read,
    }
}

async fn backend(cfg: SupabaseAuthConfig) -> (SupabaseAuthBackend, Arc<SqliteCatalog>) {
    let catalog = Arc::new(SqliteCatalog::connect_in_memory().await.unwrap());
    let env = EnvAuthBackend::from_str("static-admin-tok:admin");
    let inner = SessionAuthBackend::new(catalog.clone(), env, false);
    (
        SupabaseAuthBackend::new(cfg, catalog.clone(), inner),
        catalog,
    )
}

#[tokio::test]
async fn valid_jwt_authenticates_with_default_role() {
    let (be, _) = backend(config(URL)).await;
    let p = be
        .authenticate(&hs256(&valid_claims("alice@example.com")))
        .await
        .expect("valid token authenticates");
    assert_eq!(p.role, Role::Read);
    assert_eq!(p.subject.as_deref(), Some("alice@example.com"));
}

#[tokio::test]
async fn expired_jwt_is_rejected() {
    let (be, _) = backend(config(URL)).await;
    let mut claims = valid_claims("alice@example.com");
    claims["exp"] = serde_json::json!(now() - 7200);
    assert!(be.authenticate(&hs256(&claims)).await.is_err());
}

#[tokio::test]
async fn wrong_audience_is_rejected() {
    let (be, _) = backend(config(URL)).await;
    let mut claims = valid_claims("alice@example.com");
    claims["aud"] = serde_json::json!("anon");
    assert!(be.authenticate(&hs256(&claims)).await.is_err());
}

#[tokio::test]
async fn wrong_issuer_is_rejected() {
    let (be, _) = backend(config(URL)).await;
    let mut claims = valid_claims("alice@example.com");
    claims["iss"] = serde_json::json!("https://evil.example.com/auth/v1");
    assert!(be.authenticate(&hs256(&claims)).await.is_err());
}

#[tokio::test]
async fn bad_signature_is_rejected() {
    use jsonwebtoken::{encode, EncodingKey, Header};
    let (be, _) = backend(config(URL)).await;
    let forged = encode(
        &Header::default(),
        &valid_claims("alice@example.com"),
        &EncodingKey::from_secret(b"wrong-secret"),
    )
    .unwrap();
    assert!(be.authenticate(&forged).await.is_err());
}

#[tokio::test]
async fn admin_email_allowlist_grants_admin() {
    let (be, _) = backend(config(URL)).await;
    let p = be
        .authenticate(&hs256(&valid_claims("root@corp.com")))
        .await
        .unwrap();
    assert_eq!(p.role, Role::Admin);
}

#[tokio::test]
async fn app_metadata_role_claim_is_respected() {
    let (be, _) = backend(config(URL)).await;
    let mut claims = valid_claims("dev@example.com");
    claims["app_metadata"] = serde_json::json!({"role": "write"});
    let p = be.authenticate(&hs256(&claims)).await.unwrap();
    assert_eq!(p.role, Role::Write);
}

#[tokio::test]
async fn admin_allowlist_outranks_app_metadata_role() {
    let (be, _) = backend(config(URL)).await;
    let mut claims = valid_claims("root@corp.com");
    claims["app_metadata"] = serde_json::json!({"role": "read"});
    let p = be.authenticate(&hs256(&claims)).await.unwrap();
    assert_eq!(p.role, Role::Admin);
}

#[tokio::test]
async fn email_domain_allowlist_is_enforced() {
    let mut cfg = config(URL);
    cfg.allowed_email_domains = vec!["corp.com".to_string()];
    let (be, _) = backend(cfg).await;
    assert!(
        be.authenticate(&hs256(&valid_claims("dev@corp.com")))
            .await
            .is_ok(),
        "allowed domain passes"
    );
    assert!(
        be.authenticate(&hs256(&valid_claims("intruder@other.com")))
            .await
            .is_err(),
        "other domains are rejected"
    );
}

#[tokio::test]
async fn opaque_tokens_fall_through_to_inner_backend() {
    let (be, _) = backend(config(URL)).await;
    let p = be
        .authenticate("static-admin-tok")
        .await
        .expect("env static token still works under supabase backend");
    assert_eq!(p.role, Role::Admin);
    assert!(be.authenticate("not-a-known-token").await.is_err());
}

#[tokio::test]
async fn jwt_auth_jit_provisions_a_user() {
    let (be, catalog) = backend(config(URL)).await;
    be.authenticate(&hs256(&valid_claims("alice@example.com")))
        .await
        .unwrap();
    let users = catalog.list_users().await.unwrap();
    assert!(
        users.iter().any(|u| u.username == "alice@example.com"),
        "JIT user created, got: {users:?}"
    );
}

#[tokio::test]
async fn jwks_rs256_validation_works() {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let jwks = serde_json::json!({
        "keys": [{
            "kty": "RSA", "alg": "RS256", "use": "sig",
            "kid": "test-kid-1", "n": TEST_RSA_N, "e": "AQAB",
        }]
    });
    Mock::given(method("GET"))
        .and(path("/auth/v1/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&jwks))
        .mount(&server)
        .await;

    let url = server.uri();
    let mut cfg = config(&url);
    cfg.jwt_secret = None; // force the JWKS path

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-kid-1".to_string());
    let mut claims = valid_claims("rsa@example.com");
    claims["iss"] = serde_json::json!(format!("{url}/auth/v1"));
    let token = encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(TEST_RSA_PEM.as_bytes()).unwrap(),
    )
    .unwrap();

    let (be, _) = backend(cfg).await;
    let p = be.authenticate(&token).await.expect("RS256 via JWKS");
    assert_eq!(p.subject.as_deref(), Some("rsa@example.com"));

    // Unknown kid → rejected (after a refetch attempt finds nothing new).
    let mut header2 = Header::new(Algorithm::RS256);
    header2.kid = Some("rotated-away".to_string());
    let token2 = encode(
        &header2,
        &claims,
        &EncodingKey::from_rsa_pem(TEST_RSA_PEM.as_bytes()).unwrap(),
    )
    .unwrap();
    assert!(be.authenticate(&token2).await.is_err());
}
