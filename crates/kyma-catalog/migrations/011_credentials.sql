-- 011_credentials.sql
-- Credential storage referenced by migration 012's engine_config.credential_id FK.
-- Schema kept minimal — nothing in the engine reads or writes this table yet,
-- it exists so migration 012's foreign key resolves on fresh deploys.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS credentials (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT        NOT NULL UNIQUE,
    kind        TEXT        NOT NULL,
    secret_enc  BYTEA,
    secret_ref  TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (secret_enc IS NOT NULL OR secret_ref IS NOT NULL)
);
