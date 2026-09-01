---
title: Environment variables
description: Every PENSIEVE_* environment variable the binary reads, grouped by area — catalog, roles & scale-out, query limits & admission control, storage, HTTP, gRPC, OTLP, auth, staging, rate limiting, compaction, data sources, agent, local engine, node daemon, memory, cloud sync.
---

# Environment variables

Every `PENSIEVE_*` variable read by the pensieve engine (`pensieve-bin`), the
local CLI engine (`pensieve mcp` / `pensieve serve`), or the node daemon
(`pensieve worker run`). Defaults are the values the binary falls back
to when the variable is absent. Areas group related flags so a
deploy config stays coherent.

**Side legend**: **S** = server / engine only; **C** = CLI / local
engine / node daemon; **SC** = both.

## Catalog

| Name                      | Side | Default                                              | Purpose                                                                            |
| ------------------------- | ---- | ---------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `PENSIEVE_CATALOG_URL`        | SC   | `postgres://pensieve:pensieve_dev@localhost:5433/pensieve`       | Postgres connection URL for the catalog. Used by `pensieve-bin` and admin subcommands. |
| `PENSIEVE_PG_MAX_CONNS`       | S    | `16`                                                 | Catalog connection-pool size. Raise for many concurrent query/ingest nodes against one catalog. |
| `PENSIEVE_PG_ACQUIRE_TIMEOUT_SECS` | S | `10`                                               | How long a request waits for a free pool connection before erroring.               |

## Roles and scale-out

Knobs for running more than one engine pod against a catalog. By default a node
is `all_in_one` (single-writer); see [Deploy with Helm → role split](/deploy/helm#scaling-out-the-role-split).

| Name                 | Side | Default      | Purpose                                                                                              |
| -------------------- | ---- | ------------ | ---------------------------------------------------------------------------------------------------- |
| `PENSIEVE_ROLE`          | S    | `all_in_one` | Which components this node runs. `all_in_one` (or unset/unknown) = everything; `edge`/`query`/`ingest` = stateless HTTP only (no committer, no background jobs); `committer` = the commit loop only; `worker`/`compaction` = background jobs only. |
| `PENSIEVE_INGEST_MODE`   | S    | _sync_       | Set to `staged` to stage writes to object storage and ack immediately, leaving the commit to the (possibly separate) committer node. Required for the role split to offload commits. |
| `PENSIEVE_WRITE_FORMAT`  | S    | `tlm`        | Segment format for newly written extents: `tlm` (Arrow IPC) or `parquet` (ZSTD + encodings + blooms). Reads dispatch per-extent, so old extents stay readable after a flip; compaction migrates them. |

## HTTP server

| Name                         | Side | Default                | Purpose                                                  |
| ---------------------------- | ---- | ---------------------- | -------------------------------------------------------- |
| `PENSIEVE_HTTP_ADDR`             | S    | `0.0.0.0:8080`         | HTTP listen address. Serves query, ingest, agent, etc.   |
| `PENSIEVE_LOCAL_HTTP_ADDR`       | C    | `127.0.0.1:7777`       | Listen address for `pensieve serve` (local single-binary).   |
| `PENSIEVE_SCHEMA_CACHE_TTL_SECS` | S    | `5`                    | TTL for the `GET /v1/catalog/schema` server-side cache.  |
| `PENSIEVE_CORS_ALLOWED_ORIGINS`  | S    | _permissive (dev)_     | Comma-separated allowed origins for CORS. Unset → permissive (dev only); always set in production. |

## Query limits & admission control

Per-query resource ceilings and node-level backpressure. The budget caps are
deployment-wide defaults; per-request headers may lower them further. Admission
limits are **off by default** (unset/`0` ⇒ unlimited) — set them to convert
overload into a fast `429 Too Many Requests` + `Retry-After` instead of OOM or
latency collapse.

| Name                                  | Side | Default        | Purpose                                                                                |
| ------------------------------------- | ---- | -------------- | -------------------------------------------------------------------------------------- |
| `PENSIEVE_QUERY_MEMORY_BYTES`             | S    | `4294967296` (4 GiB) | Per-query DataFusion memory pool. Over-budget queries spill instead of OOM-ing.   |
| `PENSIEVE_QUERY_WALL_MS`                  | S    | `300000` (5 min) | Per-query wall-clock deadline (ms).                                                  |
| `PENSIEVE_QUERY_OBJECT_STORE_BYTES`       | S    | `10737418240` (10 GiB) | Per-query cap on bytes fetched from object storage.                            |
| `PENSIEVE_QUERY_MAX_CONCURRENT`           | S    | `0` (unlimited) | Max in-flight queries on this node. Excess → `429` + `Retry-After`.                   |
| `PENSIEVE_QUERY_RETRY_AFTER_SECS`         | S    | `1`            | `Retry-After` advertised when query admission rejects.                                 |
| `PENSIEVE_QUERY_MAX_CONCURRENT_PER_TENANT`| S    | `0` (unlimited) | Per-tenant in-flight query cap — one tenant saturating its budget can't starve others. |
| `PENSIEVE_AGENT_MAX_CONCURRENT`           | S    | `0` (unlimited) | Max concurrent agent runs on this node.                                               |
| `PENSIEVE_AGENT_RETRY_AFTER_SECS`         | S    | `1`            | `Retry-After` advertised when agent-run admission rejects.                             |
| `PENSIEVE_AGENT_MAX_CONCURRENT_PER_TENANT`| S    | `0` (unlimited) | Per-tenant concurrent agent-run cap.                                                   |

## gRPC (Arrow Flight)

| Name             | Side | Default          | Purpose                                                                                |
| ---------------- | ---- | ---------------- | -------------------------------------------------------------------------------------- |
| `PENSIEVE_GRPC_ADDR` | S    | `0.0.0.0:9090`   | Arrow Flight listen address. Set to `off` to disable the gRPC server entirely.         |

## OTLP gRPC

| Name                  | Side | Default     | Purpose                                                                              |
| --------------------- | ---- | ----------- | ------------------------------------------------------------------------------------ |
| `PENSIEVE_OTLP_ADDR`      | S    | `off`       | OTLP gRPC listen address (conventional `0.0.0.0:4317`). `off` disables the receiver. |
| `PENSIEVE_OTLP_DATABASE`  | S    | `default`   | Target database for OTLP-received logs. Tables auto-created as `otel_logs`.          |

## Object store

S3-compatible (works with MinIO, AWS S3, R2, GCS via S3 API). When
`PENSIEVE_LOCAL_MODE=1` or no S3 env vars are set, the local filesystem
is used instead (set automatically by `pensieve serve`).

| Name                          | Side | Default       | Purpose                                                                                |
| ----------------------------- | ---- | ------------- | -------------------------------------------------------------------------------------- |
| `PENSIEVE_PATH_PREFIX`            | S    | `pensieve`        | Path prefix prepended to every object key.                                             |
| `PENSIEVE_LOCAL_MODE`             | SC   | _unset_       | `1` forces the local filesystem store (auto-set by `pensieve serve`).                     |
| `PENSIEVE_S3_ENDPOINT`            | S    | _unset_       | S3 endpoint URL (e.g. `http://localhost:9000` for MinIO). Empty → AWS S3.              |
| `PENSIEVE_S3_REGION`              | S    | `us-east-1`   | Region passed to the S3 client.                                                        |
| `PENSIEVE_S3_BUCKET`              | S    | `pensieve`        | Bucket name.                                                                           |
| `PENSIEVE_S3_ACCESS_KEY_ID`       | S    | _unset_       | Access key. Falls back to the AWS credential chain when unset.                         |
| `PENSIEVE_S3_SECRET_ACCESS_KEY`   | S    | _unset_       | Secret key. When both key vars are unset, the standard AWS provider chain applies (ECS/Fargate task role, web identity, `AWS_*` env, IMDS) — keyless IAM-role deployments set only bucket + region. |
| `PENSIEVE_S3_PATH_STYLE`          | S    | `true`        | Path-style addressing (vs. virtual-hosted). Set `false` for virtual-host style.        |
| `PENSIEVE_S3_ALLOW_HTTP`          | S    | `true`        | Allow plain HTTP (for local MinIO). Set `false` to require TLS.                        |

## Local engine data paths

Read by `pensieve mcp`, `pensieve serve`, and `pensieve sync`. Data lives under
`~/.pensieve` by default.

| Name               | Side | Default              | Purpose                                                              |
| ------------------ | ---- | -------------------- | -------------------------------------------------------------------- |
| `PENSIEVE_HOME`        | C    | `~/.pensieve`            | Root data directory for the local engine.                            |
| `PENSIEVE_LOCAL_DB`    | C    | `$PENSIEVE_HOME/catalog.db` | SQLite catalog path (local engine only).                          |
| `PENSIEVE_LOCAL_DATA`  | C    | `$PENSIEVE_HOME/data`    | Local object-store root directory.                                   |
| `PENSIEVE_LOCAL_USER`  | C    | `admin`              | Default admin username for `pensieve serve`.                             |
| `PENSIEVE_LOCAL_PASSWORD` | C | `admin`              | Default admin password for `pensieve serve`. Override in shared envs.    |

## Auth

| Name                    | Side | Default       | Purpose                                                                                                  |
| ----------------------- | ---- | ------------- | -------------------------------------------------------------------------------------------------------- |
| `PENSIEVE_AUTH_BACKEND`     | S    | `env`         | `env` = static `PENSIEVE_AUTH_TOKENS`; `session` = username/password login backed by the catalog; `supabase` = Supabase Auth JWTs; `oidc` = generic OIDC. |
| `PENSIEVE_AUTH_TOKENS`      | S    | _unset_       | (env backend) Comma-separated `token:role` pairs (e.g. `alice-tok:admin,reader-tok:read`). Empty / unset disables auth. |
| `PENSIEVE_ADMIN_USER`       | S    | _unset_       | (session backend) Seed an admin user on first boot if no users exist yet. Requires `PENSIEVE_ADMIN_PASSWORD`. |
| `PENSIEVE_ADMIN_PASSWORD`   | S    | _unset_       | Password for the seeded admin user. Only used when no users exist.                                       |
| `PENSIEVE_ACCESS_TTL_SECS`  | S    | `3600`        | Access-token lifetime (1 hour). Applies to the session and supabase backends.                            |
| `PENSIEVE_REFRESH_TTL_SECS` | S    | `2592000`     | Refresh-token lifetime (30 days).                                                                        |
| `PENSIEVE_INTERNAL_BEARER`  | SC   | _auto_        | Bearer token used by the local engine to call itself (e.g. dreaming). Auto-derived from `PENSIEVE_AUTH_TOKENS`; override only when embedding the server. |

Roles: `read` ⊆ `write` ⊆ `admin`. A higher role grants everything below it.

### Supabase Auth (`PENSIEVE_AUTH_BACKEND=supabase`)

Validates Supabase-issued JWTs (project JWKS by default, with `kid`-rotation
refetch) and JIT-provisions pensieve users from them. Opaque tokens (static keys,
pensieve session/API tokens) still authenticate through the session backend, so
CLI/MCP/CI clients keep working. Used by the [production deployment](/deploy/).

| Name                          | Side | Default         | Purpose                                                                                          |
| ----------------------------- | ---- | --------------- | ------------------------------------------------------------------------------------------------ |
| `PENSIEVE_SUPABASE_URL`           | S    | _required_      | Project base URL, e.g. `https://<ref>.supabase.co`. JWKS is fetched under it.                    |
| `PENSIEVE_SUPABASE_ANON_KEY`      | S    | _unset_         | Publishable anon key, served to the web login page via `GET /v1/auth/config`.                    |
| `PENSIEVE_SUPABASE_JWT_SECRET`    | S    | _unset_         | Legacy HS256 shared secret. Unset → asymmetric JWKS validation (preferred).                      |
| `PENSIEVE_SUPABASE_JWT_AUD`       | S    | `authenticated` | Expected `aud` claim.                                                                            |
| `PENSIEVE_SUPABASE_PROVIDERS`     | S    | _unset_         | OAuth buttons to render on the login page (e.g. `google,github`); enable them in Supabase too.   |
| `PENSIEVE_ADMIN_EMAILS`           | S    | _unset_         | Comma-separated emails granted the pensieve `admin` role on sign-in.                                  |
| `PENSIEVE_ALLOWED_EMAIL_DOMAINS`  | S    | _unset_         | When set, only these email domains may sign in — guards projects with open signup.               |
| `PENSIEVE_SUPABASE_DEFAULT_ROLE`  | S    | `read`          | Role for users matching neither the admin list nor an `app_metadata.role` claim.                 |
| `PENSIEVE_SUPABASE_API_BASE`      | C    | `https://api.supabase.com` | Supabase Management API base URL. Override in tests or private deployments. Used by `pensieve deploy`. |
| `PENSIEVE_SUPABASE_OAUTH_CLIENT_ID` | C  | _unset_         | OAuth app client id for the `pensieve deploy` browser OAuth flow against Supabase.                   |
| `PENSIEVE_SUPABASE_OAUTH_CLIENT_SECRET` | C | _unset_     | OAuth app client secret for the `pensieve deploy` browser OAuth flow.                               |

### OIDC backend (`PENSIEVE_AUTH_BACKEND=oidc`)

Validates JWTs from any standards-compliant OIDC issuer (Okta, Auth0,
Keycloak, etc.) and maps claims to pensieve roles.

| Name                          | Side | Default    | Purpose                                                                               |
| ----------------------------- | ---- | ---------- | ------------------------------------------------------------------------------------- |
| `PENSIEVE_OIDC_ISSUERS`           | S    | _unset_    | Comma-separated issuer URLs. Unset / empty disables OIDC.                             |
| `PENSIEVE_OIDC_AUDIENCE`          | S    | `pensieve`     | Required `aud` claim value.                                                           |
| `PENSIEVE_OIDC_ROLE_CLAIM`        | S    | `pensieve_role` | JWT claim to map to the pensieve role.                                                   |
| `PENSIEVE_OIDC_SUBJECT_CLAIM`     | S    | `sub`      | JWT claim to use as the pensieve username.                                                |
| `PENSIEVE_OIDC_DATABASES_CLAIM`   | S    | `pensieve_databases` | JWT claim containing an allowlist of database names this token may access.      |

## Ingest staging (group commit)

| Name                       | Side | Default        | Purpose                                                                                  |
| -------------------------- | ---- | -------------- | ---------------------------------------------------------------------------------------- |
| `PENSIEVE_STAGING_DISABLED`    | S    | _unset_ (off)  | Set to `1` or `true` to disable group-commit. Each ingest request becomes one extent.    |
| `PENSIEVE_FLUSH_MAX_ROWS`      | S    | `8000`         | Per-table flush trigger — row count.                                                     |
| `PENSIEVE_FLUSH_MAX_BYTES`     | S    | `16777216`     | Per-table flush trigger — bytes (16 MiB).                                                |
| `PENSIEVE_FLUSH_MAX_AGE_MS`    | S    | `50`           | Per-table flush trigger — wall-clock age.                                                |
| `PENSIEVE_COMMIT_WINDOW_MS`    | S    | `5`            | Commit-coordinator window. Flushes within this window land in one snapshot.              |
| `PENSIEVE_COMMIT_MAX_EXTENTS`  | S    | `128`          | Maximum extents the coordinator collapses into a single snapshot.                        |

### Ingest rate limiting

Per-database token bucket on the ingest path. Off by default — set the rate to
turn ingest overload into a `429` + `Retry-After` instead of unbounded queueing.

| Name                     | Side | Default        | Purpose                                                                                  |
| ------------------------ | ---- | -------------- | ---------------------------------------------------------------------------------------- |
| `PENSIEVE_INGEST_RATE_RPS`   | S    | `0` (off)      | Steady-state requests/sec per database. `0` or unset ⇒ unlimited.                        |
| `PENSIEVE_INGEST_RATE_BURST` | S    | `2 × rps`      | Token-bucket burst cap (min 1). Only meaningful when `PENSIEVE_INGEST_RATE_RPS` is set.      |

## Compaction, retention, GC

| Name                              | Side | Default       | Purpose                                                                       |
| --------------------------------- | ---- | ------------- | ----------------------------------------------------------------------------- |
| `PENSIEVE_COMPACTION_IDLE_SLEEP_MS`   | S    | _bin default_ | Sleep between work polls when the compaction queue is empty.                  |
| `PENSIEVE_COMPACTION_POLL_SECS`       | S    | _bin default_ | Scheduler poll interval (seconds).                                            |
| `PENSIEVE_COMPACTION_MIN_EXTENTS`     | S    | _bin default_ | Minimum extent count per `(table, time-bucket)` before compaction fires.      |
| `PENSIEVE_RETENTION_POLL_SECS`        | S    | _bin default_ | Retention sweeper poll interval (seconds).                                    |
| `PENSIEVE_PHYSICAL_GC_POLL_SECS`      | S    | _bin default_ | Physical-delete worker poll interval (seconds).                               |
| `PENSIEVE_PHYSICAL_GC_GRACE_SECS`     | S    | _bin default_ | Grace period between soft-delete and hard-delete (seconds).                   |
| `PENSIEVE_ARTIFACT_GC_POLL_SECS`      | S    | `300`         | Poll interval for the artifact-retention sweeper + artifact-graph/content-index sync. |
| `PENSIEVE_ARTIFACT_GC_GRACE_SECS`     | S    | _bin default_ | Grace period before expired object-store artifacts are physically deleted.    |

All of these run only on job-running roles (`all_in_one` / `worker`); see
[Roles & scale-out](#roles-and-scale-out).

## File-drop

| Name                                | Side | Default     | Purpose                                                                                |
| ----------------------------------- | ---- | ----------- | -------------------------------------------------------------------------------------- |
| `PENSIEVE_FILEDROP_ENABLED`             | S    | _unset_     | `1` or `true` enables the file-drop watcher.                                            |
| `PENSIEVE_FILEDROP_PREFIXES`            | S    | `ingest`    | Comma-separated object-store prefixes to watch.                                         |
| `PENSIEVE_FILEDROP_PREFIX`              | S    | _unset_     | Legacy single-prefix form. `PENSIEVE_FILEDROP_PREFIXES` wins when both are set.             |
| `PENSIEVE_FILEDROP_POLL_SECS`           | S    | `5`         | Watcher poll interval.                                                                  |
| `PENSIEVE_FILEDROP_DELETE_AFTER_INGEST` | S    | `false`     | Delete the source object after it commits. Off by default — replays remain idempotent.  |
| `PENSIEVE_FILEDROP_AUTO_CREATE`         | S    | `true`      | Create the database + table on first sight.                                             |
| `PENSIEVE_FILEDROP_SCHEMA_EVOLVE`       | S    | `true`      | Add new columns mid-batch.                                                              |

## Kafka

| Name                          | Side | Default          | Purpose                                                                  |
| ----------------------------- | ---- | ---------------- | ------------------------------------------------------------------------ |
| `PENSIEVE_KAFKA_ENABLED`          | S    | _unset_          | `1` or `true` enables the Kafka consumer.                                |
| `PENSIEVE_KAFKA_BROKERS`          | S    | `localhost:9092` | Comma-separated broker list.                                             |
| `PENSIEVE_KAFKA_GROUP`            | S    | `pensieve-ingest`    | Consumer group id.                                                       |
| `PENSIEVE_KAFKA_TOPICS`           | S    | _unset_          | Comma-separated `topic:database.table` mappings. Required to enable.     |
| `PENSIEVE_KAFKA_BATCH_SIZE`       | S    | `500`            | Per-batch row count.                                                     |
| `PENSIEVE_KAFKA_BATCH_TIMEOUT_MS` | S    | `500`            | Per-batch wall-clock timeout.                                            |

## Data sources

| Name                       | Side | Default | Purpose                                                                                              |
| -------------------------- | ---- | ------- | ---------------------------------------------------------------------------------------------------- |
| `PENSIEVE_DATA_SOURCE_WORKERS`   | S    | `4`     | Number of data-source-runner tasks. Also the fallback for `PENSIEVE_FABRIC_WORKERS` when unset.            |
| `PENSIEVE_DISCOVER_MAX_SOURCES` | S   | _bin default_ | Cap on sources returned by the discovery endpoint.                                             |
| `PENSIEVE_GH_PAT`              | S    | _unset_ | GitHub personal-access token injected into `git clone` credentials for private-repo data sources.      |

Per-data-source secrets are resolved through the `EnvSecretStore` —
config values written as `$env:VAR_NAME` resolve to whatever
`std::env::var("VAR_NAME")` returns at fetch time.

## Credentials & OAuth

The typed credentials store (PATs, OAuth tokens, connection URLs) is encrypted
at rest; the [OAuth connect flow](/data-sources/oauth) needs a public origin and a
client app per provider.

| Name                          | Side | Default                 | Purpose                                                                                              |
| ----------------------------- | ---- | ----------------------- | ---------------------------------------------------------------------------------------------------- |
| `PENSIEVE_SECRET_KEY`             | S    | _unset_ (required for credentials) | AES-256-GCM key for the credentials store. Base64 of 32 random bytes (`openssl rand -base64 32`); shorter values are SHA-256 stretched (dev only). |
| `PENSIEVE_OAUTH_REDIRECT_BASE`    | S    | `http://localhost:8080` | Externally reachable origin. Builds the provider `redirect_uri` (`<base>/v1/oauth/<provider>/callback`). |
| `PENSIEVE_OAUTH_UI_RETURN_BASE`   | S    | = redirect base         | Where the callback sends the browser back to (the web UI origin).                                    |
| `PENSIEVE_OAUTH_<PROVIDER>_CLIENT_ID` | S | _unset_             | Operator client id. `<PROVIDER>` ∈ `GOOGLE`, `NOTION`, `ATLASSIAN`, `SLACK`.                          |
| `PENSIEVE_OAUTH_<PROVIDER>_CLIENT_SECRET` | S | _unset_         | Operator client secret. A per-tenant bring-your-own app (set in the UI) takes precedence over these.  |

## Worker fabric

The server-side job dispatch layer that routes work to registered worker nodes.

| Name                        | Side | Default | Purpose                                                                             |
| --------------------------- | ---- | ------- | ----------------------------------------------------------------------------------- |
| `PENSIEVE_FABRIC_WORKERS`       | S    | `4`     | Goroutine-pool size for the fabric dispatcher. Falls back to `PENSIEVE_DATA_SOURCE_WORKERS`. |
| `PENSIEVE_FABRIC_LEASE_SECS`    | S    | `300`   | How long a job lease is held before the fabric considers it expired (5 minutes).    |
| `PENSIEVE_FABRIC_SWEEP_SECS`    | S    | `30`    | How often the fabric sweeps for expired leases.                                     |
| `PENSIEVE_FABRIC_OFFLINE_SECS`  | S    | `90`    | Seconds without a heartbeat before a node is marked offline.                        |

## Node daemon / local sync

Variables read by `pensieve worker run` (the fabric node daemon) and
`pensieve sync` / `pensieve worker install` (the CC-memory sync service).
These are **client-side** — the running server reads none of them.

| Name                    | Side | Default        | Purpose                                                                                |
| ----------------------- | ---- | -------------- | -------------------------------------------------------------------------------------- |
| `PENSIEVE_SERVER_URL`       | C    | _required_     | Control-plane URL. Required for `pensieve worker run`; also overrides the saved client endpoint for all `pensieve` subcommands. |
| `PENSIEVE_TOKEN`            | C    | _unset_        | Bearer token override for all client subcommands (wins over `~/.pensieve/config.json`).    |
| `PENSIEVE_WORKER_TOKEN`     | C    | _required_     | Node authentication token minted by `pensieve worker create`. Required for `pensieve worker run`. |
| `PENSIEVE_WORKER_INSECURE`  | C    | _unset_        | Set to `1` to allow HTTP (non-TLS) connections from the node daemon to the server. Tokens ride this channel. |
| `PENSIEVE_CC_SYNC_POLL_SECS` | C   | `30`           | Poll interval (seconds) for `pensieve sync --watch` and the background worker service.    |
| `PENSIEVE_CC_WATCH`         | C    | `0` (off)      | Set to `1` to enable the continuous CC-file sync watcher inside `pensieve serve`.         |
| `PENSIEVE_CC_FILE_SYNC`     | C    | `1` (on)       | Set to `0` to disable the Claude Code file-memory phase of sync entirely.             |
| `PENSIEVE_CC_SYNC_ON_MCP`   | C    | `1` (on)       | Set to `0` to skip the one-shot file sync that fires when the MCP server initialises. |
| `PENSIEVE_CC_HOME`          | C    | `~/.claude`    | Root directory that pensieve treats as the Claude Code home (scans `CLAUDE.md`, `projects/*/memory`). |
| `PENSIEVE_CLOUD_URL`        | C    | _unset_        | URL of the pensieve control plane to push/pull memories to. Unset → local-only sync.      |
| `PENSIEVE_CLOUD_TOKEN`      | C    | _unset_        | Bearer token for the cloud control plane.                                              |
| `PENSIEVE_SYNC_REALM`       | C    | _unset_        | Memory realm to sync to/from on the control plane. Unset → server default.            |
| `PENSIEVE_NO_UPDATE_CHECK`  | C    | _unset_        | Set to any value to suppress the background update-available nudge in `pensieve status` / `pensieve version`. |

## CC-memory curation

Knobs for the Claude Code memory curation pass (promotion and LLM-curated
deduplication / staleness review).

| Name                           | Side | Default | Purpose                                                                                 |
| ------------------------------ | ---- | ------- | --------------------------------------------------------------------------------------- |
| `PENSIEVE_CC_CURATE`               | SC   | `1` (on) | Master switch. Set to `0` to disable all curation.                                     |
| `PENSIEVE_CC_PROMOTE`              | SC   | `1` (on) | Write high-importance memories back as native `.md` files for Claude Code to load.      |
| `PENSIEVE_CC_PROMOTE_MAX`          | SC   | `15`    | Max number of memories promoted to files per project.                                   |
| `PENSIEVE_CC_PROMOTE_MIN_IMPORTANCE` | SC | `0.6`  | Minimum importance score a memory needs to be eligible for promotion.                   |
| `PENSIEVE_CC_QUIET_WINDOW`         | SC   | `300`   | Seconds of inactivity before a promotion pass is allowed (prevents churn during edits). |
| `PENSIEVE_CC_LOCK_TTL`             | C    | `300`   | Seconds before a stale curation lock is forcibly released.                              |
| `PENSIEVE_CC_STALE_DAYS`           | SC   | `90`    | Age in days before a memory is considered stale for LLM review.                         |
| `PENSIEVE_CC_DUP_COSINE`           | SC   | `0.90–0.97 band` | Cosine-similarity range where duplication is plausible but not certain — the model decides. |

## Agentic memory

| Name                              | Side | Default   | Purpose                                                                                    |
| --------------------------------- | ---- | --------- | ------------------------------------------------------------------------------------------ |
| `PENSIEVE_MEMORY_ASYNC`               | SC   | `1` (on)  | `0` makes memory writes synchronous (blocks until the embedding + save complete).          |
| `PENSIEVE_MEMORY_QUEUE_DURABLE`       | SC   | `0` (local) / `1` (server) | `1` persists the in-flight memory queue to the catalog so it survives restarts. Defaults on for the server binary, off for local/stdio.            |
| `PENSIEVE_MEMORY_CONSOLIDATION`       | S    | `1` (on)  | `0` disables the background memory-consolidation worker.                                   |
| `PENSIEVE_MEMORY_CONSOLIDATION_POLL_SECS` | S | _bin default_ | Poll interval for the memory consolidation background task.                           |
| `PENSIEVE_SESSION_SUMMARY_EVERY`      | S    | `12`      | Refresh the rolling session summary every N turns.                                         |
| `PENSIEVE_CI_CORRELATE`               | S    | `1` (on)  | `0` disables the CI failure-correlation pipeline (writes recurring-failure memories).      |
| `PENSIEVE_CI_CORRELATE_POLL_SECS`     | S    | _bin default_ | Poll interval for the CI-correlation pipeline.                                         |
| `PENSIEVE_FILE_PROMOTE`               | S    | `1` (on)  | `0` disables the file-candidate promotion pipeline (stitches contributed File nodes to live repo nodes). |
| `PENSIEVE_FILE_PROMOTE_POLL_SECS`     | S    | _bin default_ | Poll interval for the file-promotion pipeline.                                         |
| `PENSIEVE_MEMORY_PPR`                 | S    | `0` (off) | `1`/`true` rescores recall with personalized PageRank over the memory graph (else hop-decay proximity). Capability is shipped; enabling-by-default is gated on benchmark uplift. |
| `PENSIEVE_MEMORY_CLASS_DECAY`         | S    | `0` (off) | `1`/`true` applies memory-class-aware recency decay (episodic short half-life, semantic invalidation-only). Default keeps the uniform recency term. |

The consolidation / CI-correlate / file-promote pipelines run only on
job-running roles (`all_in_one` / `worker`); see [Roles & scale-out](#roles-and-scale-out).

## Agent

| Name                  | Side | Default                          | Purpose                                                                    |
| --------------------- | ---- | -------------------------------- | -------------------------------------------------------------------------- |
| `PENSIEVE_AGENT_MCP_URL`  | S    | _auto (derived from HTTP addr)_  | Override the MCP endpoint the agent routes tool calls to. `off` disables.  |
| `PENSIEVE_OLLAMA_HOST`    | S    | `http://localhost:11434`         | Ollama host the server-side agent uses.                                    |

## Embeddings

For dense-vector ingestion + nearest-neighbour scan. Selects the
embedding backend at startup.

| Name                     | Side | Default                                 | Purpose                                                                        |
| ------------------------ | ---- | --------------------------------------- | ------------------------------------------------------------------------------ |
| `PENSIEVE_EMBED_PROVIDER`    | SC   | `fastembed`                             | One of `fastembed`, `ollama`, `openai-compat`, `gemini`.                       |
| `PENSIEVE_EMBED_MODEL_ID`    | SC   | provider-specific (e.g. `bge-small-en-v1.5`) | Model id passed to the provider.                                          |
| `PENSIEVE_EMBED_BASE_URL`    | SC   | provider-specific                       | Base URL for HTTP-based providers (Ollama, OpenAI-compat).                     |
| `PENSIEVE_EMBED_MODEL_PATH`  | SC   | _unset_                                 | Local model path for `fastembed` (overrides id).                               |
| `PENSIEVE_EMBED_API_KEY_ENV` | SC   | `OPENAI_API_KEY`                        | Env var to read the API key from (for `openai-compat`).                        |

`gemini` reads the API key from `GOOGLE_API_KEY` (fixed).

### Cross-encoder reranker

Optional final reranking stage for hybrid search + memory recall. Off unless
`PENSIEVE_RERANK_MODEL` is set; when configured, the top fused results are re-scored
by a cross-encoder before returning.

| Name                    | Side | Default | Purpose                                                                            |
| ----------------------- | ---- | ------- | ---------------------------------------------------------------------------------- |
| `PENSIEVE_RERANK_MODEL`     | SC   | _unset_ | fastembed reranker model id (e.g. `bge-reranker-base`). Unset ⇒ no reranking.      |
| `PENSIEVE_RERANK_MODEL_PATH`| SC   | _unset_ | Local model path override for the reranker.                                        |
| `PENSIEVE_RERANK_POOL_SIZE` | SC   | `1`     | Number of reranker inference instances (parallel ONNX sessions).                   |

## UI / icon gallery

| Name                | Side | Default | Purpose                                                                                      |
| ------------------- | ---- | ------- | -------------------------------------------------------------------------------------------- |
| `PENSIEVE_ICON_GALLERY` | S    | _unset_ | Path to a JSON file that extends or overrides the built-in icon gallery (`{"kinds":{...},"vendors":{...}}`). |

## Debug / verbose flags

These are CLI-side flags that increase output verbosity for specific operations.
They are not set in production.

| Name                    | Side | Default | Purpose                                                                    |
| ----------------------- | ---- | ------- | -------------------------------------------------------------------------- |
| `PENSIEVE_INGEST_VERBOSE`   | C    | _unset_ | Print per-row ingest detail from `pensieve ingest push`.                       |
| `PENSIEVE_DISTILL_VERBOSE`  | C    | _unset_ | Print distillation progress from `pensieve distill`.                           |

## Logging

`tracing-subscriber` reads the standard `RUST_LOG` env var. The
default filter is `info,sqlx=warn,hyper=warn,h2=warn`.
