---
title: CLI
description: kyma-cli reference — every subcommand, every flag, the schema-spec syntax, and the one env var that matters.
---

# CLI

`kyma-cli` is the administrative CLI. It talks straight to the
Postgres catalog — it does not require a running engine — so you
can use it from a terminal, a shell script, or a CI step that's
provisioning a fresh database.

## Global flags

Every subcommand inherits these.

| Flag             | Env var              | Default                                                | Purpose                       |
| ---------------- | -------------------- | ------------------------------------------------------ | ----------------------------- |
| `--catalog-url`  | `KYMA_CATALOG_URL`   | `postgres://kyma:kyma_dev@localhost:5433/kyma`         | Postgres connection URL.      |

In production setups the env-var form is the one you want — set
`KYMA_CATALOG_URL` once in your shell and every subsequent
`kyma-cli` call picks it up.

## Subcommands

### `create-database <name>`

Create a new database (kyma's namespace inside the catalog). Returns
the database id.

```bash
kyma-cli create-database analytics
# created database analytics (a3f1...)
```

### `create-table --db <name> --name <name> --schema <spec> [--retention-days N]`

Create a new table in the named database. The `--schema` argument is
a column-spec string: comma-separated `name:type` pairs.

| Flag                | Required | Notes                                                                |
| ------------------- | -------- | -------------------------------------------------------------------- |
| `--db`              | yes      | Target database. Must already exist.                                 |
| `--name`            | yes      | Table name.                                                          |
| `--schema`          | yes      | `col:type[, col:type, ...]`.                                         |
| `--retention-days`  | no       | TTL in days. Extents past TTL are soft-deleted by the retention sweeper. |

Schema-spec types:

| Type         | Arrow logical type                             | Nullable | Notes                                                       |
| ------------ | ---------------------------------------------- | -------- | ----------------------------------------------------------- |
| `bool`       | `Boolean`                                      | yes      |                                                             |
| `int`        | `Int32`                                        | yes      |                                                             |
| `long`       | `Int64`                                        | yes      |                                                             |
| `real`       | `Float64`                                      | yes      |                                                             |
| `string`     | `Utf8`                                         | yes      |                                                             |
| `timestamp`  | `Timestamp(Nanosecond, None)`                  | yes      | UTC; RFC3339 strings or epoch nanos accepted on ingest.     |
| `dynamic`    | `Binary`                                       | yes      | Coerced JSON; queryable via path syntax.                    |
| `vector(N)`  | `FixedSizeList<Float32, N>`                    | **no**   | Null-vector ingest is rejected; all vector columns are non-nullable. |

Example:

```bash
kyma-cli create-table \
  --db analytics \
  --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
# created table analytics.pageviews (b0c4...)
```

### `alter-table --db <name> --table <name> --add-column <spec>`

Add a single nullable column to an existing table. `<spec>` is one
`name:type` pair using the same type set as `create-table` (bool,
int, long, real, string, timestamp, dynamic).

```bash
kyma-cli alter-table --db analytics --table pageviews \
  --add-column "referrer:string"
# altered analytics.pageviews: added column referrer:string (schema_snapshot=4)
```

Schema only widens — there is no `drop-column`, no `rename-column`,
no type narrowing. See [Schema model](/concepts/schema-model) for
the rationale.

### `list-tables --db <name>`

List tables in a database. Each row prints the table name followed by
its schema as Arrow logical types.

```bash
kyma-cli list-tables --db analytics
# pageviews  [_timestamp:Timestamp(Nanosecond, None), user_id:Utf8, path:Utf8, ms:Int32, referrer:Utf8]
```

If the database has no tables, prints `(no tables in database <name>)`.

### `version`

Print the CLI version (the cargo package version it was built with)
and exit.

```bash
kyma-cli version
# kyma-cli 0.1.0
```

## Typical onboarding flow

```bash
export KYMA_CATALOG_URL=postgres://kyma:kyma_dev@localhost:5433/kyma

kyma-cli create-database analytics
kyma-cli create-table \
  --db analytics --name pageviews \
  --schema "_timestamp:timestamp, user_id:string, path:string, ms:int" \
  --retention-days 30
kyma-cli list-tables --db analytics
```

After this, `POST /v1/ingest` against the running engine with
`X-Database: analytics` and `X-Table: pageviews` will land rows in
the table you just created. With auto-create on (the default) you
can also skip this whole step — the first `POST /v1/ingest` provisions
the database and table on demand.
