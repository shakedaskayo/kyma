---
title: Microsoft Fabric
description: Query Fabric Lakehouse and Warehouse tables live over the SQL analytics endpoint — schemas cataloged in kyma, data stays in Fabric, whole subplans pushed down as T-SQL.
---

# Microsoft Fabric

`type`: `"msfabric"`. The first **federated** connector: it never ingests
rows. A periodic metadata sync mirrors the remote tables into a kyma
database as *federated tables* (schema only), and queries against them
execute live on Fabric's compute — filters, joins between two Fabric
tables, and aggregations are pushed down as a single T-SQL statement
over the [SQL analytics endpoint](https://learn.microsoft.com/en-us/fabric/data-engineering/lakehouse-sql-analytics-endpoint)
(TDS). Works for Lakehouse, Warehouse, and SQL DB items.

## Prerequisites

1. An Entra ID **service principal** (app registration + client secret).
2. The Fabric tenant setting **"Service principals can use Fabric APIs"**
   enabled (Admin portal → Tenant settings → Developer settings), and the
   principal granted access to the workspace (Viewer is enough).
3. The item's SQL analytics endpoint host — Fabric portal → your
   Lakehouse/Warehouse → *SQL analytics endpoint* → copy the connection
   string host, e.g. `xxxxxxxx.datawarehouse.fabric.microsoft.com`.

Everything below is also available in the web UI: add a **Service
principal** credential under *Settings → Credentials*, then pick
*Microsoft Fabric* in the connector wizard — it prompts for the endpoint,
item name, stored credential, and the wildcard opt-out. The connector
detail page shows sync runs, last error, and lets you swap the linked
credential.

## Store the credential

```bash
curl -sS -X POST http://localhost:8080/v1/credentials \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "label": "fabric-prod-sp",
  "value": {
    "kind": "service_principal",
    "tenant_id": "<entra-directory-id>",
    "client_id": "<app-client-id>",
    "client_secret": "<client-secret>"
  }
}
JSON
```

Response: `{"id": "<credential-uuid>", ...}`.

## Register the connector

```bash
curl -sS -X POST http://localhost:8080/v1/connectors \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "fabric_prod",
  "type": "msfabric",
  "target_database": "fabric_prod",
  "config": {
    "endpoint": "xxxxxxxx.datawarehouse.fabric.microsoft.com",
    "database": "my_lakehouse",
    "credential_id": "<credential-uuid>",
    "exclude_from_wildcard": false
  }
}
JSON
```

Each sync tick (default every 30 min) walks `INFORMATION_SCHEMA` on the
endpoint and reconciles `target_database`:

- new remote tables appear as federated tables (bare name for the `dbo`
  schema, `schema__table` otherwise);
- schema drift drops + recreates the catalog entry (no data is touched —
  there is none in kyma);
- tables that disappear remotely are dropped;
- a name collision with a non-federated kyma table is skipped with a
  warning — ingested data is never clobbered.

It also registers an `msfabric` context graph
(`FabricEndpoint → FabricItem → FabricTable`) for discovery and lineage.

## Query it

Federated tables look like any other table — query editor, `POST
/v1/query`, agents' `run_sql`, and the cross-database `x-database: *`
union all work:

```sql
-- single database (x-database: fabric_prod)
SELECT customer_id, sum(amount) AS total
FROM   orders
WHERE  order_date >= '2026-01-01'
GROUP  BY customer_id
ORDER  BY total DESC
LIMIT  20;
```

The whole statement compiles to one T-SQL query and runs on Fabric;
kyma streams back only the result rows. A join between a Fabric table
and a local kyma table runs the Fabric side remotely and the join
locally.

`describe_table` / `explore_schema` mark these tables with
`"federated": {"platform": "msfabric", ...}` so agents know queries hit
a live remote system.

## Guardrails

Remote queries run on your Fabric capacity. Server-wide bounds, env-tunable:

| Env var | Default | Meaning |
|---|---|---|
| `KYMA_FEDERATION_TIMEOUT_MS` | `30000` | Wall-clock budget per remote query |
| `KYMA_FEDERATION_MAX_ROWS` | `100000` | Row cap per remote result (truncates + warns) |
| `KYMA_FEDERATION_MAX_CONCURRENT` | `4` | In-flight remote queries per source |

Set `"exclude_from_wildcard": true` to keep the source out of
`x-database: *` fan-outs; explicit queries still work.

## Type mapping

| T-SQL | Arrow |
|---|---|
| `bit` | `Boolean` |
| `tinyint` / `smallint` / `int` | `Int32` |
| `bigint` | `Int64` |
| `real` | `Float32` |
| `float` | `Float64` |
| `decimal` / `numeric` / `money` | `Float64` (precision loss past 2^53) |
| `date` | `Date32` |
| `datetime` / `datetime2` / `smalldatetime` | `Timestamp(µs)` |
| `datetimeoffset` | `Timestamp(µs, UTC)` |
| everything else (`varchar`, `uniqueidentifier`, `varbinary` as hex, …) | `Utf8` |

## Limitations

- SQL only for now — KQL/Cypher frontends don't run against federated
  tables yet.
- `OFFSET` without `ORDER BY` is not expressible in T-SQL and surfaces a
  remote error; add an `ORDER BY`.
- The SQL analytics endpoint's Delta→SQL metadata sync can lag a recent
  OneLake write by a short window; the next connector tick (or Fabric's
  metadata refresh API) catches it up.
- Databricks, Snowflake, and BigQuery follow as further platforms on the
  same federation substrate.
