-- Ingest idempotency ledger.
--
-- A row per (idempotency_key) that records the effect of a prior ingest
-- so a duplicate POST can return the original response without re-ingesting.

CREATE TABLE IF NOT EXISTS ingest_ledger (
    idempotency_key text PRIMARY KEY,
    table_id        uuid NOT NULL REFERENCES tables(id) ON DELETE CASCADE,
    snapshot_id     uuid NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    rows_ingested   bigint NOT NULL,
    bytes_written   bigint NOT NULL,
    applied_at      timestamptz NOT NULL DEFAULT now(),
    ttl_expires_at  timestamptz NOT NULL
);

CREATE INDEX IF NOT EXISTS ingest_ledger_ttl
    ON ingest_ledger (ttl_expires_at);
