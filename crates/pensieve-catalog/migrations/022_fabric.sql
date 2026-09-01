-- Worker/job fabric: the distributed context-engine control plane.
--
-- `workers` are fabric compute identities — the server's embedded in-process
-- worker, or remote daemons (`kyma worker run`) registered from any compute.
-- Distinct from `nodes` (001), which remains server/cluster membership.
--
-- `jobs` are placement-aware work units pulled by workers: `connector_sync`
-- (connector ingestion, cut over from background_tasks' connector_tick),
-- `dreaming` (agentic memory housekeeping), and `source_sync` (node-local
-- source reads, pinned to the owning worker via affinity).

CREATE TABLE IF NOT EXISTS workers (
    id              uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id       uuid NOT NULL,
    name            text NOT NULL,
    hostname        text,
    kind            text NOT NULL CHECK (kind IN ('embedded','daemon')),
    -- sha256 hex of the worker token (`kyw_…`); NULL for the embedded worker,
    -- which authenticates in-process and never over HTTP.
    token_hash      text,
    capabilities    text[] NOT NULL DEFAULT '{}',
    labels          jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- Local sources this worker owns, e.g. {"claude_code":{"roots":[...],"realms":[...]}}.
    sources         jsonb NOT NULL DEFAULT '{}'::jsonb,
    status          text NOT NULL DEFAULT 'offline'
                        CHECK (status IN ('online','draining','offline')),
    max_concurrent  integer NOT NULL DEFAULT 1,
    version         text,
    -- Presence: active coding-agent sessions relayed by the daemon
    -- ([{session_id, realm, last_activity}]). Refreshed on heartbeat.
    presence        jsonb NOT NULL DEFAULT '[]'::jsonb,
    last_heartbeat  timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, name)
);

CREATE INDEX IF NOT EXISTS workers_tenant_status_idx ON workers (tenant_id, status);
CREATE INDEX IF NOT EXISTS workers_token_hash_idx ON workers (token_hash)
    WHERE token_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS jobs (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id          uuid NOT NULL,
    kind               text NOT NULL,   -- 'connector_sync' | 'dreaming' | 'source_sync'
    payload            jsonb NOT NULL DEFAULT '{}'::jsonb,
    status             text NOT NULL DEFAULT 'pending'
                           CHECK (status IN ('pending','claimed','running','done','failed','canceled')),
    priority           integer NOT NULL DEFAULT 0,
    -- Placement: hard pin to one worker (source_sync), and/or required
    -- capabilities the claiming worker must advertise (ALL must match).
    affinity_worker_id uuid REFERENCES workers(id) ON DELETE SET NULL,
    req_capabilities   text[] NOT NULL DEFAULT '{}',
    label_selector     jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- Lease/claim (mirrors background_tasks semantics; dead workers release
    -- their jobs via claim_expires_at TTL + the sweep).
    claimed_by         uuid REFERENCES workers(id) ON DELETE SET NULL,
    claim_expires_at   timestamptz,
    attempt            integer NOT NULL DEFAULT 0,
    max_attempts       integer NOT NULL DEFAULT 3,
    -- Observability + linkage: progress is a live snapshot the executor
    -- replaces as it works ({current_phase, pct, activity:[{ts,icon,text}…],
    -- thinking, counters}); result is the terminal payload; dreaming_run_id
    -- links to memory_pipeline_runs.
    progress           jsonb NOT NULL DEFAULT '{}'::jsonb,
    result             jsonb,
    dreaming_run_id    uuid,
    last_error         text,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- Fast pull: filter + sort path for the claim query.
CREATE INDEX IF NOT EXISTS jobs_pending_idx ON jobs (kind, priority DESC, created_at)
    WHERE status = 'pending';
-- Reclaiming expired leases (dead worker cleanup).
CREATE INDEX IF NOT EXISTS jobs_claimed_expiry_idx ON jobs (claim_expires_at)
    WHERE status IN ('claimed','running');
CREATE INDEX IF NOT EXISTS jobs_affinity_idx ON jobs (affinity_worker_id)
    WHERE status = 'pending';

-- Idempotent connector_sync enqueue: one job per (connector, schedule bucket)
-- while it is in flight — mirrors background_tasks_connector_tick_uniq.
CREATE UNIQUE INDEX IF NOT EXISTS jobs_connector_sync_uniq
    ON jobs ((payload->>'connector_id'), (payload->>'scheduled_for'))
    WHERE kind = 'connector_sync' AND status IN ('pending','claimed','running');

-- One in-flight source_sync per (worker, source) — prevents pile-up when a
-- daemon's watcher fires faster than syncs complete.
CREATE UNIQUE INDEX IF NOT EXISTS jobs_source_sync_uniq
    ON jobs (affinity_worker_id, (payload->>'source'))
    WHERE kind = 'source_sync' AND status IN ('pending','claimed','running');

-- Cutover: connector ticks now ride the jobs fabric. Drain any in-flight
-- connector_tick background_tasks so connectors don't double-run after deploy.
UPDATE background_tasks SET status = 'done', updated_at = now()
    WHERE kind = 'connector_tick' AND status IN ('pending','claimed');
