-- 013_enabled_skills.sql
-- Singleton row holding the names of skills the user has toggled on. v1 is
-- single-tenant; a future per-tenant variant adds (tenant_id) and relaxes
-- the singleton constraint.
CREATE TABLE enabled_skills (
    id          SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    skills      TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[],
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO enabled_skills (id) VALUES (1) ON CONFLICT (id) DO NOTHING;
