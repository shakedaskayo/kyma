-- System-wide credentials store.
--
-- A `Credential` is a typed, encrypted secret value (PAT, basic auth, OAuth2
-- token pair, connection URL, AWS keys, API key, GitHub App, …) referenced by
-- id. Connectors store a `credential_id` in their config instead of inline
-- secrets so a single rotation propagates; the same store is reusable by MCP
-- servers, agents, and any other subsystem that needs a managed secret.
--
-- `enc_value` is `nonce(12 bytes) || ciphertext(AES-256-GCM)` of the JSON-
-- serialized `CredentialValue`. The encryption key is derived from
-- `KYMA_SECRET_KEY` (env-loaded at server start; SHA-256 stretched if shorter
-- than 32 bytes). `metadata` is public, non-secret information about the
-- credential — masked preview, scopes hint, etc.

CREATE TABLE credentials (
    id          uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id   uuid NOT NULL,
    label       text NOT NULL,
    kind        text NOT NULL,
    enc_value   bytea NOT NULL,
    metadata    jsonb NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, label)
);

CREATE INDEX credentials_tenant_idx ON credentials (tenant_id);
CREATE INDEX credentials_kind_idx   ON credentials (kind);
