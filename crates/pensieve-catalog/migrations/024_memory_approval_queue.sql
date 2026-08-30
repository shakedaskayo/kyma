-- Human-in-the-loop approval queue for memory mutations.
--
-- When the HITL policy (stored in memory_settings.settings.hitl) classifies a
-- mutation as `gate` (hold for approval) or `post_hoc` (apply but flag for
-- review/undo), a row lands here. Gate rows carry the deferred op in `payload`
-- and are applied on approve; post_hoc rows carry a precomputed `inverse` so an
-- undo is deterministic. This is operational metadata (not memory content), so
-- it lives in Postgres alongside memory_settings rather than the columnar store.
-- Local mode (no Postgres) uses a JSON file instead — see memory_queue_store.rs.
CREATE TABLE memory_approval_queue (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          UUID NOT NULL,
    realm              TEXT NOT NULL,
    operation          TEXT NOT NULL,   -- MemoryOp wire string (e.g. 'merge')
    mode               TEXT NOT NULL,   -- 'gate' | 'post_hoc'
    status             TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending','approved','rejected','auto_applied','rolled_back')),
    confidence         REAL,
    reason             TEXT,
    source             TEXT NOT NULL,   -- 'dreaming' | 'realtime' | 'file_promote'
    source_run_id      UUID,
    payload            JSONB NOT NULL,  -- the deferred/applied op descriptor
    inverse            JSONB,           -- precomputed undo (post_hoc rows)
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at        TIMESTAMPTZ,
    resolved_by        TEXT,
    resolution_comment TEXT
);

-- The inbox worklist: pending/auto_applied rows for a tenant, newest first.
CREATE INDEX idx_maq_tenant_status
    ON memory_approval_queue (tenant_id, status, created_at DESC);

-- Deep-link from a dreaming run into the rows it produced.
CREATE INDEX idx_maq_run
    ON memory_approval_queue (source_run_id)
    WHERE source_run_id IS NOT NULL;
