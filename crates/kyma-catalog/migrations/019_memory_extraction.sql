-- Intelligent (LLM) memory extraction extends the consolidation run record.
-- A run now reports how many entities/relationships it wrote, the A.U.D.N.
-- decision breakdown (added/updated/noop/invalidated), and which mode produced
-- it: 'deterministic' (firehose summary, no engine) or 'extraction' (LLM fact
-- extraction + conflict resolution). Columns are additive + defaulted so the
-- existing overview query keeps working.
ALTER TABLE memory_pipeline_runs
    ADD COLUMN entities_written      BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN relationships_written BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN decisions_json        JSONB,
    ADD COLUMN mode                  TEXT NOT NULL DEFAULT 'deterministic';
