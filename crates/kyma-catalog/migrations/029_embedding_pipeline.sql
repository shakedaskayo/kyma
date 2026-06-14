-- Async embedding backfill pipeline (S1.5).
--
-- Decouples ingest from embedding: rows can land with an empty/NULL embedding
-- column, and a background `embed_backfill` fabric job computes embeddings for
-- a configured text column and fills them into the vector column by REWRITING
-- the extent (extents are immutable). Two tables support this:
--
--   * `embedding_cache` — a content-addressed cache so the same text is never
--     embedded twice (across re-ingest, duplicates, or re-runs of a job). The
--     key is (tenant, content_hash, model_id); the vector is stored as raw
--     little-endian f32 bytes (dim*4 bytes).
--   * `table_embed_config` — per-table declaration "embed THIS text column INTO
--     THAT vector column with THIS model", consulted by the backfill scheduler
--     and executor.

CREATE TABLE embedding_cache (
    tenant_id    uuid NOT NULL,
    content_hash text NOT NULL,                 -- sha256(source text), hex
    model_id     text NOT NULL,                 -- embedding backend id
    dim          int  NOT NULL,
    embedding    bytea NOT NULL,                -- dim*4 bytes, f32 little-endian
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, content_hash, model_id)
);

CREATE TABLE table_embed_config (
    tenant_id        uuid NOT NULL,
    table_id         uuid NOT NULL,
    source_column    text NOT NULL,             -- text column to embed
    embedding_column text NOT NULL,             -- vector column to fill
    model_id         text NOT NULL,
    dim              int  NOT NULL,
    auto_embed       bool NOT NULL DEFAULT true, -- scheduler auto-enqueues backfill
    updated_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, table_id, embedding_column)
);

CREATE INDEX table_embed_config_table_idx ON table_embed_config (tenant_id, table_id);
