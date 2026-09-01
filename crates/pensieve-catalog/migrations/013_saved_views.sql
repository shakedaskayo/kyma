-- 013_saved_views.sql
-- Saved Discover "views": a named scope (set of db.table patterns) owned by a user.
-- Scope is the only thing persisted; search text, filters, and time range stay
-- ephemeral (per spec section 4.2).

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS saved_views (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID        NOT NULL,
    owner_subject   TEXT        NOT NULL,
    name            TEXT        NOT NULL,
    sources_json    JSONB       NOT NULL,
    columns_json    JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT saved_views_owner_name_unique
        UNIQUE (tenant_id, owner_subject, name)
);

CREATE INDEX IF NOT EXISTS saved_views_by_owner
    ON saved_views (tenant_id, owner_subject, updated_at DESC);
