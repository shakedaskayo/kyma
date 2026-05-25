//! Integration tests for the auth catalog layer (users + api_tokens).
//!
//! Requires the `test-support` feature (testcontainers Postgres with all
//! migrations applied, including 009_auth).

#![cfg(feature = "test-support")]

use sha2::{Digest, Sha256};

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().to_vec()
}

#[tokio::test]
async fn auth_user_and_token_crud_roundtrip() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let cat = &state.catalog;

    // --- users ---

    // Initially empty.
    assert_eq!(cat.count_users().await.unwrap(), 0);

    // Create a user.
    let phc = kyma_server::auth::passwords::hash_password("hunter2").unwrap();
    let user = cat
        .create_user("alice", &phc, "admin")
        .await
        .unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.role, "admin");

    // Count is now 1.
    assert_eq!(cat.count_users().await.unwrap(), 1);

    // get_user_with_hash returns the user + the stored hash.
    let (got_user, got_hash) = cat
        .get_user_with_hash("alice")
        .await
        .unwrap()
        .expect("user should exist");
    assert_eq!(got_user.id, user.id);
    assert_eq!(got_user.username, "alice");
    assert_eq!(got_hash, phc);

    // Non-existent user → None.
    assert!(cat.get_user_with_hash("bob").await.unwrap().is_none());

    // --- api tokens ---

    let raw_token = b"my-super-secret-session-token-32bytes";
    let token_hash = sha256(raw_token);

    // Insert a session token for alice.
    cat.insert_api_token(&token_hash, "admin", Some("alice"), "session", None)
        .await
        .unwrap();

    // lookup_api_token returns a TokenPrincipal.
    let principal = cat
        .lookup_api_token(&token_hash)
        .await
        .unwrap()
        .expect("token should be found");
    assert_eq!(principal.role, "admin");
    assert_eq!(principal.subject.as_deref(), Some("alice"));

    // Revoke the token → returns true.
    assert!(cat.revoke_api_token(&token_hash).await.unwrap());

    // After revocation, lookup returns None.
    assert!(cat.lookup_api_token(&token_hash).await.unwrap().is_none());

    // Second revoke → false (already revoked).
    assert!(!cat.revoke_api_token(&token_hash).await.unwrap());
}
