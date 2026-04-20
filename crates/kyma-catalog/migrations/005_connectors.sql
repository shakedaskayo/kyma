-- Catalog tables for the ingestion-connector subsystem.
--
-- * `connectors` — operator-managed definitions, one row per connector instance.
-- * `connector_cursors` — per-connector checkpoint state (API cursor / last
--   timestamp / etc). Separate table so cursor updates are small, frequent
--   writes that don't churn the connectors row.
-- * `connector_leases` — pre-provisioned for the `Continuous` drive model
--   (streaming connectors). Unused in slice-1.
-- * The unique index on `background_tasks` prevents duplicate `connector_tick`
--   enqueues when multiple kyma nodes run schedulers concurrently.

CREATE TABLE IF NOT EXISTS connectors (
    id                  uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    name                text NOT NULL UNIQUE,
    type                text NOT NULL,
    target_database     text NOT NULL,
    target_table        text NOT NULL,
    config_jsonb        jsonb NOT NULL,
    schedule_ms         bigint NOT NULL CHECK (schedule_ms >= 100),
    drive_model         text NOT NULL
                            CHECK (drive_model IN ('periodic','continuous')),
    enabled             boolean NOT NULL DEFAULT TRUE,
    disabled_reason     text,
    last_run_at         timestamptz,
    last_success_at     timestamptz,
    last_error          text,
    last_rows_ingested  bigint,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS connectors_enabled_drive_idx
    ON connectors (drive_model, enabled)
    WHERE enabled = TRUE;

CREATE TABLE IF NOT EXISTS connector_cursors (
    connector_id  uuid PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    cursor_jsonb  jsonb,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS connector_leases (
    connector_id  uuid PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    node_id       text NOT NULL,
    expires_at    timestamptz NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS background_tasks_connector_tick_uniq
    ON background_tasks ((payload->>'connector_id'),
                         (payload->>'scheduled_for'))
    WHERE kind = 'connector_tick' AND status IN ('pending', 'claimed');
