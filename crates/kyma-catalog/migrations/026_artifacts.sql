-- Standalone object-store artifacts: full-file blobs (CI job logs,
-- agent-contributed files, filesystem-watch snapshots) that live on the file
-- layer OUTSIDE the columnar extents. The extent GC (PhysicalDeleteWorker)
-- only knows about extents, so these blobs need their own tracking row to be
-- discoverable, retrievable, and retained.
--
-- Lifecycle mirrors the extent soft-delete + grace pattern:
--   * `expires_at` is stamped at write time from the resolved retention setting
--     (NULL = retain forever), and re-stamped when retention settings change.
--   * ArtifactRetentionWorker pass 1 soft-deletes rows past `expires_at`.
--   * pass 2, after a grace period, deletes the blob then the row.
CREATE TABLE artifacts (
    id             uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id      uuid NOT NULL,
    object_path    text NOT NULL,
    source         text NOT NULL,                 -- connector / origin, e.g. 'github', 'fswatch'
    artifact_class text NOT NULL DEFAULT 'log',   -- 'log' | 'file' | ...
    table_ref      text,                          -- 'database.table' this blob is referenced from
    connector_id   uuid,
    size_bytes     bigint NOT NULL DEFAULT 0,
    sha256         text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    expires_at     timestamptz,                   -- NULL = retain forever
    deleted_at     timestamptz,
    UNIQUE (tenant_id, object_path)
);

-- Pass-1 sweep: live rows past expiry. Partial index keeps it cheap.
CREATE INDEX artifacts_expiry_idx ON artifacts (expires_at)
    WHERE deleted_at IS NULL AND expires_at IS NOT NULL;

-- Pass-2 sweep: soft-deleted rows awaiting physical delete.
CREATE INDEX artifacts_gc_idx ON artifacts (deleted_at)
    WHERE deleted_at IS NOT NULL;
