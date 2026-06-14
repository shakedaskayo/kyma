-- Staged extents for the router/committer split (S2.2, server mode).
--
-- A router writes the extent object to storage ("S3 is the WAL"), records its
-- manifest here, and acks — decoupling ack latency from the per-table snapshot
-- CAS. The committer drains these and commits each table's group as one
-- snapshot, deleting the staged rows in the SAME transaction (exactly-once).
--
-- batch_id makes re-staging idempotent (a retried router request is a no-op).
-- Opt-in: nothing writes here unless KYMA_INGEST_MODE=staged.

CREATE TABLE staged_extents (
    id          uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id   uuid NOT NULL,
    table_id    uuid NOT NULL,
    batch_id    uuid NOT NULL,
    manifest    jsonb NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- Idempotent staging: one row per (tenant, batch).
CREATE UNIQUE INDEX staged_extents_batch_uniq ON staged_extents (tenant_id, batch_id);
-- Drain oldest-first.
CREATE INDEX staged_extents_drain ON staged_extents (tenant_id, created_at);
