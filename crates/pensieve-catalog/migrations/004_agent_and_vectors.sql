-- 004_agent_and_vectors.sql
-- Day-0 vector primitives + agent infrastructure tables.
-- See docs/superpowers/specs/2026-04-21-nl-query-agent-and-vectors-design.md §4.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE column_metadata (
    database           TEXT NOT NULL,
    table_name         TEXT NOT NULL,
    column_name        TEXT NOT NULL,
    column_type        TEXT NOT NULL,
    description        TEXT,
    embedding_model_id TEXT,
    dimension          INT,
    distance_metric    TEXT,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (database, table_name, column_name)
);

CREATE TABLE schema_embeddings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    database            TEXT NOT NULL,
    table_name          TEXT NOT NULL,
    column_name         TEXT,
    kind                TEXT NOT NULL CHECK (kind IN ('table','column')),
    text_source         TEXT NOT NULL,
    text_source_sha256  BYTEA NOT NULL,
    text_format_version TEXT NOT NULL DEFAULT 'v1',
    model_id            TEXT NOT NULL,
    embedding           vector(384) NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX schema_embeddings_uniq_table
    ON schema_embeddings (database, table_name, model_id)
    WHERE column_name IS NULL;
CREATE UNIQUE INDEX schema_embeddings_uniq_column
    ON schema_embeddings (database, table_name, column_name, model_id)
    WHERE column_name IS NOT NULL;
CREATE INDEX schema_embeddings_hnsw ON schema_embeddings
    USING hnsw (embedding vector_cosine_ops);
CREATE INDEX schema_embeddings_db ON schema_embeddings (database);

CREATE TABLE agent_runs (
    run_id             UUID PRIMARY KEY,
    question           TEXT NOT NULL,
    model_id           TEXT NOT NULL,
    auth_subject       TEXT NOT NULL,
    session_id         UUID,
    started_at         TIMESTAMPTZ NOT NULL,
    finished_at        TIMESTAMPTZ NOT NULL,
    status             TEXT NOT NULL CHECK (status IN
                         ('success','error','budget_exceeded','cancelled','replay_miss')),
    usage_json         JSONB NOT NULL,
    trace_json         JSONB NOT NULL,
    replay_cache_hit   BOOL NOT NULL DEFAULT FALSE
);
CREATE INDEX agent_runs_subject_time ON agent_runs (auth_subject, started_at DESC);
CREATE INDEX agent_runs_session       ON agent_runs (session_id, started_at DESC);

CREATE TABLE agent_sessions (
    session_id    UUID PRIMARY KEY,
    auth_subject  TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_active   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata_json JSONB NOT NULL DEFAULT '{}'
);
CREATE TABLE agent_session_turns (
    session_id   UUID NOT NULL REFERENCES agent_sessions(session_id) ON DELETE CASCADE,
    turn_index   INT  NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('user','assistant')),
    content_json JSONB NOT NULL,
    run_id       UUID REFERENCES agent_runs(run_id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, turn_index)
);

CREATE TABLE agent_replay_cache (
    cache_key     BYTEA PRIMARY KEY,
    layer         TEXT NOT NULL CHECK (layer IN ('generate','run')),
    response_json JSONB NOT NULL,
    model_id      TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hit_count     INT NOT NULL DEFAULT 0
);
CREATE INDEX agent_replay_cache_layer_model ON agent_replay_cache (layer, model_id);
