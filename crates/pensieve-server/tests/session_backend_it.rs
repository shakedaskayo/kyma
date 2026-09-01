//! Integration tests for [`SessionAuthBackend`].
//!
//! Requires `--features pensieve-server/test-support`.
//! Covers: env-token path, db-session-token path, unknown token.

#![cfg(feature = "test-support")]

use pensieve_server::auth::{
    hash_token, AuthBackend, AuthError, EnvAuthBackend, Role, SessionAuthBackend,
};

#[tokio::test]
async fn env_token_authenticates_and_returns_write_role() {
    let state = pensieve_server::test_support::seeded_state_empty().await;
    let backend = SessionAuthBackend::new(
        state.catalog.clone(),
        EnvAuthBackend::from_str("tok:write"),
        false,
    );
    assert!(backend.enabled(), "env token → enabled");

    let p = backend.authenticate("tok").await.unwrap();
    assert_eq!(p.role, Role::Write);
}

#[tokio::test]
async fn db_session_token_authenticates() {
    let state = pensieve_server::test_support::seeded_state_empty().await;
    let cat = &state.catalog;

    // Insert a session token directly.
    let raw = b"my-test-session-token-32-bytes-ok";
    let hash = hash_token(std::str::from_utf8(raw).unwrap());
    cat.insert_api_token(&hash, "admin", Some("alice"), "session", None)
        .await
        .unwrap();

    let backend = SessionAuthBackend::new(
        state.catalog.clone(),
        EnvAuthBackend::from_str(""),
        true, // users_exist
    );
    assert!(backend.enabled());

    let token_str = std::str::from_utf8(raw).unwrap();
    let p = backend.authenticate(token_str).await.unwrap();
    assert_eq!(p.role, Role::Admin);
    assert_eq!(p.subject.as_deref(), Some("alice"));
}

#[tokio::test]
async fn unknown_token_returns_unknown_token_error() {
    let state = pensieve_server::test_support::seeded_state_empty().await;
    let backend = SessionAuthBackend::new(
        state.catalog.clone(),
        EnvAuthBackend::from_str(""),
        false,
    );

    let err = backend.authenticate("no-such-token-at-all").await.unwrap_err();
    assert!(
        matches!(err, AuthError::UnknownToken),
        "expected UnknownToken, got {err:?}"
    );
}

#[tokio::test]
async fn disabled_when_no_users_and_no_env_tokens() {
    let state = pensieve_server::test_support::seeded_state_empty().await;
    let backend = SessionAuthBackend::new(
        state.catalog.clone(),
        EnvAuthBackend::from_str(""),
        false, // no users
    );
    assert!(!backend.enabled(), "no users + no env tokens → disabled");
}

#[tokio::test]
async fn env_token_takes_priority_over_db_lookup() {
    // The env token must match before we even attempt a DB lookup.
    let state = pensieve_server::test_support::seeded_state_empty().await;
    let backend = SessionAuthBackend::new(
        state.catalog.clone(),
        EnvAuthBackend::from_str("fast-tok:read"),
        true,
    );

    // "fast-tok" matches env → should return Read without touching the DB.
    let p = backend.authenticate("fast-tok").await.unwrap();
    assert_eq!(p.role, Role::Read);
}
