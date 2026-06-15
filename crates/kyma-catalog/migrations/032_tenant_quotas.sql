-- Per-tenant resource quotas (S2.6 ops hardening).
--
-- One row per tenant that configures DIFFERENT admission limits than the
-- process-global KYMA_*_MAX_CONCURRENT[_PER_TENANT] env defaults. NULL in a
-- column means that dimension is not overridden for the tenant (env default
-- applies). Only the dimensions whose request path carries a tenant are
-- configurable here (query + agent concurrency); ingest rate limiting keys by
-- database and stays env/per-db.
--
-- The query/agent admission limiter reads an in-RAM cache refreshed from this
-- table, so a missing row (the default) keeps every existing deployment and the
-- fresh-install byte-identical.

CREATE TABLE tenant_quotas (
    tenant_id             uuid PRIMARY KEY,
    max_query_concurrent  integer,
    max_agent_concurrent  integer,
    updated_at            timestamptz NOT NULL DEFAULT now()
);
