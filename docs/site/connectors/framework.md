---
title: Framework
description: The Connector trait, the type registry, the periodic scheduler and runner, the admin REST API at /v1/connectors, and the secret-by-reference resolver. One generic loop; engines plug in.
---

# Framework

A connector is a small Rust impl that produces rows on a schedule. The
framework owns everything else — claiming the next tick, resolving
secrets, sinking rows into the ingest write path, advancing the cursor,
classifying failures, and exposing the admin REST API.

This page is the contract. Every page that follows (Prometheus, Postgres,
MySQL, Mongo) is a config schema and a row shape on top of it.

## The `Connector` trait

```rust
#[async_trait]
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
```

`run_once` returns `ConnectorRun { rows: Vec<serde_json::Value>,
new_cursor: Option<serde_json::Value> }`. The framework JSON-to-Arrow
coerces the rows, sinks them through the ingest write path with a
deterministic idempotency key, and — only if the sink succeeds — upserts
`new_cursor` into `connector_cursors`. Sink failure is treated as
transient: the next tick replays from the previous cursor, and the same
idempotency key dedupes any rows that got partway through.

## Failure classification

`ConnectorError` has three variants and the runner branches on them:

| Variant      | What happens                                                              |
| ------------ | ------------------------------------------------------------------------- |
| `Transient`  | Logged. `last_error` set. Task fails; the scheduler reschedules normally. |
| `Permanent`  | Logged. `last_error` set. Task is completed (no retry); next tick runs.   |
| `Config`     | Connector is disabled with `disabled_reason`. Operator must `POST /resume`.|

Engine impls map their native errors to one of these three. The shared
runner does not interpret engine-specific errors.

## Drive models

`DriveModel` is `Periodic { interval_ms }` or `Continuous { heartbeat_ms }`.
Periodic is what the shipped runner supports — the scheduler enqueues
one `connector_tick` task per due connector per bucketed interval, and a
runner claims it. Bucketing means that even with N runners and a flaky
clock, you get exactly one tick per `schedule_ms` window.

`Continuous` is reserved for CDC-style connectors — see
[Postgres](/connectors/postgres), [MySQL](/connectors/mysql),
[MongoDB](/connectors/mongo).

## Registration

Types register into a `ConnectorRegistry` at startup:

```rust
let mut registry = ConnectorRegistry::new();
registry.register(Arc::new(PromConnector));
// future: registry.register(Arc::new(PostgresConnector));
let registry = Arc::new(registry);
```

The admin API rejects `type` values not in the registry with `400`. New
engine types are a one-line registration plus a trait impl.

## Admin REST API

```
POST   /v1/connectors            create
GET    /v1/connectors            list
GET    /v1/connectors/:id        get one (secrets scrubbed)
PATCH  /v1/connectors/:id        update name/schedule/enabled/config
DELETE /v1/connectors/:id        remove
POST   /v1/connectors/:id/pause  enabled=false, disabled_reason='manual'
POST   /v1/connectors/:id/resume enabled=true,  disabled_reason=null
POST   /v1/connectors/:id/trigger enqueue an out-of-band tick now
```

A create body looks like this:

```bash
curl -sS -X POST http://localhost:8080/v1/connectors \
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

`GET /v1/connectors/:id` returns the row plus `last_run_at`,
`last_success_at`, `last_error`, `last_rows_ingested`. Anything that
looks like a secret in the config (`token`, `password`, `secret`, `key`)
is redacted to `***` unless it's an unresolved `$env:` reference, which
is returned verbatim.

DB-M0 extends `POST /v1/connectors` with `mode`, `connection`, `scope`,
and `sync` fields, plus `GET /v1/connectors/:id/status`,
`/events`, `/test-connection`, and a scoped pause
(`?scope=sync|federation|all`). See
[the design spec](https://github.com/shaked/engine/blob/main/docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md)
for the full shape.

## Secrets by reference

The framework never stores plaintext credentials in the catalog. Config
values referencing a secret use `$env:NAME` — the literal string lands
in `connectors.config_jsonb`, and the connector calls
`ctx.secrets.resolve(&value)` at tick time:

```json
{ "auth": { "type": "bearer", "token_ref": "$env:NODE_EXPORTER_TOKEN" } }
```

`SecretStore` is a trait with one method (`resolve(&self, &str) -> Result<String>`).
The shipped impl is `EnvSecretStore`, which reads `$env:NAME` from the
environment of the kyma process. Database-engine connectors in DB-M0+
add a file-based store (`KYMA_SECRETS_FILE`) and an in-cluster store;
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

The scheduler walks `connectors` every 500 ms, finds rows due for a
tick (`now() - last_run_at >= schedule_ms`), and enqueues a
`connector_tick` task with a `bucketed = (now_ms / schedule_ms) * schedule_ms`
key — so two schedulers on two nodes never both enqueue the same tick.

The runner claims one task at a time with a 60-second lease. Inside
the claim it loads the connector row, calls `Connector::run_once` with
the resolved cursor, sinks rows, advances the cursor, and either
completes or fails the task. On `kill -9` mid-tick, the lease expires
and another runner picks the same `(connector_id, bucketed)` up; the
idempotency key on the sink ensures replay-safety.

## Where to go next

- The reference impl: [Prometheus](/connectors/prometheus).
- Database engines: [Postgres](/connectors/postgres),
  [MySQL](/connectors/mysql), [MongoDB](/connectors/mongo).
- The conceptual companion: [Multi-source data](/concepts/multi-source-data).
- The full admin shape and CDC details:
  [the spec](https://github.com/shaked/engine/blob/main/docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md).
