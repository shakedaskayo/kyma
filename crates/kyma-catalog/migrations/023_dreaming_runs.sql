-- Dreaming: scheduled agentic memory housekeeping, recorded alongside the
-- deterministic consolidation runs in the unified memory_pipeline_runs feed
-- (kind = 'dreaming' joins 'consolidation'; the column has no CHECK).
--
-- A dreaming run executes as a fabric `dreaming` job; the executor links the
-- run to its job, to the agent session it minted (conversation drilldown),
-- and to the agent_runs row carrying the full tool-call trace.

ALTER TABLE memory_pipeline_runs
    ADD COLUMN job_id        UUID,
    ADD COLUMN session_id    UUID,
    ADD COLUMN agent_run_id  UUID,
    ADD COLUMN engine        TEXT,
    ADD COLUMN model         TEXT,
    ADD COLUMN worker_id     UUID,
    ADD COLUMN trigger       TEXT NOT NULL DEFAULT 'scheduled'
                              CHECK (trigger IN ('scheduled','manual')),
    -- DreamingOutcome counters (memories created/updated/merged/archived,
    -- entities linked, gaps filled, connector reads, tool calls, …).
    ADD COLUMN stats_json    JSONB,
    -- Final activity-feed snapshot copied from the job at finalize so the
    -- run detail renders without joining the jobs table.
    ADD COLUMN progress_json JSONB;

CREATE INDEX IF NOT EXISTS memory_pipeline_runs_kind_time
    ON memory_pipeline_runs (tenant_id, kind, started_at DESC);

-- Dreaming runs mint agent sessions with source = 'dreaming'.
ALTER TABLE agent_sessions DROP CONSTRAINT IF EXISTS agent_sessions_source_check;
ALTER TABLE agent_sessions ADD CONSTRAINT agent_sessions_source_check
    CHECK (source IN ('kyma','claude_code','dreaming'));

-- At most one dreaming job in flight per tenant: the scheduler and the
-- manual trigger both enqueue with ON CONFLICT DO NOTHING semantics.
CREATE UNIQUE INDEX IF NOT EXISTS jobs_dreaming_uniq
    ON jobs (tenant_id)
    WHERE kind = 'dreaming' AND status IN ('pending','claimed','running');
