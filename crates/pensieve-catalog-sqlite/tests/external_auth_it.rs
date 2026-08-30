//! JIT external-identity user provisioning (`upsert_external_user_in_tenant`)
//! against the SQLite catalog — the contract the Supabase auth backend relies
//! on: create on first sight, refresh on later sights, keyed by
//! (auth_provider, external_id), never duplicating users.

use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::tenant::DEFAULT_TENANT;

#[tokio::test]
async fn upsert_external_user_creates_then_refreshes() {
    let cat = SqliteCatalog::connect_in_memory().await.unwrap();

    // First sight: JIT-creates the user with the resolved role.
    let u1 = cat
        .upsert_external_user_in_tenant(
            DEFAULT_TENANT,
            "supabase",
            "sub-123",
            "alice@example.com",
            "read",
        )
        .await
        .unwrap();
    assert_eq!(u1.username, "alice@example.com");
    assert_eq!(u1.role, "read");

    // Same identity again with a new email + elevated role: updates in place.
    let u2 = cat
        .upsert_external_user_in_tenant(
            DEFAULT_TENANT,
            "supabase",
            "sub-123",
            "alice@corp.com",
            "admin",
        )
        .await
        .unwrap();
    assert_eq!(u2.id, u1.id, "same external identity → same user row");
    assert_eq!(u2.username, "alice@corp.com");
    assert_eq!(u2.role, "admin");
    assert_eq!(cat.count_users().await.unwrap(), 1, "no duplicate rows");

    // A different subject is a different user.
    let u3 = cat
        .upsert_external_user_in_tenant(
            DEFAULT_TENANT,
            "supabase",
            "sub-456",
            "bob@example.com",
            "read",
        )
        .await
        .unwrap();
    assert_ne!(u3.id, u1.id);
    assert_eq!(cat.count_users().await.unwrap(), 2);
}

#[tokio::test]
async fn external_users_coexist_with_password_users() {
    let cat = SqliteCatalog::connect_in_memory().await.unwrap();

    // A classic password user (no external identity).
    cat.create_user("admin", "$argon2id$fake", "admin")
        .await
        .unwrap();

    // An external user with a different username.
    cat.upsert_external_user_in_tenant(
        DEFAULT_TENANT,
        "supabase",
        "sub-1",
        "alice@example.com",
        "read",
    )
    .await
    .unwrap();

    assert_eq!(cat.count_users().await.unwrap(), 2);
    // Password lookup for the external user's name yields a row whose hash can
    // never verify (sentinel, not a valid PHC string) — external users cannot
    // password-login.
    let (_, hash) = cat
        .get_user_with_hash("alice@example.com")
        .await
        .unwrap()
        .expect("external user visible to user lookups");
    assert!(
        kyma_server_test_helpers::phc_parse_fails(&hash),
        "external-user sentinel hash must not be a valid PHC string"
    );
}

/// Tiny local helper namespace so the test reads clearly without depending on
/// kyma-server: a valid argon2 PHC string starts with `$argon2`.
mod kyma_server_test_helpers {
    pub fn phc_parse_fails(hash: &str) -> bool {
        !hash.starts_with("$argon2")
    }
}
