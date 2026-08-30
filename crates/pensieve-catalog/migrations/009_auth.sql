CREATE TABLE users (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id     uuid NOT NULL,
  username      text NOT NULL,
  password_hash text NOT NULL,          -- argon2 PHC string
  role          text NOT NULL,          -- 'admin' | 'write' | 'read'
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, username)
);
CREATE INDEX users_tenant_idx ON users (tenant_id);

-- api_tokens: documented in db_backend.rs but never migrated. Add it now;
-- both login sessions AND static API tokens live here.
CREATE TABLE api_tokens (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id     uuid NOT NULL,
  token_hash    bytea NOT NULL UNIQUE,  -- SHA-256(presented token)
  scopes        text NOT NULL,          -- 'admin'|'write'|'read'
  subject       text,                   -- user id/username or token label
  kind          text NOT NULL DEFAULT 'session',  -- 'session' | 'api'
  expires_at    timestamptz,
  last_used_at  timestamptz,
  revoked_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX api_tokens_tenant_idx ON api_tokens (tenant_id);
