-- Global ANN centroid tree (S1.3, server mode only).
--
-- One row per (table, column, embedding model). The tree blob (global centroids
-- + posting lists of (extent, local-partition)) lives on object storage at
-- object_path; this row is the catalog handle + staleness metadata. generation
-- bumps on every rebuild; extent_fingerprint is a stable hash of the live
-- extent-id set the tree was built from, so the maintenance scheduler can skip
-- rebuilds when nothing changed and the query path can detect staleness.
--
-- The query path falls back to per-extent fan-out when no row exists, so this is
-- purely a latency optimization — never load-bearing for correctness.

CREATE TABLE ann_tree (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id          uuid NOT NULL,
    table_id           uuid NOT NULL,
    column_name        text NOT NULL,
    embedding_model_id text,
    generation         bigint NOT NULL DEFAULT 0,
    extent_fingerprint text NOT NULL,
    object_path        text NOT NULL,
    byte_size          bigint NOT NULL,
    params             jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at         timestamptz NOT NULL DEFAULT now()
);

-- One tree per (table, column, model): re-registering upserts in place.
CREATE UNIQUE INDEX ann_tree_uniq
    ON ann_tree (table_id, column_name, COALESCE(embedding_model_id, ''));
CREATE INDEX ann_tree_tenant ON ann_tree (tenant_id);
