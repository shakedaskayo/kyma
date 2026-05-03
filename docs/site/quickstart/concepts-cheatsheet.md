---
title: Concepts cheatsheet
description: One-page reference — the five invariants, data shape, default endpoints, ports, and env vars.
---

# Concepts cheatsheet

A one-screen reference. For the longer-form mental model see
[Concepts](/concepts/); for the living architecture spec see
[Architecture](/architecture/architecture).

## The five invariants

Encoded as architectural tests in `benches/distribution/`. Regressions block
merge.

1. **Object storage is the only source of truth.** Local disk is cache, never master.
2. **Query nodes are stateless.** A node's state is its config + its cache.
3. **Catalog is externalized from byte one.** No embedded-catalog code path.
4. **Format is pluggable.** `SegmentFormat` is the trait boundary.
5. **Parser is pluggable.** `QueryFrontend` is the trait boundary.

Violating any of them turns distribution from a deployment change into a
rewrite.

## Data shape

- **Address:** `database.table.column`
- **Default database:** `default`
- **System columns** — present only on tables synced from external sources
  via the connector framework. Internal kyma tables don't have these.
  - `_kyma_pk` — concatenated source primary key
  - `_kyma_op` — `'insert' | 'update' | 'delete'`
  - `_kyma_lsn` — engine-specific cursor at commit time
  - `_kyma_event_at` — wall-clock time the source emitted the event

## Default endpoints

| Path                    | Method | Content-Type                                              | Auth         | Purpose                                                    |
|-------------------------|--------|-----------------------------------------------------------|--------------|------------------------------------------------------------|
| `/health`               | GET    | —                                                         | none         | Liveness probe.                                            |
| `/metrics`              | GET    | `text/plain` (Prometheus)                                 | none         | Prometheus scrape endpoint.                                |
| `/v1/query`             | POST   | `application/sql`, `application/x-kql`, `application/x-promql` | `Role::Read`  | Run a SQL / KQL / PromQL query. NDJSON response.           |
| `/v1/ingest`            | POST   | `application/x-ndjson`                                    | `Role::Write` | Ingest rows. `X-Database`, `X-Table` headers required.     |
| `/v1/catalog/schema`    | GET    | —                                                         | `Role::Read`  | List databases, tables, columns, types.                    |
| `/v1/agent/ask`         | POST   | `application/json`                                        | `Role::Read`  | Run one agent turn. Streams SSE.                           |
| `/v1/agent/runs/{id}`   | GET    | —                                                         | `Role::Read`  | Look up a persisted agent run by id.                       |
| Arrow Flight gRPC       | —      | Flight protocol                                           | `Role::Read`  | Streaming Arrow results over gRPC on `:9090`.              |

PromQL is on the roadmap — the content-type is reserved; the frontend lands
in a later milestone.

## Default ports

| Port  | Service                                                                    |
|-------|----------------------------------------------------------------------------|
| 8080  | HTTP — query, ingest, agent, health, metrics.                              |
| 9090  | Arrow Flight gRPC.                                                         |
| 4317  | OTLP gRPC (off by default — set `KYMA_OTLP_ADDR` to enable).               |
| 5433  | Postgres catalog (host port; container port is `5432`).                    |
| 9000  | MinIO S3 API.                                                              |
| 9001  | MinIO console UI.                                                          |
| 9092  | Redpanda Kafka wire protocol.                                              |

## Key env vars

Pulled from `kyma-bin/src/main.rs` and the storage / auth modules.

| Name                       | Default                                                | Purpose                                                                  |
|----------------------------|--------------------------------------------------------|--------------------------------------------------------------------------|
| `KYMA_CATALOG_URL`         | `postgres://kyma:kyma_dev@localhost:5433/kyma`         | Postgres catalog connection string.                                      |
| `KYMA_HTTP_ADDR`           | `0.0.0.0:8080`                                         | HTTP listen address.                                                     |
| `KYMA_GRPC_ADDR`           | `0.0.0.0:9090`                                         | Arrow Flight listen address. Set to `off` to disable.                    |
| `KYMA_OTLP_ADDR`           | `off`                                                  | OTLP gRPC listen address (typically `0.0.0.0:4317`). `off` disables it.  |
| `KYMA_OTLP_DATABASE`       | `default`                                              | Database OTLP-received logs land in.                                     |
| `KYMA_AUTH_TOKENS`         | _(empty — auth disabled)_                              | Comma-separated `token:role` pairs. Roles: `admin`, `write`, `read`.     |
| `KYMA_PATH_PREFIX`         | `kyma`                                                 | Object-store key prefix for all extents.                                 |
| `KYMA_S3_ENDPOINT`         | _(unset — uses AWS default)_                           | S3 endpoint. Set to `http://minio:9000` for local MinIO.                 |
| `KYMA_S3_BUCKET`           | `kyma`                                                 | Bucket holding extents.                                                  |
| `KYMA_S3_REGION`           | `us-east-1`                                            | S3 region.                                                               |
| `KYMA_S3_ACCESS_KEY_ID`    | _(unset)_                                              | S3 access key.                                                           |
| `KYMA_S3_SECRET_ACCESS_KEY`| _(unset)_                                              | S3 secret key.                                                           |
| `KYMA_S3_PATH_STYLE`       | `true`                                                 | Path-style addressing (required for MinIO).                              |
| `KYMA_S3_ALLOW_HTTP`       | `false`                                                | Permit non-TLS S3. `true` for local MinIO.                               |

For the full list — including compaction, retention, GC, file-drop, Kafka,
and connector-worker tunables — see [Reference](/reference/).
