-- Per-tenant bring-your-own OAuth client apps.
--
-- When an operator hasn't configured a provider's client app via env
-- (KYMA_OAUTH_<PROVIDER>_CLIENT_ID / _CLIENT_SECRET), a user can paste their own
-- client_id / client_secret in the UI when connecting. We persist them here
-- (secret AES-256-GCM encrypted) so later token refreshes — which run with no UI
-- session — can reuse the same client app. A row here takes precedence over the
-- operator env for the same (tenant, provider).

CREATE TABLE oauth_clients (
    tenant_id   uuid NOT NULL,
    provider    text NOT NULL,
    client_id   text NOT NULL,
    enc_secret  bytea NOT NULL,                     -- AES-256-GCM(client_secret)
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, provider)
);
