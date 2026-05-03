---
title: Arrow Flight
description: Zero-copy gRPC query surface on :9090. Submit a JSON ticket via do_get, stream Arrow RecordBatches back to any Flight client — Python, Rust, JS over gRPC-web.
---

# Arrow Flight

When the HTTP NDJSON response is the bottleneck — streaming millions of
rows into a Polars DataFrame, feeding a Spark connector, talking to an
Arrow-native client — Arrow Flight is what you want. It's a gRPC service
that returns query results as raw Arrow `RecordBatch`es over the wire.
No JSON encode, no JSON decode, no row-by-row materialization.

## Endpoint

```
KYMA_GRPC_ADDR  default: 0.0.0.0:9090
```

Set `KYMA_GRPC_ADDR=off` to disable the gRPC listener entirely (useful
on deployments where only HTTP is exposed). The HTTP query surface keeps
working either way.

The compose stack in [Five-minute start](/quickstart/five-minute-start)
brings this up automatically alongside HTTP `8080`.

## Protocol

kyma implements the standard Arrow Flight RPC service. Clients use any
Arrow Flight client — `pyarrow.flight`, the Rust `arrow-flight` crate,
the Java `flight-core` package, browser clients via gRPC-web.

The implemented surface is intentionally minimal:

| RPC                | Behaviour                                                 |
| ------------------ | --------------------------------------------------------- |
| `do_get`           | Execute a query ticket; stream `FlightData` back.         |
| `handshake`        | Accepted unauthenticated for now (token via metadata).    |
| `do_put`           | `Unimplemented` — use `POST /v1/ingest` instead.          |
| `do_action`        | `Unimplemented`.                                          |
| `list_flights`     | `Unimplemented` — issue `do_get` directly with a ticket.  |
| `get_flight_info`  | `Unimplemented` — same as above.                          |

Full Flight-SQL compliance (`get_flight_info`, prepared statements,
catalog discovery) is on the roadmap; today the path is just `do_get`
with a JSON ticket.

## The ticket

A Flight ticket is opaque bytes. kyma defines a tiny JSON envelope:

```json
{
  "database": "default",
  "query": "SELECT service_name, COUNT(*) FROM otel_logs WHERE _timestamp > now() - INTERVAL '1 hour' GROUP BY service_name",
  "language": "sql"
}
```

`database` defaults to `"default"`. `language` is `"sql"` (default) or
`"kql"`. The query string is the same body you'd `POST` to
`/v1/query` — the underlying executor is the same DataFusion
`SessionContext` either way.

## Auth

Bearer tokens travel in gRPC metadata under the `authorization` key,
exactly the same `KYMA_AUTH_TOKENS` value as the HTTP path. With auth
disabled (`KYMA_AUTH_TOKENS` empty), the Flight surface is open — the
same bypass as HTTP.

```python
import pyarrow.flight as fl

client = fl.connect("grpc://localhost:9090")
options = fl.FlightCallOptions(
    headers=[(b"authorization", b"Bearer reader-tok")]
)
```

## Example: Python

```python
import json
import pyarrow.flight as fl

client = fl.connect("grpc://localhost:9090")

ticket = fl.Ticket(json.dumps({
    "database": "default",
    "query": (
        "SELECT service_name, severity_text, COUNT(*) AS n "
        "FROM otel_logs "
        "WHERE _timestamp > now() - INTERVAL '1 hour' "
        "GROUP BY service_name, severity_text "
        "ORDER BY n DESC"
    ),
    "language": "sql",
}).encode())

reader = client.do_get(ticket)
table = reader.read_all()  # arrow.Table — zero-copy from the stream
print(table.to_pandas())
```

`reader.read_all()` consumes the entire stream into a single Arrow
`Table`. For multi-gigabyte results, iterate `reader` chunk-by-chunk
instead and keep memory bounded.

## Example: Rust

```rust
use arrow_flight::{flight_service_client::FlightServiceClient, Ticket};
use serde_json::json;

let mut client = FlightServiceClient::connect("http://localhost:9090").await?;
let body = json!({
    "database": "default",
    "query":    "SELECT * FROM otel_logs LIMIT 1000",
    "language": "sql",
});
let mut stream = client
    .do_get(Ticket { ticket: body.to_string().into() })
    .await?
    .into_inner();
while let Some(flight_data) = stream.message().await? {
    // decode flight_data into a RecordBatch via the arrow-flight decoder
}
```

## TLS in production

gRPC plaintext is fine on a trusted network. In production, terminate
TLS at the right layer:

- **Cloud Run / App Runner / Fargate** — the platform's load balancer
  handles TLS; kyma sees plaintext gRPC inside the network.
- **Self-managed** — front kyma with a reverse proxy that terminates
  TLS and forwards gRPC (Envoy, Caddy with `reverse_proxy ... h2c`,
  Nginx with `grpc_pass`).

The Flight server itself doesn't yet do TLS termination directly — that
landing is part of the auth-hardening pass.

## gRPC-web for browsers

When the `web-ui` build feature is on, the same Flight service is
exposed at `/flight/*` over gRPC-web — browsers can call it from
JavaScript without a sidecar. The auth model is identical: bearer
token in the `Authorization` header (axum middleware enforces it
before the gRPC layer sees the request). Native gRPC clients should
keep using `:9090`; gRPC-web is for the browser.

## Where to go next

- SQL syntax and DataFusion features: [SQL](/query/sql).
- Why "zero-copy" actually means zero copy: [Extents and
  snapshots](/concepts/extents-and-snapshots).
- The full pruning-cascade story behind every Flight result: [The
  pruning cascade](/concepts/the-pruning-cascade).
- Other ingest/query surfaces: [Query](/query/).
