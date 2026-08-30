//! Slice 0 verification gate: both AuthBackend impls work end-to-end.

#![cfg(all(feature = "test-support", feature = "cloud-auth"))]

use pensieve_core::tenant::{TenantId, DEFAULT_TENANT};
use pensieve_server::auth::{AuthBackend, DbAuthBackend, EnvAuthBackend, Role};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

#[tokio::test]
async fn env_backend_accepts_known_token_and_returns_default_tenant() {
    let b = EnvAuthBackend::from_str("alpha:admin,beta:read");
    assert!(b.enabled());
    let p = b.authenticate("alpha").await.unwrap();
    assert_eq!(p.role, Role::Admin);
    assert_eq!(p.tenant, DEFAULT_TENANT);
    let p2 = b.authenticate("beta").await.unwrap();
    assert_eq!(p2.role, Role::Read);
    assert_eq!(p2.tenant, DEFAULT_TENANT);
}

#[tokio::test]
async fn env_backend_rejects_unknown_token() {
    let b = EnvAuthBackend::from_str("alpha:admin");
    assert!(b.authenticate("ghost").await.is_err());
}

#[tokio::test]
async fn db_backend_returns_workspace_tenant() {
    let container = Postgres::default()
        .with_user("pensieve").with_password("pensieve_dev").with_db_name("pensieve")
        .start().await.expect("postgres up");
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://pensieve:pensieve_dev@localhost:{port}/pensieve");
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await.unwrap();

    sqlx::query(r#"CREATE EXTENSION IF NOT EXISTS "pgcrypto""#)
        .execute(&pool).await.unwrap();
    sqlx::query(
        r#"CREATE TABLE api_tokens (
            id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id     uuid NOT NULL,
            token_hash    bytea NOT NULL UNIQUE,
            scopes        text NOT NULL,
            subject       text,
            last_used_at  timestamptz,
            revoked_at    timestamptz,
            expires_at    timestamptz,
            created_at    timestamptz NOT NULL DEFAULT now()
        )"#,
    ).execute(&pool).await.unwrap();

    let workspace_a = TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let workspace_b = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());

    let token_a = "tok-workspace-a-secret";
    let token_b = "tok-workspace-b-secret";
    let hash = |t: &str| {
        let mut h = Sha256::new();
        h.update(t.as_bytes());
        h.finalize().to_vec()
    };

    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject) VALUES ($1, $2, $3, $4)",
    )
    .bind(workspace_a.as_uuid()).bind(hash(token_a)).bind("admin").bind("workspace_a")
    .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject) VALUES ($1, $2, $3, $4)",
    )
    .bind(workspace_b.as_uuid()).bind(hash(token_b)).bind("read").bind("workspace_b")
    .execute(&pool).await.unwrap();

    let backend = DbAuthBackend::new(pool);
    assert!(backend.enabled());

    let pa = backend.authenticate(token_a).await.unwrap();
    assert_eq!(pa.tenant, workspace_a);
    assert_eq!(pa.role, Role::Admin);
    assert_eq!(pa.subject.as_deref(), Some("workspace_a"));

    let pb = backend.authenticate(token_b).await.unwrap();
    assert_eq!(pb.tenant, workspace_b);
    assert_eq!(pb.role, Role::Read);

    assert!(backend.authenticate("nonexistent-token").await.is_err());
}

#[tokio::test]
async fn db_backend_rejects_revoked_tokens() {
    let container = Postgres::default()
        .with_user("pensieve").with_password("pensieve_dev").with_db_name("pensieve")
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://pensieve:pensieve_dev@localhost:{port}/pensieve");
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await.unwrap();

    sqlx::query(r#"CREATE EXTENSION IF NOT EXISTS "pgcrypto""#)
        .execute(&pool).await.unwrap();
    sqlx::query(
        r#"CREATE TABLE api_tokens (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id uuid NOT NULL,
            token_hash bytea NOT NULL UNIQUE,
            scopes text NOT NULL,
            subject text,
            last_used_at timestamptz,
            revoked_at timestamptz,
            expires_at timestamptz,
            created_at timestamptz NOT NULL DEFAULT now()
        )"#,
    ).execute(&pool).await.unwrap();

    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let token = "revoked-tok";
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let hash = h.finalize().to_vec();
    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, revoked_at)
         VALUES ($1, $2, 'admin', now())",
    )
    .bind(tenant.as_uuid()).bind(&hash)
    .execute(&pool).await.unwrap();

    let backend = DbAuthBackend::new(pool);
    assert!(backend.authenticate(token).await.is_err());
}

#[tokio::test]
async fn db_backend_rejects_expired_tokens() {
    let container = Postgres::default()
        .with_user("pensieve").with_password("pensieve_dev").with_db_name("pensieve")
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://pensieve:pensieve_dev@localhost:{port}/pensieve");
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await.unwrap();

    sqlx::query(r#"CREATE EXTENSION IF NOT EXISTS "pgcrypto""#)
        .execute(&pool).await.unwrap();
    sqlx::query(
        r#"CREATE TABLE api_tokens (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id uuid NOT NULL,
            token_hash bytea NOT NULL UNIQUE,
            scopes text NOT NULL,
            subject text,
            last_used_at timestamptz,
            revoked_at timestamptz,
            expires_at timestamptz,
            created_at timestamptz NOT NULL DEFAULT now()
        )"#,
    ).execute(&pool).await.unwrap();

    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let token = "expired-tok";
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let hash = h.finalize().to_vec();
    // Insert token with expires_at already in the past (1 second ago).
    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, expires_at)
         VALUES ($1, $2, 'admin', now() - interval '1 second')",
    )
    .bind(tenant.as_uuid()).bind(&hash)
    .execute(&pool).await.unwrap();

    let backend = DbAuthBackend::new(pool);
    assert!(backend.authenticate(token).await.is_err());
}
