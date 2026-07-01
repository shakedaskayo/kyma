-- Per-memory usage/reinforcement counters (M8.1 — closing the retrieval-quality
-- gap vs. engram's hit/miss + forgetting-curve mechanics).
--
-- Deliberately NOT columns on memory_nodes: every memory_nodes mutation
-- re-embeds and appends a whole new versioned row (append-only latest-wins),
-- so bumping a counter on every recall hit that way would mean paying a full
-- embedding rewrite per hit. This is a small, separately-mutable side table
-- instead — operational metadata, not memory content, same rationale as
-- memory_approval_queue living here rather than in the columnar store.
-- Local mode (no Postgres) uses a JSON file instead — see memory_usage_store.rs.
CREATE TABLE memory_usage_stats (
    memory_id          TEXT NOT NULL,          -- "memory:<uuid>" node id
    tenant_id          UUID NOT NULL,
    realm              TEXT NOT NULL,
    hit_count          BIGINT NOT NULL DEFAULT 0,
    reinforced_count   BIGINT NOT NULL DEFAULT 0,
    miss_count         BIGINT NOT NULL DEFAULT 0,
    last_surfaced_at   TIMESTAMPTZ,
    last_reinforced_at TIMESTAMPTZ,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, memory_id)
);

CREATE INDEX idx_memory_usage_stats_realm ON memory_usage_stats (tenant_id, realm);
