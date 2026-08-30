-- 035_rename_source_marker.sql
-- Kyma -> Pensieve rebrand: move the stored 'kyma' source marker to 'pensieve'.
--
-- Migrations 001-034 are frozen byte-for-byte: sqlx::migrate!() checksums every
-- applied migration and validates on startup, so editing 015 or 023 in place
-- would break any database that has already run them. The two CHECK constraints
-- that pin the literal move forward here instead.
--
-- The Rust counterpart is pensieve_ccmem::SOURCE_MARKER.

-- Drop the constraint first so the UPDATE cannot transiently violate it.
ALTER TABLE agent_sessions DROP CONSTRAINT IF EXISTS agent_sessions_source_check;

UPDATE agent_sessions SET source = 'pensieve' WHERE source = 'kyma';

ALTER TABLE agent_sessions ALTER COLUMN source SET DEFAULT 'pensieve';

ALTER TABLE agent_sessions ADD CONSTRAINT agent_sessions_source_check
    CHECK (source IN ('pensieve','claude_code','dreaming'));
