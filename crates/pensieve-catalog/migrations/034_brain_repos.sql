-- Brain-repo registry (hosted mode): one row per published brain — the
-- config JSON (name, realm selector, filters, schedule, gardener) plus the
-- mutable runtime JSON (last export, head commit, capped run ring). Local
-- mode uses ${KYMA_HOME}/brains.json instead — see kyma-local's
-- brain_registry.rs. The bare repos themselves live on the KYMA_BRAIN_DIR
-- volume; this table is only the control-plane record.
CREATE TABLE brain_repos (
    tenant_id  UUID NOT NULL,
    name       TEXT NOT NULL,
    config     JSONB NOT NULL,
    runtime    JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, name)
);
