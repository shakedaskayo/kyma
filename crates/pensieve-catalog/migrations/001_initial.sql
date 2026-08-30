-- kyma-catalog — initial schema (Iceberg-mirroring metadata model)
--
-- Runs once at startup via sqlx::migrate!() — idempotent by virtue of
-- `IF NOT EXISTS` guards and sqlx's _sqlx_migrations tracking.

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- ------------------------------------------------------------------
-- databases & tables
-- ------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS databases (
    id          uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    name        text NOT NULL UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS schema_snapshots (
    id           uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    table_id     uuid NOT NULL,
    -- Serialized Arrow schema (JSON form) for portability.
    arrow_schema jsonb NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS tables (
    id                    uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    database_id           uuid NOT NULL REFERENCES databases(id) ON DELETE CASCADE,
    name                  text NOT NULL,
    -- current_snapshot_id / schema_snapshot_id are filled in after the first
    -- snapshot is created. They are NOT NULL after initial-commit; to allow
    -- insert-then-snapshot bootstrap we keep them nullable at the row level
    -- but enforce non-null via CHECK after initial setup in the app layer.
    current_snapshot_id   uuid,
    schema_snapshot_id    uuid REFERENCES schema_snapshots(id),
    config                jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at            timestamptz NOT NULL DEFAULT now(),
    UNIQUE (database_id, name)
);

ALTER TABLE schema_snapshots
    ADD CONSTRAINT schema_snapshots_table_fk
    FOREIGN KEY (table_id) REFERENCES tables(id) ON DELETE CASCADE
    DEFERRABLE INITIALLY DEFERRED;

-- ------------------------------------------------------------------
-- snapshots, manifests, extents
-- ------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS snapshots (
    id                  uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    table_id            uuid NOT NULL REFERENCES tables(id) ON DELETE CASCADE,
    parent_id           uuid REFERENCES snapshots(id),
    sequence_number     bigint NOT NULL,
    schema_snapshot_id  uuid NOT NULL REFERENCES schema_snapshots(id),
    summary             jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at          timestamptz NOT NULL DEFAULT now(),
    UNIQUE (table_id, sequence_number)
);

ALTER TABLE tables
    ADD CONSTRAINT tables_current_snapshot_fk
    FOREIGN KEY (current_snapshot_id) REFERENCES snapshots(id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE IF NOT EXISTS manifests (
    id             uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    snapshot_id    uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    kind           text NOT NULL CHECK (kind IN ('data','delete','compaction')),
    extent_count   integer NOT NULL DEFAULT 0,
    byte_size      bigint NOT NULL DEFAULT 0,
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS extents (
    id                   uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    table_id             uuid NOT NULL REFERENCES tables(id) ON DELETE CASCADE,
    manifest_id          uuid REFERENCES manifests(id) ON DELETE CASCADE,
    schema_snapshot_id   uuid NOT NULL REFERENCES schema_snapshots(id),
    object_path          text NOT NULL,
    byte_size            bigint NOT NULL,
    row_count            bigint NOT NULL,
    min_timestamp        timestamptz,
    max_timestamp        timestamptz,
    column_stats         jsonb NOT NULL DEFAULT '{}'::jsonb,
    present_paths        text[] NOT NULL DEFAULT '{}'::text[],
    bloom_filters        bytea,
    compaction_gen       integer NOT NULL DEFAULT 0,
    created_at           timestamptz NOT NULL DEFAULT now(),
    deleted_at           timestamptz
);

-- Pruning indices — first level of the three-level cascade.
-- Time-range pruning uses GIST over tstzrange.
CREATE INDEX IF NOT EXISTS extents_tbl_ts_range
    ON extents USING gist (table_id, tstzrange(min_timestamp, max_timestamp));

-- Dynamic-path pruning uses GIN.
CREATE INDEX IF NOT EXISTS extents_present_paths
    ON extents USING gin (present_paths);

-- Live-extent listing fast path.
CREATE INDEX IF NOT EXISTS extents_live
    ON extents (table_id)
    WHERE deleted_at IS NULL;

-- ------------------------------------------------------------------
-- nodes — cluster membership + heartbeat
-- ------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS nodes (
    id              uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    role            text NOT NULL CHECK (role IN ('all_in_one','ingest','query','compaction')),
    endpoint        text NOT NULL,
    capabilities    jsonb NOT NULL DEFAULT '{}'::jsonb,
    lease_id        uuid NOT NULL,
    last_heartbeat  timestamptz NOT NULL DEFAULT now(),
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS nodes_last_heartbeat
    ON nodes (last_heartbeat);
