-- Group the access + refresh tokens minted by one login under a session_id so
-- the whole session can be rotated (on refresh) and revoked (on logout)
-- together. NULL for legacy/static tokens.
ALTER TABLE api_tokens ADD COLUMN session_id uuid;
CREATE INDEX api_tokens_session_idx ON api_tokens (session_id);
