-- External identity-provider users (e.g. Supabase Auth): JIT-provisioned,
-- keyed by (auth_provider, external_id). Password users leave both NULL.
ALTER TABLE users
  ADD COLUMN external_id   text,
  ADD COLUMN auth_provider text;

CREATE UNIQUE INDEX users_external_identity_idx
  ON users (tenant_id, auth_provider, external_id)
  WHERE auth_provider IS NOT NULL AND external_id IS NOT NULL;
