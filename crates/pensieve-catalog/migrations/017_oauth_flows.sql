-- OAuth2 authorization-code flows — short-lived PKCE / CSRF-state records.
--
-- One row per in-progress "Connect <provider>" flow. Created by
-- POST /v1/oauth/:provider/start (which stores the encrypted PKCE verifier and
-- stamps the tenant that initiated the flow) and consumed exactly once by the
-- unauthenticated GET callback the identity provider redirects to. The
-- single-use random `state` token is the entire CSRF boundary: the callback
-- carries no bearer, so it derives its tenant solely from this row.
--
-- Rows are valid for ~10 minutes (`expires_at`); a callback past that, or for an
-- already-consumed `state`, is rejected. `enc_code_verifier` reuses the same
-- AES-256-GCM key as the `credentials` table (`KYMA_SECRET_KEY`).

CREATE TABLE oauth_flows (
    id                 uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    tenant_id          uuid NOT NULL,
    state              text NOT NULL UNIQUE,
    provider           text NOT NULL,
    connector_type     text NOT NULL,
    label              text NOT NULL,
    scopes             text NOT NULL DEFAULT '',
    redirect_uri       text NOT NULL,
    enc_code_verifier  bytea,                       -- PKCE verifier, encrypted (NULL when the provider has no PKCE)
    enc_byo_secret     bytea,                       -- reserved; BYO client secrets live in oauth_clients
    byo_client_id      text,                        -- reserved
    status             text NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending','consumed','completed','error')),
    credential_id      uuid,                        -- set when the flow mints a credential
    error              text,
    created_at         timestamptz NOT NULL DEFAULT now(),
    expires_at         timestamptz NOT NULL
);

CREATE INDEX oauth_flows_tenant_idx  ON oauth_flows (tenant_id);
CREATE INDEX oauth_flows_expires_idx ON oauth_flows (expires_at);
