---
title: Framework
description: The DataSource trait, the type registry, the periodic scheduler and runner, the admin REST API at /v1/data-sources, and the secret-by-reference resolver. One generic loop; engines plug in.
---

# Framework

A data source is a small Rust impl that produces rows on a schedule. The
framework owns everything else — claiming the next tick, resolving
secrets, sinking rows into the ingest write path, advancing the cursor,
classifying failures, and exposing the admin REST API.

This page is the contract. Every page that follows (Prometheus, Postgres,
MySQL, Mongo) is a config schema and a row shape on top of it.

## The `DataSource` trait

```rust
#[async_trait]
pub trait DataSource: Send + Sync + 'static {
    fn type_id(&self) -> &'static str;

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError>;

    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError>;
}
```

`run_once` returns `DataSourceRun { rows: Vec<serde_json::Value>,
new_cursor: Option<serde_json::Value> }`. The framework JSON-to-Arrow
coerces the rows, sinks them through the ingest write path with a
deterministic idempotency key, and — only if the sink succeeds — upserts
`new_cursor` into `data_source_cursors`. Sink failure is treated as
transient: the next tick replays from the previous cursor, and the same
idempotency key dedupes any rows that got partway through.

## Failure classification

`DataSourceError` has three variants and the runner branches on them:

| Variant      | What happens                                                              |
| ------------ | ------------------------------------------------------------------------- |
| `Transient`  | Logged. `last_error` set. Task fails; the scheduler reschedules normally. |
| `Permanent`  | Logged. `last_error` set. Task is completed (no retry); next tick runs.   |
| `Config`     | Data source is disabled with `disabled_reason`. Operator must `POST /resume`.|

Engine impls map their native errors to one of these three. The shared
runner does not interpret engine-specific errors.

## Drive models

`DriveModel` is `Periodic { interval_ms }` or `Continuous { heartbeat_ms }`.
Periodic is what the shipped runner supports — the scheduler enqueues
one `data_source_tick` task per due data source per bucketed interval, and a
runner claims it. Bucketing means that even with N runners and a flaky
clock, you get exactly one tick per `schedule_ms` window.

`Continuous` is reserved for CDC-style data sources — see
[Postgres](/data-sources/postgres), [MySQL](/data-sources/mysql),
[MongoDB](/data-sources/mongo).

## Registration

Types register into a `DataSourceRegistry` at startup:

```rust
let mut registry = DataSourceRegistry::new();
registry.register(Arc::new(PromDataSource));
// future: registry.register(Arc::new(PostgresDataSource));
let registry = Arc::new(registry);
```

The admin API rejects `type` values not in the registry with `400`. New
engine types are a one-line registration plus a trait impl.

## Admin REST API

```
POST   /v1/data-sources            create
GET    /v1/data-sources            list
GET    /v1/data-sources/:id        get one (secrets scrubbed)
PATCH  /v1/data-sources/:id        update name/schedule/enabled/config
DELETE /v1/data-sources/:id        remove
POST   /v1/data-sources/:id/pause  enabled=false, disabled_reason='manual'
POST   /v1/data-sources/:id/resume enabled=true,  disabled_reason=null
POST   /v1/data-sources/:id/trigger enqueue an out-of-band tick now
```

A create body looks like this:

```bash
curl -sS -X POST http://localhost:8080/v1/data-sources \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "node-exporter-1",
  "type": "prometheus",
  "target_database": "default",
  "target_table": "metrics",
  "schedule_ms": 15000,
  "config": {
    "endpoint": "http://node-exporter:9100/metrics",
    "auth": { "type": "none" },
    "timeout_ms": 5000
  }
}
JSON
```

The handler calls `validate_config` against the registered impl before
persisting; bad config returns `400` with the validator's message. The
`schedule_ms` must be in `[100, 86_400_000]` (100 ms to one day).

`GET /v1/data-sources/:id` returns the row plus `last_run_at`,
`last_success_at`, `last_error`, `last_rows_ingested`. Anything that
looks like a secret in the config (`token`, `password`, `secret`, `key`)
is redacted to `***` unless it's an unresolved `$env:` reference, which
is returned verbatim.

DB-M0 extends `POST /v1/data-sources` with `mode`, `connection`, `scope`,
and `sync` fields. The `GET /v1/data-sources/:id` detail endpoint, and
`POST /v1/data-sources/:id/pause`, `/resume`, and `/trigger`, work the
same way for all data source types including the DB engines.

## Secrets by reference

The framework never stores plaintext credentials in the catalog. Config
values referencing a secret use `$env:NAME` — the literal string lands
in `data_sources.config_jsonb`, and the data source calls
`ctx.secrets.resolve(&value)` at tick time:

```json
{ "auth": { "type": "bearer", "token_ref": "$env:NODE_EXPORTER_TOKEN" } }
```

`SecretStore` is a trait with one method (`resolve(&self, &str) -> Result<String>`).
The shipped impl is `EnvSecretStore`, which reads `$env:NAME` from the
environment of the pensieve process. Database-engine data sources in DB-M0+
add a file-based store (`PENSIEVE_SECRETS_FILE`) and an in-cluster store;
Vault / AWS Secrets Manager / GCP Secret Manager are documented
extension points.

## Scheduler and runner

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Scheduler   │ ──▶ │ background   │ ◀── │   Runner     │
│ tick every   │     │   _tasks     │     │ claim_and_   │
│   500 ms     │     │   queue      │     │   run_one    │
└──────────────┘     └──────────────┘     └──────────────┘
```

The scheduler walks `data_sources` every 500 ms, finds rows due for a
tick (`now() - last_run_at >= schedule_ms`), and enqueues a
`data_source_tick` task with a `bucketed = (now_ms / schedule_ms) * schedule_ms`
key — so two schedulers on two nodes never both enqueue the same tick.

The runner claims one task at a time with a 60-second lease. Inside
the claim it loads the data source row, calls `DataSource::run_once` with
the resolved cursor, sinks rows, advances the cursor, and either
completes or fails the task. On `kill -9` mid-tick, the lease expires
and another runner picks the same `(data_source_id, bucketed)` up; the
idempotency key on the sink ensures replay-safety.

## Where to go next

- The reference impl: [Prometheus](/data-sources/prometheus).
- Database engines: [Postgres](/data-sources/postgres),
  [MySQL](/data-sources/mysql), [MongoDB](/data-sources/mongo).
- The conceptual companion: [Multi-source data](/concepts/multi-source-data).
