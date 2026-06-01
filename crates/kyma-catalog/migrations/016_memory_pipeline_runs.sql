-- Run history for the memory consolidation ("dreaming") pipeline: each tick
-- that scans the conversation firehose and distills durable memories records a
-- row here so the Agent tab can show ingestion-pipeline activity.
CREATE TABLE memory_pipeline_runs (
    id               UUID PRIMARY KEY,
    tenant_id        UUID NOT NULL,
    kind             TEXT NOT NULL DEFAULT 'consolidation',
    status           TEXT NOT NULL CHECK (status IN ('running','success','error','skipped')),
    started_at       TIMESTAMPTZ NOT NULL,
    finished_at      TIMESTAMPTZ,
    events_scanned   BIGINT NOT NULL DEFAULT 0,
    memories_written BIGINT NOT NULL DEFAULT 0,
    watermark_ts     TIMESTAMPTZ,
    error            TEXT
);
CREATE INDEX memory_pipeline_runs_time ON memory_pipeline_runs (tenant_id, started_at DESC);
