-- Background task queue.
--
-- Workers pull tasks with `SELECT ... FOR UPDATE SKIP LOCKED` so many
-- workers can operate concurrently against one Postgres instance without
-- any external queue / coordination protocol. Dead workers release their
-- tasks via `claim_expires_at` TTL.

CREATE TABLE IF NOT EXISTS background_tasks (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    kind               text NOT NULL,   -- 'compaction' | 'retention_sweep' | 'physical_gc'
    table_id           uuid REFERENCES tables(id) ON DELETE CASCADE,
    payload            jsonb NOT NULL DEFAULT '{}'::jsonb,
    status             text NOT NULL DEFAULT 'pending'
                           CHECK (status IN ('pending','claimed','done','failed')),
    claimed_by         uuid REFERENCES nodes(id) ON DELETE SET NULL,
    claim_expires_at   timestamptz,
    priority           integer NOT NULL DEFAULT 0,
    attempt            integer NOT NULL DEFAULT 0,
    max_attempts       integer NOT NULL DEFAULT 3,
    last_error         text,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);

-- Fast pull: index covering the filter + sort path.
CREATE INDEX IF NOT EXISTS background_tasks_pending
    ON background_tasks (kind, priority DESC, created_at)
    WHERE status = 'pending';

-- For reclaiming expired leases (dead worker cleanup).
CREATE INDEX IF NOT EXISTS background_tasks_claimed_expiry
    ON background_tasks (claim_expires_at)
    WHERE status = 'claimed';
