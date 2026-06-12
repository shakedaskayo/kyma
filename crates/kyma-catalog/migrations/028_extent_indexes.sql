-- Index sidecars (S0.4): one row per auxiliary index object (ANN / FTS) built
-- from one extent's data and stored on the object store next to the extent
-- (`{tenant}/indexes/{extent_id}/{column}.{kind}`).
--
-- Lifecycle:
--   * Builders upload the object FIRST, then register the row — a crash in
--     between leaves an orphan object, never a dangling row.
--   * Rows cascade with their extent. The physical-delete worker reads the
--     sidecar object paths for GC-candidate extents and deletes those objects
--     under the same grace period as the extent data, before the extent rows
--     (and, via cascade, these rows) are dropped.
CREATE TABLE extent_indexes (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id          uuid NOT NULL,
    table_id           uuid NOT NULL,
    extent_id          uuid NOT NULL REFERENCES extents(id) ON DELETE CASCADE,
    column_name        text NOT NULL,
    kind               text NOT NULL,                  -- 'ivf_rabitq' | 'tantivy_fts'
    object_path        text NOT NULL,
    byte_size          bigint NOT NULL,
    params             jsonb NOT NULL DEFAULT '{}'::jsonb,
    embedding_model_id text,                           -- ANN only; NULL for FTS
    created_at         timestamptz NOT NULL DEFAULT now()
);

-- Idempotency key: one sidecar per (extent, column, kind, model).
-- embedding_model_id is nullable and Postgres UNIQUE treats NULLs as
-- distinct, so uniqueness (and the upsert conflict target) goes through
-- COALESCE(embedding_model_id, '') instead of the raw column.
CREATE UNIQUE INDEX extent_indexes_uniq
    ON extent_indexes (extent_id, column_name, kind, COALESCE(embedding_model_id, ''));

-- Planner / job lookup: which sidecars exist for (table, column, kind).
CREATE INDEX extent_indexes_lookup ON extent_indexes (table_id, column_name, kind);

CREATE INDEX extent_indexes_tenant_idx ON extent_indexes (tenant_id);
