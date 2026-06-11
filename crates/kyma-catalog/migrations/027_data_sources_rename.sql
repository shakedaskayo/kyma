-- 027_data_sources_rename.sql
-- Clean-break rename: connectors -> data_sources (concept-wide).
--
-- Covers everything connector-named that migrations 001-026 created:
--   * 005: the three base tables, their columns/indexes, and the
--     background_tasks tick-dedup index.
--   * 007: tenant indexes + the (tenant_id, name) unique constraint.
--   * 017: oauth_flows.connector_type column.
--   * 022: the jobs fabric — 'connector_sync' kind, payload key, the
--     jobs_connector_sync_uniq dedup index, and the 'connector' capability
--     vocabulary on workers.capabilities / jobs.req_capabilities.
--   * 026: artifacts.connector_id column.
--   * ingest_ledger idempotency keys minted as 'connector:<id>:<ts>' by the
--     runner (so a tick retried across the deploy still dedups).

-- ------------------------------------------------------------------
-- Base tables (005) + columns
-- ------------------------------------------------------------------
ALTER TABLE connectors RENAME TO data_sources;
ALTER TABLE connector_cursors RENAME TO data_source_cursors;
ALTER TABLE connector_leases RENAME TO data_source_leases;

ALTER TABLE data_source_cursors RENAME COLUMN connector_id TO data_source_id;
ALTER TABLE data_source_leases RENAME COLUMN connector_id TO data_source_id;

ALTER INDEX connectors_enabled_drive_idx RENAME TO data_sources_enabled_drive_idx;

-- Constraint names left behind by the table renames (005 auto-named +
-- 007 explicit). Cheap coherence so \d output reads sanely.
ALTER TABLE data_sources RENAME CONSTRAINT connectors_pkey TO data_sources_pkey;
ALTER TABLE data_sources RENAME CONSTRAINT connectors_schedule_ms_check TO data_sources_schedule_ms_check;
ALTER TABLE data_sources RENAME CONSTRAINT connectors_drive_model_check TO data_sources_drive_model_check;
ALTER TABLE data_sources RENAME CONSTRAINT connectors_tenant_name_uniq TO data_sources_tenant_name_uniq;
ALTER TABLE data_source_cursors RENAME CONSTRAINT connector_cursors_pkey TO data_source_cursors_pkey;
ALTER TABLE data_source_cursors RENAME CONSTRAINT connector_cursors_connector_id_fkey TO data_source_cursors_data_source_id_fkey;
ALTER TABLE data_source_leases RENAME CONSTRAINT connector_leases_pkey TO data_source_leases_pkey;
ALTER TABLE data_source_leases RENAME CONSTRAINT connector_leases_connector_id_fkey TO data_source_leases_data_source_id_fkey;

-- Tenant indexes from 007.
ALTER INDEX connectors_tenant_idx RENAME TO data_sources_tenant_idx;
ALTER INDEX connector_cursors_tenant_idx RENAME TO data_source_cursors_tenant_idx;
ALTER INDEX connector_leases_tenant_idx RENAME TO data_source_leases_tenant_idx;

-- ------------------------------------------------------------------
-- background_tasks (003/005): rekey pending tick payloads while the old
-- kind is still in place, then flip the kind.
-- ------------------------------------------------------------------
UPDATE background_tasks
   SET payload = (payload - 'connector_id')
                 || jsonb_build_object('data_source_id', payload->'connector_id')
 WHERE kind IN ('connector_tick','connector_sync') AND payload ? 'connector_id';

UPDATE background_tasks SET kind = 'data_source_tick' WHERE kind = 'connector_tick';
UPDATE background_tasks SET kind = 'data_source_sync' WHERE kind = 'connector_sync';

-- The dedup index keys on the payload expression + kind predicate, so a
-- RENAME is not enough: rebuild it under the new vocabulary.
DROP INDEX IF EXISTS background_tasks_connector_tick_uniq;
CREATE UNIQUE INDEX IF NOT EXISTS background_tasks_data_source_tick_uniq
    ON background_tasks ((payload->>'data_source_id'),
                         (payload->>'scheduled_for'))
    WHERE kind = 'data_source_tick' AND status IN ('pending', 'claimed');

-- ------------------------------------------------------------------
-- Jobs fabric (022): same rekey for 'connector_sync' jobs, plus the
-- capability vocabulary on workers and job requirements.
-- ------------------------------------------------------------------
UPDATE jobs
   SET payload = (payload - 'connector_id')
                 || jsonb_build_object('data_source_id', payload->'connector_id')
 WHERE kind = 'connector_sync' AND payload ? 'connector_id';

UPDATE jobs SET kind = 'data_source_sync' WHERE kind = 'connector_sync';

UPDATE jobs
   SET req_capabilities = array_replace(req_capabilities, 'connector', 'data_source')
 WHERE 'connector' = ANY(req_capabilities);

UPDATE workers
   SET capabilities = array_replace(capabilities, 'connector', 'data_source'),
       updated_at = now()
 WHERE 'connector' = ANY(capabilities);

DROP INDEX IF EXISTS jobs_connector_sync_uniq;
CREATE UNIQUE INDEX IF NOT EXISTS jobs_data_source_sync_uniq
    ON jobs ((payload->>'data_source_id'), (payload->>'scheduled_for'))
    WHERE kind = 'data_source_sync' AND status IN ('pending','claimed','running');

-- ------------------------------------------------------------------
-- oauth_flows (017): connector_type is a real column. OAuth-minted
-- credentials also stamp the type into their metadata jsonb (011) —
-- rekey those so old credentials keep displaying their type.
-- ------------------------------------------------------------------
ALTER TABLE oauth_flows RENAME COLUMN connector_type TO data_source_type;

UPDATE credentials
   SET metadata = (metadata - 'connector_type')
                  || jsonb_build_object('data_source_type', metadata->'connector_type'),
       updated_at = now()
 WHERE metadata ? 'connector_type';

-- ------------------------------------------------------------------
-- memory_settings (020): the `dreaming` object inside the per-tenant
-- settings jsonb carries operator budgets under the old serde field
-- names. The struct loads with `#[serde(default)]`, so an old-keyed row
-- would silently reset those budgets to defaults — rekey in place.
-- (DreamingSettings also carries serde aliases for the old keys, so
-- this is belt-and-braces rather than load-bearing.)
-- ------------------------------------------------------------------
UPDATE memory_settings
   SET settings = jsonb_set(settings, '{dreaming}',
                  ((settings->'dreaming') - 'connector_read_budget')
                  || jsonb_build_object('data_source_read_budget',
                                        settings->'dreaming'->'connector_read_budget')),
       updated_at = now()
 WHERE settings->'dreaming' ? 'connector_read_budget';

UPDATE memory_settings
   SET settings = jsonb_set(settings, '{dreaming}',
                  ((settings->'dreaming') - 'connector_read_max_bytes')
                  || jsonb_build_object('data_source_read_max_bytes',
                                        settings->'dreaming'->'connector_read_max_bytes')),
       updated_at = now()
 WHERE settings->'dreaming' ? 'connector_read_max_bytes';

-- ------------------------------------------------------------------
-- memory_pipeline_runs (016, stats_json added in 023): DreamingOutcome
-- counters from finished runs carry the old `connector_reads` key.
-- ------------------------------------------------------------------
UPDATE memory_pipeline_runs
   SET stats_json = (stats_json - 'connector_reads')
                    || jsonb_build_object('data_source_reads',
                                          stats_json->'connector_reads')
 WHERE stats_json ? 'connector_reads';

-- ------------------------------------------------------------------
-- artifacts (026): provenance column.
-- ------------------------------------------------------------------
ALTER TABLE artifacts RENAME COLUMN connector_id TO data_source_id;

-- ------------------------------------------------------------------
-- ingest_ledger: the runner mints idempotency keys as
-- 'connector:<data_source_id>[:<table>]:<scheduled_for_ns>'. Rewrite stored
-- keys so a tick that straddles the deploy still deduplicates.
-- ------------------------------------------------------------------
UPDATE ingest_ledger
   SET idempotency_key = 'data_source:' || substr(idempotency_key, length('connector:') + 1)
 WHERE idempotency_key LIKE 'connector:%';

-- ------------------------------------------------------------------
-- Watcher registry (file watchers / cc-sync provenance). Server-mode only;
-- kyma-local keeps an in-process equivalent.
-- ------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS data_source_watchers (
    id                uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    kind              text NOT NULL CHECK (kind IN ('filedrop','cc_sync')),
    node_host         text NOT NULL,
    node_id           text NOT NULL,
    identity          text NOT NULL,
    config_jsonb      jsonb NOT NULL,
    config_hash       text NOT NULL,
    started_at        timestamptz NOT NULL DEFAULT now(),
    last_heartbeat_at timestamptz NOT NULL DEFAULT now(),
    last_scan_jsonb   jsonb,
    UNIQUE (kind, node_id, config_hash)
);
