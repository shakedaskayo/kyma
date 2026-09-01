-- 007_tenant_id.sql
-- Add tenant_id to every tenant-scoped table for the cloud retrofit.
--
-- Strategy:
--   1. ADD COLUMN with DEFAULT '00000000-...' so existing rows backfill in place.
--   2. Drop the DEFAULT after backfill so future inserts must supply tenant_id.
--   3. Replace single-column UNIQUE constraints with (tenant_id, name) where the
--      cloud product needs same-name resources across workspaces.
--   4. Index every tenant_id column so per-tenant queries hit indices.
--   5. Add connector KMS schema columns now (Slice 2 wires the actual encryption).
--
-- Lock note: `ADD COLUMN ... NOT NULL DEFAULT '...'` is safe on Postgres 11+
-- (no table rewrite for non-volatile defaults), but takes ACCESS EXCLUSIVE
-- briefly per ALTER. Revisit if any of these tables grow past ~10M rows;
-- in that regime, prefer the column-then-default-then-not-null three-step.

-- 1. databases
ALTER TABLE databases ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE databases ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE databases DROP CONSTRAINT IF EXISTS databases_name_key;
ALTER TABLE databases ADD CONSTRAINT databases_tenant_name_uniq UNIQUE (tenant_id, name);
CREATE INDEX databases_tenant_idx ON databases (tenant_id);

-- 2. tables
ALTER TABLE tables ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE tables ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX tables_tenant_idx ON tables (tenant_id);

-- 3. snapshots
ALTER TABLE snapshots ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE snapshots ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX snapshots_tenant_idx ON snapshots (tenant_id);

-- 4. schema_snapshots
ALTER TABLE schema_snapshots ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE schema_snapshots ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX schema_snapshots_tenant_idx ON schema_snapshots (tenant_id);

-- 5. manifests
ALTER TABLE manifests ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE manifests ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX manifests_tenant_idx ON manifests (tenant_id);

-- 6. extents
ALTER TABLE extents ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE extents ALTER COLUMN tenant_id DROP DEFAULT;
DROP INDEX IF EXISTS extents_live;
CREATE INDEX extents_live ON extents (tenant_id, table_id) WHERE deleted_at IS NULL;
CREATE INDEX extents_tenant_idx ON extents (tenant_id);

-- 7. ingest_ledger
ALTER TABLE ingest_ledger ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE ingest_ledger ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE ingest_ledger DROP CONSTRAINT IF EXISTS ingest_ledger_pkey;
ALTER TABLE ingest_ledger ADD CONSTRAINT ingest_ledger_pkey
    PRIMARY KEY (tenant_id, idempotency_key);

-- 8. background_tasks
ALTER TABLE background_tasks ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE background_tasks ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX background_tasks_tenant_idx ON background_tasks (tenant_id);

-- 9. dashboards + dashboard_panels
ALTER TABLE dashboards ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE dashboards ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX dashboards_tenant_idx ON dashboards (tenant_id);
ALTER TABLE dashboard_panels ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE dashboard_panels ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX dashboard_panels_tenant_idx ON dashboard_panels (tenant_id);

-- 10. connectors + connector_cursors + connector_leases
ALTER TABLE connectors ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connectors ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE connectors ADD COLUMN kms_key_id text;
ALTER TABLE connectors ADD COLUMN encrypted_secrets bytea;
ALTER TABLE connectors DROP CONSTRAINT IF EXISTS connectors_name_key;
ALTER TABLE connectors ADD CONSTRAINT connectors_tenant_name_uniq UNIQUE (tenant_id, name);
CREATE INDEX connectors_tenant_idx ON connectors (tenant_id);
ALTER TABLE connector_cursors ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connector_cursors ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX connector_cursors_tenant_idx ON connector_cursors (tenant_id);
ALTER TABLE connector_leases ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connector_leases ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX connector_leases_tenant_idx ON connector_leases (tenant_id);

-- 11. agent surface
ALTER TABLE agent_runs ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_runs ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX agent_runs_tenant_idx ON agent_runs (tenant_id, started_at DESC);
ALTER TABLE agent_sessions ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_sessions ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX agent_sessions_tenant_idx ON agent_sessions (tenant_id);
ALTER TABLE agent_session_turns ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_session_turns ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX agent_session_turns_tenant_idx ON agent_session_turns (tenant_id);
ALTER TABLE agent_replay_cache ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_replay_cache ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE agent_replay_cache DROP CONSTRAINT IF EXISTS agent_replay_cache_pkey;
ALTER TABLE agent_replay_cache ADD CONSTRAINT agent_replay_cache_pkey
    PRIMARY KEY (tenant_id, cache_key);

-- 12. column_metadata, schema_embeddings
ALTER TABLE column_metadata ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE column_metadata ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE column_metadata DROP CONSTRAINT IF EXISTS column_metadata_pkey;
ALTER TABLE column_metadata ADD CONSTRAINT column_metadata_pkey
    PRIMARY KEY (tenant_id, database, table_name, column_name);

ALTER TABLE schema_embeddings ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE schema_embeddings ALTER COLUMN tenant_id DROP DEFAULT;
DROP INDEX IF EXISTS schema_embeddings_uniq_table;
DROP INDEX IF EXISTS schema_embeddings_uniq_column;
DROP INDEX IF EXISTS schema_embeddings_db;
CREATE UNIQUE INDEX schema_embeddings_uniq_table
    ON schema_embeddings (tenant_id, database, table_name, model_id)
    WHERE column_name IS NULL;
CREATE UNIQUE INDEX schema_embeddings_uniq_column
    ON schema_embeddings (tenant_id, database, table_name, column_name, model_id)
    WHERE column_name IS NOT NULL;
CREATE INDEX schema_embeddings_db
    ON schema_embeddings (tenant_id, database);

-- nodes is intentionally NOT tenant-scoped — node identity is cluster-global.
