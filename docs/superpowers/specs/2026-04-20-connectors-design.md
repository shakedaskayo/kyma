# Connectors Framework + Prometheus Reference Connector — Design

**Date:** 2026-04-20
**Status:** Design approved; implementation plan pending
**Owner:** Shaked

## 1. Goal and scope

Introduce an **ingestion connectors** subsystem: long-lived framework components that pull data from third-party observability sources (Prometheus, Sentry, Loki/Grafana, Elastic, etc.) into kyma tables, reusing the existing ingest pipeline (WritePath → staging buffer → CommitCoordinator → catalog).

The existing four ingest surfaces (REST push, file-drop, OTLP gRPC, Kafka consume) are all **push** — external systems send data to kyma. Connectors are the missing **pull/poll** dual: kyma reaches out to external APIs on a schedule, fetches data, transforms it, and ingests it.

### In scope (slice-1)

- A reusable connector framework: trait, scheduler, runner, registry, secret store, admin HTTP API, catalog-managed configuration, metrics.
- One reference connector implementation end-to-end: **Prometheus `/metrics` scrape**.
- Crash-safety guarantees: at-least-once at the connector boundary, exactly-once at the table (via existing `ingest_ledger` idempotency).
- Pre-provisioning for future connector types (Sentry, Loki, Elastic, etc.) so adding each is ~1-day work against the framework.

### Out of scope (deferred, tracked as follow-ons)

- `Continuous` drive model for streaming connectors (Loki tail, Elastic follow). Trait variant and `connector_leases` table pre-provisioned; implementation deferred.
- Additional connector types: Sentry, Loki, Elastic, Datadog, Cloudwatch.
- Pluggable `SecretStore` backends (Vault, AWS Secrets Manager, GCP SM). Slice-1 ships the trait plus env-var reference default only.
- OpenMetrics exemplar / trace-ID extraction.
- Service discovery (Consul, Kubernetes) for Prometheus targets — static endpoints only.
- Distributed fair-share scheduling across a large number of runner nodes (current lease-based model is correct but not optimized for oversubscription).

## 2. Primitives reused

The connector framework is small because the engine already has the infrastructure:

| Primitive | Existing use | Connector use |
|---|---|---|
| `background_tasks` work queue | Compaction, retention, physical-delete | Periodic connector tick scheduling |
| `ingest_ledger` idempotency | REST X-Idempotency-Key, file-drop SHA-dedup, Kafka at-least-once | Per-tick `scheduled_for` key to dedupe retries |
| `WritePath` (`kyma-ingest-core`) | REST, file-drop, OTLP, Kafka | All connector output goes through it |
| NDJSON→Arrow coercion | REST ingest, file-drop | Connector `Vec<serde_json::Value>` rows |
| `metrics` facade + Prometheus exporter | All existing observability | Per-connector metric cardinality |
| axum router + auth middleware | Ingest and query surfaces | Admin API under `/v1/connectors` |
| `kyma-bin::main` wiring pattern | Scheduler + worker spawns for compaction | Mirror pattern for connector scheduler + runner |

## 3. Crates and layout

**New crate:** `kyma-connectors` at `crates/kyma-connectors/`.

```
crates/kyma-connectors/
├── Cargo.toml
├── src/
│   ├── lib.rs           -- Connector trait, ConnectorCtx, ConnectorRun,
│                           ConnectorError, DriveModel enum
│   ├── secrets.rs       -- SecretStore trait + EnvSecretStore impl
│   ├── registry.rs      -- ConnectorRegistry (type_id → Arc<dyn Connector>)
│   ├── scheduler.rs     -- Periodic tick scheduler
│   ├── runner.rs        -- Tick worker loop
│   ├── metrics.rs       -- Metric helpers
│   ├── prometheus.rs    -- Reference PromConnector impl
│   └── admin.rs         -- axum router for /v1/connectors CRUD
└── tests/               -- Unit tests (OpenMetrics parser, config validation,
                            secret resolution)
```

**Wiring changes:**

- `kyma-bin/src/main.rs` spawns connector scheduler + N runner tasks, analogous to compaction.
- `kyma-server` mounts `admin::router()` on its axum app.
- `kyma-catalog` gains a new migration and a handful of helper methods.

## 4. Catalog schema (migration 005)

```sql
-- 005_connectors.sql

CREATE TABLE connectors (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE,
    type             TEXT NOT NULL,                       -- 'prometheus', 'sentry', ...
    target_database  TEXT NOT NULL,
    target_table     TEXT NOT NULL,
    config_jsonb     JSONB NOT NULL,
    schedule_ms      BIGINT NOT NULL CHECK (schedule_ms >= 100),
    drive_model      TEXT NOT NULL CHECK (drive_model IN ('periodic','continuous')),
    enabled          BOOLEAN NOT NULL DEFAULT TRUE,
    disabled_reason  TEXT,
    last_run_at      TIMESTAMPTZ,
    last_success_at  TIMESTAMPTZ,
    last_error       TEXT,
    last_rows_ingested BIGINT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX connectors_enabled_drive_idx
    ON connectors (drive_model, enabled)
    WHERE enabled = TRUE;

CREATE TABLE connector_cursors (
    connector_id  UUID PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    cursor_jsonb  JSONB,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Pre-provisioned for Continuous drive model. Unused in slice-1.
CREATE TABLE connector_leases (
    connector_id  UUID PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    node_id       TEXT NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL
);

-- Prevent duplicate connector_tick enqueues from concurrent schedulers on
-- different nodes. Partial index scoped to active tasks so that after a tick
-- completes the same (connector_id, scheduled_for) pair can re-appear for a
-- retry path if needed.
CREATE UNIQUE INDEX background_tasks_connector_tick_uniq
    ON background_tasks ((payload->>'connector_id'),
                         (payload->>'scheduled_for'))
    WHERE kind = 'connector_tick' AND status IN ('pending', 'claimed');
```

## 5. The Connector trait

```rust
// crates/kyma-connectors/src/lib.rs

#[async_trait::async_trait]
pub trait Connector: Send + Sync + 'static {
    fn type_id(&self) -> &'static str;

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError>;

    async fn run_once(
        &self,
        ctx: &ConnectorCtx,
        cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError>;
}

pub struct ConnectorRun {
    /// JSON rows — fed through existing NDJSON→Arrow coercion.
    pub rows: Vec<serde_json::Value>,
    /// None = no cursor update this tick.
    pub new_cursor: Option<serde_json::Value>,
}

pub struct ConnectorCtx {
    pub connector_id: Uuid,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    pub scheduled_for: chrono::DateTime<chrono::Utc>,
    pub metrics: ConnectorMetrics,
}

pub enum ConnectorError {
    /// Framework will retry with jittered exponential backoff within the tick.
    Transient(String),
    /// Framework fails this tick, schedules next on normal interval.
    Permanent(String),
    /// Framework disables the connector; operator must re-enable.
    Config(String),
}

pub enum DriveModel {
    Periodic { interval_ms: u64 },
    Continuous { heartbeat_ms: u64 },  // stub in slice-1
}

pub trait SecretStore: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, SecretError>;
}
```

### Why JSON rows, not `RecordBatch`

Connector authors work with `serde_json::Value` — the natural output shape for HTTP/API clients. The framework runs rows through the existing NDJSON→Arrow coercion path (already used by REST ingest), so zero new coercion code. Performance cost relative to direct Arrow construction is not a bottleneck — scrape is I/O bound. If a specific future connector profiles hot, the trait can grow an opt-in `Arrow` variant; not needed in slice-1.

### Registration

Slice-1 uses compile-time registration only. No dynamic loading.

```rust
// kyma-bin/src/main.rs
let mut reg = ConnectorRegistry::new();
reg.register(Arc::new(kyma_connectors::prometheus::PromConnector::default()));
// Future: reg.register(Arc::new(kyma_connectors::sentry::SentryConnector::default()));
```

## 6. Runtime model

### Drive models

- `Periodic { interval_ms }` — implemented in slice-1. Scheduler periodically enqueues `connector_tick` work-queue rows, runners claim via `SELECT FOR UPDATE SKIP LOCKED`.
- `Continuous { heartbeat_ms }` — trait variant exists; runner path returns `todo!()` for now. `connector_leases` table pre-provisioned to avoid a future migration.

### Scheduler

Single task per `kyma-bin` instance. Tick interval: 500 ms.

```
loop every 500ms:
  rows = SELECT id, schedule_ms, last_run_at
         FROM connectors
         WHERE enabled AND drive_model = 'periodic'
  for r in rows:
    scheduled_for = floor(now_ms / r.schedule_ms) * r.schedule_ms   -- bucketed
    if (now - r.last_run_at) >= r.schedule_ms:
      INSERT INTO background_tasks
        (kind='connector_tick',
         payload={connector_id: r.id, scheduled_for: scheduled_for})
      ON CONFLICT DO NOTHING   -- partial unique index wins; other scheduler got there first
```

Multiple kyma nodes running the scheduler concurrently is safe: the partial unique index `background_tasks_connector_tick_uniq` (added in migration 005) prevents duplicate `connector_tick` rows for the same `(connector_id, scheduled_for)` pair while the task is `pending` or `claimed`. Concurrent enqueues race on the insert; the loser treats the unique-violation as expected and moves on. `scheduled_for` is bucketed to the connector's `schedule_ms` grid so two schedulers computing "now" a few ms apart still converge on the same key.

### Runner worker

N per instance (env `KYMA_CONNECTOR_WORKERS`, default 4), mirroring compaction.

```
loop:
  task = claim next connector_tick task (FOR UPDATE SKIP LOCKED)
  if none: sleep 100ms; continue
  connector = load(task.payload.connector_id)          -- catalog + cursor
  impl = registry.lookup(connector.type)
  ctx = build_ctx(connector, scheduled_for)
  match impl.run_once(ctx, connector.config_jsonb, cursor).await:
    Ok(run) =>
      idem_key = format!("connector:{}:{}", id, scheduled_for_ns)
      WritePath::ingest(target_db, target_table, run.rows, Some(idem_key)).await?
      UPSERT connector_cursors SET cursor_jsonb = run.new_cursor
      UPDATE connectors SET last_run_at, last_success_at, last_rows_ingested
      mark_task_complete()
      metrics: ticks_total{result=ok} +1; rows_ingested_total +=N;
               last_success_timestamp_seconds = now
    Err(Transient) =>
      mark_task_failed() -- background_tasks retry infra handles backoff
      metrics: ticks_total{result=transient} +1; errors_total{reason=transient} +1
    Err(Permanent) =>
      mark_task_complete()  -- don't retry within this tick
      UPDATE connectors SET last_run_at, last_error
      metrics: ticks_total{result=permanent} +1; errors_total{reason=permanent} +1
    Err(Config) =>
      UPDATE connectors SET enabled=false, disabled_reason
      mark_task_complete()
      metrics: errors_total{reason=config} +1
```

### Retry behaviour inside `run_once`

A connector implementation is free to retry transient HTTP failures inside `run_once` with jittered exponential backoff up to 3 attempts, which is what `PromConnector` does. If still failing after retries, it returns `ConnectorError::Transient`, and the `background_tasks` infrastructure retries the tick itself.

### Crash safety

Two failure windows exist. Both are covered by `ingest_ledger`, keyed `connector:{id}:{scheduled_for_ns}`:

1. **Extent committed, crash before cursor update.** Next run (may come from another worker after lease expiry) re-invokes `run_once` with the *old* cursor → same rows emitted → idempotency ledger short-circuits the WritePath ingest. Cursor then advances.
2. **Cursor updated, crash before task marked complete.** Task lease expires → another worker claims it → `run_once` with the *new* cursor → different rows (or empty) → idempotency ledger still matches on the same `scheduled_for_ns` → no duplication.

Net guarantee: at-least-once at the connector boundary, exactly-once at the destination table.

## 7. Secrets: `SecretStore`

```rust
pub trait SecretStore: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, SecretError>;
}

pub struct EnvSecretStore;

impl SecretStore for EnvSecretStore {
    fn resolve(&self, r: &str) -> Result<String, SecretError> {
        if let Some(name) = r.strip_prefix("$env:") {
            std::env::var(name).map_err(|_| SecretError::NotFound(name.into()))
        } else {
            // literal value; pass through
            Ok(r.to_string())
        }
    }
}
```

`$env:NAME` is the only reference syntax in slice-1. Additional backends (Vault, AWS SM) plug in as `impl SecretStore` later — no schema or API change needed.

## 8. Schema shape for Prometheus data

Single wide table `metrics` (default `target_table`):

| Column | Type | Notes |
|---|---|---|
| `timestamp` | timestamp | Scrape time (we ignore per-sample timestamps in the text exposition format). |
| `name` | string | e.g. `http_requests_total`, `http_requests_bucket`. Histograms/summaries arrive already exploded. |
| `value` | float64 | `NaN`/`±Inf` → `null`. |
| `labels` | dynamic | All Prom labels as a JSON object. |

One row per Prometheus sample. `kyma-exec`'s column statistics + equality index + text-index pruning apply naturally to `name`. Label pruning uses the `dynamic` column path-presence bitmap already on the roadmap.

Rationale for single-wide over table-per-metric: avoids ALTER-TABLE storms on every new metric name, matches ADX/Honeycomb landing patterns for metrics, and query patterns (`metrics | where name == "x"`) compose well with existing pruning. Table-per-metric can be added later as an opt-in per-connector transform if profiling demands.

### Prometheus-specific transform notes

- Exposition format: OpenMetrics text (`application/openmetrics-text`) with fallback to Prometheus text (`text/plain; version=0.0.4`).
- Histograms and summaries arrive already exploded (`_bucket`, `_sum`, `_count`) — emitted as individual metric names with `le`/`quantile` as labels. No server-side re-aggregation.
- Stale markers (`# EOF` semantics) not emitted as tombstone rows in slice-1. Missing metrics simply do not advance.
- Target metadata (`HELP`, `TYPE`, `UNIT`) consumed by the parser for validation but not persisted to rows.

## 9. Admin HTTP API

Mounted under `/v1/connectors`. Uses same bearer-token auth middleware as ingest; `admin` role required.

```
POST   /v1/connectors                 -- create
GET    /v1/connectors                 -- list (basic status per row)
GET    /v1/connectors/:id             -- detail incl. cursor_updated_at, last_* fields
PATCH  /v1/connectors/:id             -- partial update (config, schedule_ms, enabled, name)
DELETE /v1/connectors/:id             -- soft-delete; cursor retained 7 days before GC
POST   /v1/connectors/:id/pause       -- enabled=false, disabled_reason='manual'
POST   /v1/connectors/:id/resume      -- enabled=true, clears disabled_reason
POST   /v1/connectors/:id/trigger     -- enqueue immediate tick
```

### Create payload example

```json
{
  "name": "prod-api-prom",
  "type": "prometheus",
  "target_database": "telemetry",
  "target_table": "metrics",
  "schedule_ms": 15000,
  "config": {
    "endpoint": "https://prod-api.internal/metrics",
    "auth": { "type": "bearer", "token_ref": "$env:PROM_TOKEN" },
    "timeout_ms": 5000
  }
}
```

### Detail response (beyond config)

```json
{
  "id": "…",
  "name": "…",
  "type": "prometheus",
  "target_database": "telemetry",
  "target_table": "metrics",
  "schedule_ms": 15000,
  "config": { "…redacted…": true },
  "status": {
    "enabled": true,
    "last_run_at": "2026-04-20T…",
    "last_success_at": "2026-04-20T…",
    "last_error": null,
    "last_rows_ingested": 1247,
    "next_scheduled_at": "2026-04-20T…",
    "cursor_updated_at": "2026-04-20T…"
  }
}
```

### Config validation

Framework calls `connector.validate_config(cfg)` before persisting on `POST`/`PATCH`. Failures return `400` with the validator's message. Dead-on-arrival configs never enter the catalog.

### Secret scrubbing on GET

- Any value matching `$env:*` is returned verbatim as `$env:*` literal.
- Any field name matching `/token|password|secret|key/i` with a literal (non-env-reference) value is replaced with `"***"` in GET responses.

## 10. Metrics

```
kyma_connector_ticks_total{connector_id, type, result}          counter
    result ∈ {ok, transient, permanent, config}
kyma_connector_rows_ingested_total{connector_id, type}          counter
kyma_connector_duration_seconds{connector_id, type}             histogram
kyma_connector_errors_total{connector_id, type, reason}         counter
kyma_connector_last_success_timestamp_seconds{connector_id}     gauge
kyma_connector_cursor_age_seconds{connector_id}                 gauge
kyma_connector_active_count{drive_model}                        gauge
```

Staleness alerting is the primary operational signal: a user alerts on `time() - kyma_connector_last_success_timestamp_seconds > 5 * schedule_seconds`.

## 11. Error handling

Every Prometheus scrape tick:

1. `PromConnector::run_once` attempts the HTTP fetch + parse.
2. Transient failures (connection refused, 5xx, timeout, DNS) → retried up to 3 times with jittered exponential backoff (100 ms → 400 ms → 1.6 s, ±30% jitter), capped at 5 s total.
3. If still failing → `ConnectorError::Transient` → `background_tasks` retries the tick per its existing retry policy.
4. 4xx errors (other than 429) → `ConnectorError::Permanent` — one log line, one metric increment, next tick on normal interval.
5. Parse errors (invalid OpenMetrics) → `Permanent` — same handling.
6. Configuration errors discovered at runtime (e.g., `auth.token_ref` references an env var that does not exist) → `ConnectorError::Config` — connector is disabled, operator re-enables via API after fixing.
7. 429 with `Retry-After` → honoured as `Transient` with the hint.

## 12. Testing

### Rust unit tests (in `kyma-connectors/tests/`)

- OpenMetrics parser on hand-crafted fixtures: counters, gauges, histograms-exploded, summaries-exploded, NaN/±Inf, comment-only lines, malformed lines.
- `PromConnector::validate_config`: happy path, missing `endpoint`, invalid scheme, unknown `auth.type`, invalid `schedule_ms`.
- `EnvSecretStore::resolve`: literal, `$env:NAME` present, `$env:NAME` missing.
- Registry: register/lookup/double-register behaviour.

### E2E test script: `scripts/test-prometheus-connector.sh`

Ten-green-assertion target. Shell-driven (matches existing E2E style).

1. Start docker-compose dev stack (Postgres + MinIO + Redpanda); start `kyma-bin`.
2. Start a mock Prometheus `/metrics` endpoint (Python `http.server` + fixture file).
3. `POST /v1/connectors` with `endpoint=http://127.0.0.1:PORT/metrics`, `schedule_ms=1000`. Assert `201`.
4. Wait 3 s. Assert `GET /v1/connectors/:id` shows `last_success_at` within the last 2 s.
5. Query kyma: `metrics | where name startswith "test_" | count` → equals fixture's expected N.
6. `metrics | where name == "http_requests_total" | project labels.status | distinct` → expected values.
7. `POST /v1/connectors/:id/pause`. Wait 2 s. Assert no new rows appeared.
8. Overwrite mock fixture with an HTTP 503 response. Wait. Assert `kyma_connector_errors_total{reason="transient"}` incremented.
9. Restore mock. Wait. Assert `last_success_at` advances.
10. `kill -9` the kyma process mid-scrape. Restart. Assert **no duplicate rows** in `metrics` for any `(timestamp, name, labels)` triple (idempotency guarantee).
11. `DELETE /v1/connectors/:id`. Assert connector absent from list. Assert `connector_cursors` row still present (soft-delete retains cursor 7 days).

### Integration test (testcontainers, Rust)

`kyma-connectors/tests/integration_cursor_crash.rs` — spins up a Postgres + MinIO testcontainer, runs a fake connector that always errors between cursor update and task complete, asserts that the recovery path produces no duplicates at the catalog level.

## 13. Operator-facing affordances (not slice-1, captured for follow-on)

- `/v1/connectors/:id/logs` — recent tick error tail.
- `/v1/connectors/:id/sample` — invoke `run_once` once and return the `rows` without ingesting (dry-run / debugging).
- Per-connector rate limit override.
- Multi-target bulk-create (POST an array).
- WebSocket/SSE stream of tick results (for admin UI).

These are cheap to add once the framework lands; capturing them here so the trait and admin API don't accidentally close the door.

## 14. Open questions for implementation phase

None load-bearing. The `validate_config` body for `PromConnector` is a small authoring decision best made while the surrounding code is live. The writing-plans skill will mark it as a user-contribution task in the plan.

## 15. Follow-on work (tracked, not implemented in slice-1)

1. Continuous drive model — Loki tail connector is the motivating consumer.
2. Sentry connector — cursor-paginated REST, bearer auth, rate-limit handling.
3. Loki, Elastic, Datadog, Cloudwatch connectors.
4. Service discovery integration (Kubernetes, Consul) for Prometheus targets.
5. Vault / AWS-SM / GCP-SM `SecretStore` implementations.
6. Exemplar / trace-ID extraction from OpenMetrics.
7. Distributed fair-share scheduling when `N_runner_nodes >> N_connectors`.
8. Opt-in per-connector table-per-metric transform for high-cardinality Prometheus deployments.
9. Admin UI surface (React app calling `/v1/connectors`).
10. Dry-run endpoint `POST /v1/connectors/:id/sample`.
