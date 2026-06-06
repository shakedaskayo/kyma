---
title: Environment variables
description: Every KYMA_* environment variable the binary reads, grouped by area — catalog, storage, HTTP, gRPC, OTLP, auth, staging, compaction, connectors, agent.
---

# Environment variables

Every `KYMA_*` variable the engine reads at startup or during a
worker's config-from-env hook. Defaults are the values the binary
falls back to when the variable is absent. Areas group related
flags so a deploy config stays coherent.

Most env vars also have CLI-flag equivalents on `kyma-bin`. The
CLI flag wins over the env var, which wins over the default.

## Catalog

| Name                      | Default                                              | Purpose                                                                            |
| ------------------------- | ---------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `KYMA_CATALOG_URL`        | `postgres://kyma:kyma_dev@localhost:5433/kyma`       | Postgres connection URL for the catalog. Used by `kyma-bin` and `kyma-cli` alike.  |

## HTTP server

| Name                      | Default          | Purpose                                                  |
| ------------------------- | ---------------- | -------------------------------------------------------- |
| `KYMA_HTTP_ADDR`          | `0.0.0.0:8080`   | HTTP listen address. Serves query, ingest, agent, etc.   |
| `KYMA_SCHEMA_CACHE_TTL_SECS` | `5`           | TTL for the `GET /v1/catalog/schema` server-side cache.  |

## gRPC (Arrow Flight)

| Name             | Default          | Purpose                                                                                |
| ---------------- | ---------------- | -------------------------------------------------------------------------------------- |
| `KYMA_GRPC_ADDR` | `0.0.0.0:9090`   | Arrow Flight listen address. Set to `off` to disable the gRPC server entirely.         |

## OTLP gRPC

| Name                  | Default     | Purpose                                                                              |
| --------------------- | ----------- | ------------------------------------------------------------------------------------ |
| `KYMA_OTLP_ADDR`      | `off`       | OTLP gRPC listen address (conventional `0.0.0.0:4317`). `off` disables the receiver. |
| `KYMA_OTLP_DATABASE`  | `default`   | Target database for OTLP-received logs. Tables auto-created as `otel_logs`.          |

## Object store

S3-compatible (works with MinIO, AWS S3, R2, GCS via S3 API).

| Name                          | Default       | Purpose                                                                                |
| ----------------------------- | ------------- | -------------------------------------------------------------------------------------- |
| `KYMA_PATH_PREFIX`            | `kyma`        | Path prefix prepended to every object key.                                             |
| `KYMA_S3_ENDPOINT`            | _unset_       | S3 endpoint URL (e.g. `http://localhost:9000` for MinIO). Empty → AWS S3.              |
| `KYMA_S3_REGION`              | `us-east-1`   | Region passed to the S3 client.                                                        |
| `KYMA_S3_BUCKET`              | `kyma`        | Bucket name.                                                                           |
| `KYMA_S3_ACCESS_KEY_ID`       | _unset_       | Access key. Falls back to the AWS credential chain when unset.                         |
| `KYMA_S3_SECRET_ACCESS_KEY`   | _unset_       | Secret key. When both key vars are unset, the standard AWS provider chain applies (ECS/Fargate task role, web identity, `AWS_*` env, IMDS) — keyless IAM-role deployments set only bucket + region. |
| `KYMA_S3_PATH_STYLE`          | `true`        | Path-style addressing (vs. virtual-hosted). Set `false` for virtual-host style.        |
| `KYMA_S3_ALLOW_HTTP`          | `true`        | Allow plain HTTP (for local MinIO). Set `false` to require TLS.                        |

## Auth

| Name                | Default       | Purpose                                                                                                  |
| ------------------- | ------------- | -------------------------------------------------------------------------------------------------------- |
| `KYMA_AUTH_BACKEND` | `env`         | `env` = static `KYMA_AUTH_TOKENS`; `session` = username/password login (`POST /v1/auth/login`) backed by the catalog; `supabase` = Supabase Auth JWTs (see below). |
| `KYMA_AUTH_TOKENS`  | _unset_       | (env backend) Comma-separated `token:role` pairs (e.g. `alice-tok:admin,reader-tok:read`). Empty / unset disables auth. |
| `KYMA_ADMIN_USER`   | _unset_       | (session backend) Seed an admin user on first boot if no users exist yet. Requires `KYMA_ADMIN_PASSWORD`. |
| `KYMA_ADMIN_PASSWORD` | _unset_     | Password for the seeded admin user. Only used when no users exist.                                       |

Roles: `read` ⊆ `write` ⊆ `admin`. A higher role grants everything below it.

### Supabase Auth (`KYMA_AUTH_BACKEND=supabase`)

Validates Supabase-issued JWTs (project JWKS by default, with `kid`-rotation
refetch) and JIT-provisions kyma users from them. Opaque tokens (static keys,
kyma session/API tokens) still authenticate through the session backend, so
CLI/MCP/CI clients keep working. Used by the [production deployment](/deploy/).

| Name                          | Default         | Purpose                                                                                          |
| ----------------------------- | --------------- | ------------------------------------------------------------------------------------------------ |
| `KYMA_SUPABASE_URL`           | _required_      | Project base URL, e.g. `https://<ref>.supabase.co`. JWKS is fetched under it.                    |
| `KYMA_SUPABASE_ANON_KEY`      | _unset_         | Publishable anon key, served to the web login page via `GET /v1/auth/config`.                    |
| `KYMA_SUPABASE_JWT_SECRET`    | _unset_         | Legacy HS256 shared secret. Unset → asymmetric JWKS validation (preferred).                      |
| `KYMA_SUPABASE_JWT_AUD`       | `authenticated` | Expected `aud` claim.                                                                            |
| `KYMA_SUPABASE_PROVIDERS`     | _unset_         | OAuth buttons to render on the login page (e.g. `google,github`); enable them in Supabase too.   |
| `KYMA_ADMIN_EMAILS`           | _unset_         | Comma-separated emails granted the kyma `admin` role on sign-in.                                  |
| `KYMA_ALLOWED_EMAIL_DOMAINS`  | _unset_         | When set, only these email domains may sign in — guards projects with open signup.               |
| `KYMA_SUPABASE_DEFAULT_ROLE`  | `read`          | Role for users matching neither the admin list nor an `app_metadata.role` claim.                 |

## Ingest staging (group commit)

| Name                       | Default        | Purpose                                                                                  |
| -------------------------- | -------------- | ---------------------------------------------------------------------------------------- |
| `KYMA_STAGING_DISABLED`    | _unset_ (off)  | Set to `1` or `true` to disable group-commit. Each ingest request becomes one extent.    |
| `KYMA_FLUSH_MAX_ROWS`      | `8000`         | Per-table flush trigger — row count.                                                     |
| `KYMA_FLUSH_MAX_BYTES`     | `16777216`     | Per-table flush trigger — bytes (16 MiB).                                                |
| `KYMA_FLUSH_MAX_AGE_MS`    | `50`           | Per-table flush trigger — wall-clock age.                                                |
| `KYMA_COMMIT_WINDOW_MS`    | `5`            | Commit-coordinator window. Flushes within this window land in one snapshot.              |
| `KYMA_COMMIT_MAX_EXTENTS`  | `128`          | Maximum extents the coordinator collapses into a single snapshot.                        |

## Compaction, retention, GC

| Name                              | Default | Purpose                                                                       |
| --------------------------------- | ------- | ----------------------------------------------------------------------------- |
| `KYMA_COMPACTION_IDLE_SLEEP_MS`   | _bin default_ | Sleep between work polls when the compaction queue is empty.            |
| `KYMA_COMPACTION_POLL_SECS`       | _bin default_ | Scheduler poll interval (seconds).                                      |
| `KYMA_COMPACTION_MIN_EXTENTS`     | _bin default_ | Minimum extent count per `(table, time-bucket)` before compaction fires. |
| `KYMA_RETENTION_POLL_SECS`        | _bin default_ | Retention sweeper poll interval (seconds).                              |
| `KYMA_PHYSICAL_GC_POLL_SECS`      | _bin default_ | Physical-delete worker poll interval (seconds).                         |
| `KYMA_PHYSICAL_GC_GRACE_SECS`     | _bin default_ | Grace period between soft-delete and hard-delete (seconds).             |

## File-drop

| Name                                | Default     | Purpose                                                                                |
| ----------------------------------- | ----------- | -------------------------------------------------------------------------------------- |
| `KYMA_FILEDROP_ENABLED`             | _unset_     | `1` or `true` enables the file-drop watcher.                                            |
| `KYMA_FILEDROP_PREFIXES`            | `ingest`    | Comma-separated object-store prefixes to watch.                                         |
| `KYMA_FILEDROP_PREFIX`              | _unset_     | Legacy single-prefix form. `KYMA_FILEDROP_PREFIXES` wins when both are set.             |
| `KYMA_FILEDROP_POLL_SECS`           | `5`         | Watcher poll interval.                                                                  |
| `KYMA_FILEDROP_DELETE_AFTER_INGEST` | `false`     | Delete the source object after it commits. Off by default — replays remain idempotent.  |
| `KYMA_FILEDROP_AUTO_CREATE`         | `true`      | Create the database + table on first sight.                                             |
| `KYMA_FILEDROP_SCHEMA_EVOLVE`       | `true`      | Add new columns mid-batch.                                                              |

## Kafka

| Name                          | Default          | Purpose                                                                  |
| ----------------------------- | ---------------- | ------------------------------------------------------------------------ |
| `KYMA_KAFKA_ENABLED`          | _unset_          | `1` or `true` enables the Kafka consumer.                                |
| `KYMA_KAFKA_BROKERS`          | `localhost:9092` | Comma-separated broker list.                                             |
| `KYMA_KAFKA_GROUP`            | `kyma-ingest`    | Consumer group id.                                                       |
| `KYMA_KAFKA_TOPICS`           | _unset_          | Comma-separated `topic:database.table` mappings. Required to enable.     |
| `KYMA_KAFKA_BATCH_SIZE`       | `500`            | Per-batch row count.                                                     |
| `KYMA_KAFKA_BATCH_TIMEOUT_MS` | `500`            | Per-batch wall-clock timeout.                                            |

## Connectors

| Name                       | Default | Purpose                                                                  |
| -------------------------- | ------- | ------------------------------------------------------------------------ |
| `KYMA_CONNECTOR_WORKERS`   | `4`     | Number of connector-runner tasks pulling work off the connector queue.   |

Per-connector secrets are resolved through the `EnvSecretStore` —
config values written as `$env:VAR_NAME` resolve to whatever
`std::env::var("VAR_NAME")` returns at fetch time.

## Credentials & OAuth

The typed credentials store (PATs, OAuth tokens, connection URLs) is encrypted
at rest; the [OAuth connect flow](/connectors/oauth) needs a public origin and a
client app per provider.

| Name                          | Default                 | Purpose                                                                                              |
| ----------------------------- | ----------------------- | ---------------------------------------------------------------------------------------------------- |
| `KYMA_SECRET_KEY`             | _unset_ (required for credentials) | AES-256-GCM key for the credentials store. Base64 of 32 random bytes (`openssl rand -base64 32`); shorter values are SHA-256 stretched (dev only). |
| `KYMA_OAUTH_REDIRECT_BASE`    | `http://localhost:8080` | Externally reachable origin. Builds the provider `redirect_uri` (`<base>/v1/oauth/<provider>/callback`). |
| `KYMA_OAUTH_UI_RETURN_BASE`   | = redirect base         | Where the callback sends the browser back to (the web UI origin).                                    |
| `KYMA_OAUTH_<PROVIDER>_CLIENT_ID` | _unset_             | Operator client id. `<PROVIDER>` ∈ `GOOGLE`, `NOTION`, `ATLASSIAN`, `SLACK`.                          |
| `KYMA_OAUTH_<PROVIDER>_CLIENT_SECRET` | _unset_         | Operator client secret. A per-tenant bring-your-own app (set in the UI) takes precedence over these.  |

## Agent

| Name                          | Default                  | Purpose                                                       |
| ----------------------------- | ------------------------ | ------------------------------------------------------------- |
| `KYMA_AGENT_OLLAMA_HOST`      | `http://localhost:11434` | Ollama host the agent talks to.                               |
| `KYMA_AGENT_MODEL`            | _bin default_            | Ollama model id (e.g. `llama3.2`). Persisted into `agent_runs.model_id`. |

## Embeddings

For dense-vector ingestion + nearest-neighbour scan. Selects the
embedding backend at startup.

| Name                     | Default                                 | Purpose                                                                        |
| ------------------------ | --------------------------------------- | ------------------------------------------------------------------------------ |
| `KYMA_EMBED_PROVIDER`    | `fastembed`                             | One of `fastembed`, `ollama`, `openai-compat`, `gemini`.                       |
| `KYMA_EMBED_MODEL_ID`    | provider-specific (e.g. `bge-small-en-v1.5`) | Model id passed to the provider.                                          |
| `KYMA_EMBED_BASE_URL`    | provider-specific                       | Base URL for HTTP-based providers (Ollama, OpenAI-compat).                     |
| `KYMA_EMBED_MODEL_PATH`  | _unset_                                 | Local model path for `fastembed` (overrides id).                               |
| `KYMA_EMBED_API_KEY_ENV` | `OPENAI_API_KEY`                        | Env var to read the API key from (for `openai-compat`).                        |

`gemini` reads the API key from `GOOGLE_API_KEY` (fixed).

## Logging

`tracing-subscriber` reads the standard `RUST_LOG` env var. The
default filter is `info,sqlx=warn,hyper=warn,h2=warn`.
