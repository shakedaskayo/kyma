-- 012_agent_engine_config.sql
-- Singleton row for the active agent engine. v1 is single-tenant globally; a
-- future per-user / per-tenant variant adds (tenant_id, user_id) columns and
-- relaxes the singleton constraint.
CREATE TABLE engine_config (
    id              SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    kind            TEXT NOT NULL,
    model           TEXT NOT NULL,
    credential_id   UUID REFERENCES credentials(id) ON DELETE SET NULL,
    host            TEXT,
    extras          JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the existing implicit default (Ollama at localhost) so a fresh deploy
-- with no API keys still gets the legacy behaviour out of the box.
INSERT INTO engine_config (id, kind, model, host)
VALUES (1, 'ollama', 'gemma4:latest', 'http://localhost:11434')
ON CONFLICT (id) DO NOTHING;
