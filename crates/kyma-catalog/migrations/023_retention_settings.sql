-- Per-tenant data-retention settings (single row per tenant). Holds the global
-- default plus per-source / per-table / per-artifact-class day-counts. Read by
-- the retention worker (to stamp artifact `expires_at`) and the settings API;
-- edited from the Settings → Retention UI. Mirrors `memory_settings`.
CREATE TABLE retention_settings (
    tenant_id  UUID PRIMARY KEY,
    settings   JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
