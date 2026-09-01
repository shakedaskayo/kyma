-- 015_memory_sessions.sql
-- Activate multi-turn sessions (the "A5" item) on the dormant agent_sessions /
-- agent_session_turns tables from 004. Adds session metadata + a server-side
-- rolling summary so long conversations stay within the context budget.
-- See docs/superpowers/specs/2026-05-31-agentic-memory-design.md §4 (M0).

ALTER TABLE agent_sessions ADD COLUMN title              TEXT;
ALTER TABLE agent_sessions ADD COLUMN rolling_summary    TEXT;
ALTER TABLE agent_sessions ADD COLUMN summary_turn_index INT  NOT NULL DEFAULT 0;
ALTER TABLE agent_sessions ADD COLUMN source             TEXT NOT NULL DEFAULT 'kyma'
                                       CHECK (source IN ('kyma','claude_code'));

CREATE INDEX agent_sessions_last_active ON agent_sessions (last_active DESC);
