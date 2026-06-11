---
title: Environment variables
description: Every KYMA_* environment variable the binary reads, grouped by area — catalog, storage, HTTP, gRPC, OTLP, auth, staging, compaction, data sources, agent, local engine, node daemon, memory, cloud sync.
---

# Environment variables

Every `KYMA_*` variable read by the kyma engine (`kyma-bin`), the
local CLI engine (`kyma mcp` / `kyma serve`), or the node daemon
(`kyma worker run`). Defaults are the values the binary falls back
to when the variable is absent. Areas group related flags so a
deploy config stays coherent.

**Side legend**: **S** = server / engine only; **C** = CLI / local
engine / node daemon; **SC** = both.

## Catalog

| Name                      | Side | Default                                              | Purpose                                                                            |
| ------------------------- | ---- | ---------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `KYMA_CATALOG_URL`        | SC   | `postgres://kyma:kyma_dev@localhost:5433/kyma`       | Postgres connection URL for the catalog. Used by `kyma-bin` and admin subcommands. |

## HTTP server

| Name                         | Side | Default                | Purpose                                                  |
| ---------------------------- | ---- | ---------------------- | -------------------------------------------------------- |
| `KYMA_HTTP_ADDR`             | S    | `0.0.0.0:8080`         | HTTP listen address. Serves query, ingest, agent, etc.   |
| `KYMA_LOCAL_HTTP_ADDR`       | C    | `127.0.0.1:7777`       | Listen address for `kyma serve` (local single-binary).   |
| `KYMA_SCHEMA_CACHE_TTL_SECS` | S    | `5`                    | TTL for the `GET /v1/catalog/schema` server-side cache.  |
| `KYMA_CORS_ALLOWED_ORIGINS`  | S    | _permissive (dev)_     | Comma-separated allowed origins for CORS. Unset → permissive (dev only); always set in production. |

## gRPC (Arrow Flight)

| Name             | Side | Default          | Purpose                                                                                |
| ---------------- | ---- | ---------------- | -------------------------------------------------------------------------------------- |
| `KYMA_GRPC_ADDR` | S    | `0.0.0.0:9090`   | Arrow Flight listen address. Set to `off` to disable the gRPC server entirely.         |

## OTLP gRPC

| Name                  | Side | Default     | Purpose                                                                              |
| --------------------- | ---- | ----------- | ------------------------------------------------------------------------------------ |
| `KYMA_OTLP_ADDR`      | S    | `off`       | OTLP gRPC listen address (conventional `0.0.0.0:4317`). `off` disables the receiver. |
| `KYMA_OTLP_DATABASE`  | S    | `default`   | Target database for OTLP-received logs. Tables auto-created as `otel_logs`.          |

## Object store

S3-compatible (works with MinIO, AWS S3, R2, GCS via S3 API). When
`KYMA_LOCAL_MODE=1` or no S3 env vars are set, the local filesystem
is used instead (set automatically by `kyma serve`).

| Name                          | Side | Default       | Purpose                                                                                |
| ----------------------------- | ---- | ------------- | -------------------------------------------------------------------------------------- |
| `KYMA_PATH_PREFIX`            | S    | `kyma`        | Path prefix prepended to every object key.                                             |
| `KYMA_LOCAL_MODE`             | SC   | _unset_       | `1` forces the local filesystem store (auto-set by `kyma serve`).                     |
| `KYMA_S3_ENDPOINT`            | S    | _unset_       | S3 endpoint URL (e.g. `http://localhost:9000` for MinIO). Empty → AWS S3.              |
| `KYMA_S3_REGION`              | S    | `us-east-1`   | Region passed to the S3 client.                                                        |
| `KYMA_S3_BUCKET`              | S    | `kyma`        | Bucket name.                                                                           |
| `KYMA_S3_ACCESS_KEY_ID`       | S    | _unset_       | Access key. Falls back to the AWS credential chain when unset.                         |
| `KYMA_S3_SECRET_ACCESS_KEY`   | S    | _unset_       | Secret key. When both key vars are unset, the standard AWS provider chain applies (ECS/Fargate task role, web identity, `AWS_*` env, IMDS) — keyless IAM-role deployments set only bucket + region. |
| `KYMA_S3_PATH_STYLE`          | S    | `true`        | Path-style addressing (vs. virtual-hosted). Set `false` for virtual-host style.        |
| `KYMA_S3_ALLOW_HTTP`          | S    | `true`        | Allow plain HTTP (for local MinIO). Set `false` to require TLS.                        |

## Local engine data paths

Read by `kyma mcp`, `kyma serve`, and `kyma sync`. Data lives under
`~/.kyma` by default.

| Name               | Side | Default              | Purpose                                                              |
| ------------------ | ---- | -------------------- | -------------------------------------------------------------------- |
| `KYMA_HOME`        | C    | `~/.kyma`            | Root data directory for the local engine.                            |
| `KYMA_LOCAL_DB`    | C    | `$KYMA_HOME/catalog.db` | SQLite catalog path (local engine only).                          |
| `KYMA_LOCAL_DATA`  | C    | `$KYMA_HOME/data`    | Local object-store root directory.                                   |
| `KYMA_LOCAL_USER`  | C    | `admin`              | Default admin username for `kyma serve`.                             |
| `KYMA_LOCAL_PASSWORD` | C | `admin`              | Default admin password for `kyma serve`. Override in shared envs.    |

## Auth

| Name                    | Side | Default       | Purpose                                                                                                  |
| ----------------------- | ---- | ------------- | -------------------------------------------------------------------------------------------------------- |
| `KYMA_AUTH_BACKEND`     | S    | `env`         | `env` = static `KYMA_AUTH_TOKENS`; `session` = username/password login backed by the catalog; `supabase` = Supabase Auth JWTs; `oidc` = generic OIDC. |
| `KYMA_AUTH_TOKENS`      | S    | _unset_       | (env backend) Comma-separated `token:role` pairs (e.g. `alice-tok:admin,reader-tok:read`). Empty / unset disables auth. |
| `KYMA_ADMIN_USER`       | S    | _unset_       | (session backend) Seed an admin user on first boot if no users exist yet. Requires `KYMA_ADMIN_PASSWORD`. |
| `KYMA_ADMIN_PASSWORD`   | S    | _unset_       | Password for the seeded admin user. Only used when no users exist.                                       |
| `KYMA_ACCESS_TTL_SECS`  | S    | `3600`        | Access-token lifetime (1 hour). Applies to the session and supabase backends.                            |
| `KYMA_REFRESH_TTL_SECS` | S    | `2592000`     | Refresh-token lifetime (30 days).                                                                        |
| `KYMA_INTERNAL_BEARER`  | SC   | _auto_        | Bearer token used by the local engine to call itself (e.g. dreaming). Auto-derived from `KYMA_AUTH_TOKENS`; override only when embedding the server. |

Roles: `read` ⊆ `write` ⊆ `admin`. A higher role grants everything below it.

### Supabase Auth (`KYMA_AUTH_BACKEND=supabase`)

Validates Supabase-issued JWTs (project JWKS by default, with `kid`-rotation
refetch) and JIT-provisions kyma users from them. Opaque tokens (static keys,
kyma session/API tokens) still authenticate through the session backend, so
CLI/MCP/CI clients keep working. Used by the [production deployment](/deploy/).

| Name                          | Side | Default         | Purpose                                                                                          |
| ----------------------------- | ---- | --------------- | ------------------------------------------------------------------------------------------------ |
| `KYMA_SUPABASE_URL`           | S    | _required_      | Project base URL, e.g. `https://<ref>.supabase.co`. JWKS is fetched under it.                    |
| `KYMA_SUPABASE_ANON_KEY`      | S    | _unset_         | Publishable anon key, served to the web login page via `GET /v1/auth/config`.                    |
| `KYMA_SUPABASE_JWT_SECRET`    | S    | _unset_         | Legacy HS256 shared secret. Unset → asymmetric JWKS validation (preferred).                      |
| `KYMA_SUPABASE_JWT_AUD`       | S    | `authenticated` | Expected `aud` claim.                                                                            |
| `KYMA_SUPABASE_PROVIDERS`     | S    | _unset_         | OAuth buttons to render on the login page (e.g. `google,github`); enable them in Supabase too.   |
| `KYMA_ADMIN_EMAILS`           | S    | _unset_         | Comma-separated emails granted the kyma `admin` role on sign-in.                                  |
| `KYMA_ALLOWED_EMAIL_DOMAINS`  | S    | _unset_         | When set, only these email domains may sign in — guards projects with open signup.               |
| `KYMA_SUPABASE_DEFAULT_ROLE`  | S    | `read`          | Role for users matching neither the admin list nor an `app_metadata.role` claim.                 |
| `KYMA_SUPABASE_API_BASE`      | C    | `https://api.supabase.com` | Supabase Management API base URL. Override in tests or private deployments. Used by `kyma deploy`. |
| `KYMA_SUPABASE_OAUTH_CLIENT_ID` | C  | _unset_         | OAuth app client id for the `kyma deploy` browser OAuth flow against Supabase.                   |
| `KYMA_SUPABASE_OAUTH_CLIENT_SECRET` | C | _unset_     | OAuth app client secret for the `kyma deploy` browser OAuth flow.                               |

### OIDC backend (`KYMA_AUTH_BACKEND=oidc`)

Validates JWTs from any standards-compliant OIDC issuer (Okta, Auth0,
Keycloak, etc.) and maps claims to kyma roles.

| Name                          | Side | Default    | Purpose                                                                               |
| ----------------------------- | ---- | ---------- | ------------------------------------------------------------------------------------- |
| `KYMA_OIDC_ISSUERS`           | S    | _unset_    | Comma-separated issuer URLs. Unset / empty disables OIDC.                             |
| `KYMA_OIDC_AUDIENCE`          | S    | `kyma`     | Required `aud` claim value.                                                           |
| `KYMA_OIDC_ROLE_CLAIM`        | S    | `kyma_role` | JWT claim to map to the kyma role.                                                   |
| `KYMA_OIDC_SUBJECT_CLAIM`     | S    | `sub`      | JWT claim to use as the kyma username.                                                |
| `KYMA_OIDC_DATABASES_CLAIM`   | S    | `kyma_databases` | JWT claim containing an allowlist of database names this token may access.      |

## Ingest staging (group commit)

| Name                       | Side | Default        | Purpose                                                                                  |
| -------------------------- | ---- | -------------- | ---------------------------------------------------------------------------------------- |
| `KYMA_STAGING_DISABLED`    | S    | _unset_ (off)  | Set to `1` or `true` to disable group-commit. Each ingest request becomes one extent.    |
| `KYMA_FLUSH_MAX_ROWS`      | S    | `8000`         | Per-table flush trigger — row count.                                                     |
| `KYMA_FLUSH_MAX_BYTES`     | S    | `16777216`     | Per-table flush trigger — bytes (16 MiB).                                                |
| `KYMA_FLUSH_MAX_AGE_MS`    | S    | `50`           | Per-table flush trigger — wall-clock age.                                                |
| `KYMA_COMMIT_WINDOW_MS`    | S    | `5`            | Commit-coordinator window. Flushes within this window land in one snapshot.              |
| `KYMA_COMMIT_MAX_EXTENTS`  | S    | `128`          | Maximum extents the coordinator collapses into a single snapshot.                        |

## Compaction, retention, GC

| Name                              | Side | Default       | Purpose                                                                       |
| --------------------------------- | ---- | ------------- | ----------------------------------------------------------------------------- |
| `KYMA_COMPACTION_IDLE_SLEEP_MS`   | S    | _bin default_ | Sleep between work polls when the compaction queue is empty.                  |
| `KYMA_COMPACTION_POLL_SECS`       | S    | _bin default_ | Scheduler poll interval (seconds).                                            |
| `KYMA_COMPACTION_MIN_EXTENTS`     | S    | _bin default_ | Minimum extent count per `(table, time-bucket)` before compaction fires.      |
| `KYMA_RETENTION_POLL_SECS`        | S    | _bin default_ | Retention sweeper poll interval (seconds).                                    |
| `KYMA_PHYSICAL_GC_POLL_SECS`      | S    | _bin default_ | Physical-delete worker poll interval (seconds).                               |
| `KYMA_PHYSICAL_GC_GRACE_SECS`     | S    | _bin default_ | Grace period between soft-delete and hard-delete (seconds).                   |

## File-drop

| Name                                | Side | Default     | Purpose                                                                                |
| ----------------------------------- | ---- | ----------- | -------------------------------------------------------------------------------------- |
| `KYMA_FILEDROP_ENABLED`             | S    | _unset_     | `1` or `true` enables the file-drop watcher.                                            |
| `KYMA_FILEDROP_PREFIXES`            | S    | `ingest`    | Comma-separated object-store prefixes to watch.                                         |
| `KYMA_FILEDROP_PREFIX`              | S    | _unset_     | Legacy single-prefix form. `KYMA_FILEDROP_PREFIXES` wins when both are set.             |
| `KYMA_FILEDROP_POLL_SECS`           | S    | `5`         | Watcher poll interval.                                                                  |
| `KYMA_FILEDROP_DELETE_AFTER_INGEST` | S    | `false`     | Delete the source object after it commits. Off by default — replays remain idempotent.  |
| `KYMA_FILEDROP_AUTO_CREATE`         | S    | `true`      | Create the database + table on first sight.                                             |
| `KYMA_FILEDROP_SCHEMA_EVOLVE`       | S    | `true`      | Add new columns mid-batch.                                                              |

## Kafka

| Name                          | Side | Default          | Purpose                                                                  |
| ----------------------------- | ---- | ---------------- | ------------------------------------------------------------------------ |
| `KYMA_KAFKA_ENABLED`          | S    | _unset_          | `1` or `true` enables the Kafka consumer.                                |
| `KYMA_KAFKA_BROKERS`          | S    | `localhost:9092` | Comma-separated broker list.                                             |
| `KYMA_KAFKA_GROUP`            | S    | `kyma-ingest`    | Consumer group id.                                                       |
| `KYMA_KAFKA_TOPICS`           | S    | _unset_          | Comma-separated `topic:database.table` mappings. Required to enable.     |
| `KYMA_KAFKA_BATCH_SIZE`       | S    | `500`            | Per-batch row count.                                                     |
| `KYMA_KAFKA_BATCH_TIMEOUT_MS` | S    | `500`            | Per-batch wall-clock timeout.                                            |

## Data sources

| Name                       | Side | Default | Purpose                                                                                              |
| -------------------------- | ---- | ------- | ---------------------------------------------------------------------------------------------------- |
| `KYMA_DATA_SOURCE_WORKERS`   | S    | `4`     | Number of data-source-runner tasks. Also the fallback for `KYMA_FABRIC_WORKERS` when unset.            |
| `KYMA_DISCOVER_MAX_SOURCES` | S   | _bin default_ | Cap on sources returned by the discovery endpoint.                                             |
| `KYMA_GH_PAT`              | S    | _unset_ | GitHub personal-access token injected into `git clone` credentials for private-repo data sources.      |

Per-data-source secrets are resolved through the `EnvSecretStore` —
config values written as `$env:VAR_NAME` resolve to whatever
`std::env::var("VAR_NAME")` returns at fetch time.

## Credentials & OAuth

The typed credentials store (PATs, OAuth tokens, connection URLs) is encrypted
at rest; the [OAuth connect flow](/data-sources/oauth) needs a public origin and a
client app per provider.

| Name                          | Side | Default                 | Purpose                                                                                              |
| ----------------------------- | ---- | ----------------------- | ---------------------------------------------------------------------------------------------------- |
| `KYMA_SECRET_KEY`             | S    | _unset_ (required for credentials) | AES-256-GCM key for the credentials store. Base64 of 32 random bytes (`openssl rand -base64 32`); shorter values are SHA-256 stretched (dev only). |
| `KYMA_OAUTH_REDIRECT_BASE`    | S    | `http://localhost:8080` | Externally reachable origin. Builds the provider `redirect_uri` (`<base>/v1/oauth/<provider>/callback`). |
| `KYMA_OAUTH_UI_RETURN_BASE`   | S    | = redirect base         | Where the callback sends the browser back to (the web UI origin).                                    |
| `KYMA_OAUTH_<PROVIDER>_CLIENT_ID` | S | _unset_             | Operator client id. `<PROVIDER>` ∈ `GOOGLE`, `NOTION`, `ATLASSIAN`, `SLACK`.                          |
| `KYMA_OAUTH_<PROVIDER>_CLIENT_SECRET` | S | _unset_         | Operator client secret. A per-tenant bring-your-own app (set in the UI) takes precedence over these.  |

## Worker fabric

The server-side job dispatch layer that routes work to registered worker nodes.

| Name                        | Side | Default | Purpose                                                                             |
| --------------------------- | ---- | ------- | ----------------------------------------------------------------------------------- |
| `KYMA_FABRIC_WORKERS`       | S    | `4`     | Goroutine-pool size for the fabric dispatcher. Falls back to `KYMA_DATA_SOURCE_WORKERS`. |
| `KYMA_FABRIC_LEASE_SECS`    | S    | `300`   | How long a job lease is held before the fabric considers it expired (5 minutes).    |
| `KYMA_FABRIC_SWEEP_SECS`    | S    | `30`    | How often the fabric sweeps for expired leases.                                     |
| `KYMA_FABRIC_OFFLINE_SECS`  | S    | `90`    | Seconds without a heartbeat before a node is marked offline.                        |

## Node daemon / local sync

Variables read by `kyma worker run` (the fabric node daemon) and
`kyma sync` / `kyma worker install` (the CC-memory sync service).
These are **client-side** — the running server reads none of them.

| Name                    | Side | Default        | Purpose                                                                                |
| ----------------------- | ---- | -------------- | -------------------------------------------------------------------------------------- |
| `KYMA_SERVER_URL`       | C    | _required_     | Control-plane URL. Required for `kyma worker run`; also overrides the saved client endpoint for all `kyma` subcommands. |
| `KYMA_TOKEN`            | C    | _unset_        | Bearer token override for all client subcommands (wins over `~/.kyma/config.json`).    |
| `KYMA_WORKER_TOKEN`     | C    | _required_     | Node authentication token minted by `kyma worker create`. Required for `kyma worker run`. |
| `KYMA_WORKER_INSECURE`  | C    | _unset_        | Set to `1` to allow HTTP (non-TLS) connections from the node daemon to the server. Tokens ride this channel. |
| `KYMA_CC_SYNC_POLL_SECS` | C   | `30`           | Poll interval (seconds) for `kyma sync --watch` and the background worker service.    |
| `KYMA_CC_WATCH`         | C    | `0` (off)      | Set to `1` to enable the continuous CC-file sync watcher inside `kyma serve`.         |
| `KYMA_CC_FILE_SYNC`     | C    | `1` (on)       | Set to `0` to disable the Claude Code file-memory phase of sync entirely.             |
| `KYMA_CC_SYNC_ON_MCP`   | C    | `1` (on)       | Set to `0` to skip the one-shot file sync that fires when the MCP server initialises. |
| `KYMA_CC_HOME`          | C    | `~/.claude`    | Root directory that kyma treats as the Claude Code home (scans `CLAUDE.md`, `projects/*/memory`). |
| `KYMA_CLOUD_URL`        | C    | _unset_        | URL of the kyma control plane to push/pull memories to. Unset → local-only sync.      |
| `KYMA_CLOUD_TOKEN`      | C    | _unset_        | Bearer token for the cloud control plane.                                              |
| `KYMA_SYNC_REALM`       | C    | _unset_        | Memory realm to sync to/from on the control plane. Unset → server default.            |
| `KYMA_NO_UPDATE_CHECK`  | C    | _unset_        | Set to any value to suppress the background update-available nudge in `kyma status` / `kyma version`. |

## CC-memory curation

Knobs for the Claude Code memory curation pass (promotion and LLM-curated
deduplication / staleness review).

| Name                           | Side | Default | Purpose                                                                                 |
| ------------------------------ | ---- | ------- | --------------------------------------------------------------------------------------- |
| `KYMA_CC_CURATE`               | SC   | `1` (on) | Master switch. Set to `0` to disable all curation.                                     |
| `KYMA_CC_PROMOTE`              | SC   | `1` (on) | Write high-importance memories back as native `.md` files for Claude Code to load.      |
| `KYMA_CC_PROMOTE_MAX`          | SC   | `15`    | Max number of memories promoted to files per project.                                   |
| `KYMA_CC_PROMOTE_MIN_IMPORTANCE` | SC | `0.6`  | Minimum importance score a memory needs to be eligible for promotion.                   |
| `KYMA_CC_QUIET_WINDOW`         | SC   | `300`   | Seconds of inactivity before a promotion pass is allowed (prevents churn during edits). |
| `KYMA_CC_LOCK_TTL`             | C    | `300`   | Seconds before a stale curation lock is forcibly released.                              |
| `KYMA_CC_STALE_DAYS`           | SC   | `90`    | Age in days before a memory is considered stale for LLM review.                         |
| `KYMA_CC_DUP_COSINE`           | SC   | `0.90–0.97 band` | Cosine-similarity range where duplication is plausible but not certain — the model decides. |

## Agentic memory

| Name                              | Side | Default   | Purpose                                                                                    |
| --------------------------------- | ---- | --------- | ------------------------------------------------------------------------------------------ |
| `KYMA_MEMORY_ASYNC`               | SC   | `1` (on)  | `0` makes memory writes synchronous (blocks until the embedding + save complete).          |
| `KYMA_MEMORY_QUEUE_DURABLE`       | SC   | `0` (local) / `1` (server) | `1` persists the in-flight memory queue to the catalog so it survives restarts. Defaults on for the server binary, off for local/stdio.            |
| `KYMA_MEMORY_CONSOLIDATION`       | S    | `1` (on)  | `0` disables the background memory-consolidation worker.                                   |
| `KYMA_MEMORY_CONSOLIDATION_POLL_SECS` | S | _bin default_ | Poll interval for the memory consolidation background task.                           |
| `KYMA_SESSION_SUMMARY_EVERY`      | S    | `12`      | Refresh the rolling session summary every N turns.                                         |

## Agent

| Name                  | Side | Default                          | Purpose                                                                    |
| --------------------- | ---- | -------------------------------- | -------------------------------------------------------------------------- |
| `KYMA_AGENT_MCP_URL`  | S    | _auto (derived from HTTP addr)_  | Override the MCP endpoint the agent routes tool calls to. `off` disables.  |
| `KYMA_OLLAMA_HOST`    | S    | `http://localhost:11434`         | Ollama host the server-side agent uses.                                    |

## Embeddings

For dense-vector ingestion + nearest-neighbour scan. Selects the
embedding backend at startup.

| Name                     | Side | Default                                 | Purpose                                                                        |
| ------------------------ | ---- | --------------------------------------- | ------------------------------------------------------------------------------ |
| `KYMA_EMBED_PROVIDER`    | SC   | `fastembed`                             | One of `fastembed`, `ollama`, `openai-compat`, `gemini`.                       |
| `KYMA_EMBED_MODEL_ID`    | SC   | provider-specific (e.g. `bge-small-en-v1.5`) | Model id passed to the provider.                                          |
| `KYMA_EMBED_BASE_URL`    | SC   | provider-specific                       | Base URL for HTTP-based providers (Ollama, OpenAI-compat).                     |
| `KYMA_EMBED_MODEL_PATH`  | SC   | _unset_                                 | Local model path for `fastembed` (overrides id).                               |
| `KYMA_EMBED_API_KEY_ENV` | SC   | `OPENAI_API_KEY`                        | Env var to read the API key from (for `openai-compat`).                        |

`gemini` reads the API key from `GOOGLE_API_KEY` (fixed).

## UI / icon gallery

| Name                | Side | Default | Purpose                                                                                      |
| ------------------- | ---- | ------- | -------------------------------------------------------------------------------------------- |
| `KYMA_ICON_GALLERY` | S    | _unset_ | Path to a JSON file that extends or overrides the built-in icon gallery (`{"kinds":{...},"vendors":{...}}`). |

## Debug / verbose flags

These are CLI-side flags that increase output verbosity for specific operations.
They are not set in production.

| Name                    | Side | Default | Purpose                                                                    |
| ----------------------- | ---- | ------- | -------------------------------------------------------------------------- |
| `KYMA_INGEST_VERBOSE`   | C    | _unset_ | Print per-row ingest detail from `kyma ingest push`.                       |
| `KYMA_DISTILL_VERBOSE`  | C    | _unset_ | Print distillation progress from `kyma distill`.                           |

## Logging

`tracing-subscriber` reads the standard `RUST_LOG` env var. The
default filter is `info,sqlx=warn,hyper=warn,h2=warn`.
