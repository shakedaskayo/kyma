-- User-tunable memory settings (single row per tenant). Holds ingestion knobs
-- (extraction on/off, min events to consolidate) and retrieval/ranking knobs
-- (hybrid blend weights, recency half-life, RRF k, ANN threshold, default
-- recall limit / graph-expansion hops). Read by the consolidation pipeline and
-- the recall orchestrator; edited from the Memory → Settings UI.
CREATE TABLE memory_settings (
    tenant_id  UUID PRIMARY KEY,
    settings   JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
