---
title: Five-minute start
description: Boot kyma with docker compose, send your first row over REST, and run your first KQL query — start to finish in five minutes.
---

# Five-minute start

From zero to your first KQL result. The dev stack is four containers; bringing
it up takes one command.

## Prereqs

- Docker and Docker Compose
- Roughly 2 GB of free RAM
- A terminal

That is the entire list. No language toolchain, no cloud account.

## Step 1: Boot

```bash
git clone https://github.com/shakedaskayo/kyma kyma
cd kyma
docker compose up -d
```

Four containers come up:

- **kyma** — the engine (HTTP `8080`, Arrow Flight gRPC `9090`).
- **postgres** — the externalized catalog (`5433` on the host).
- **minio** — S3-compatible object storage (`9000`, console at `9001`).
- **redpanda** — Kafka-compatible broker for the Kafka ingest path (`9092`).

Wait for the kyma container to log `http server listening`:

```bash
docker compose logs -f kyma | grep -m1 "http server listening"
```

Smoke test the surface:

```bash
curl -sS http://localhost:8080/health
curl -sS http://localhost:8080/metrics | head -5
```

## Step 2: Send a row

`POST /v1/ingest` takes NDJSON. Headers tell the engine which database and
table to land it in. The table is auto-created on first write — no separate
DDL step. Schema evolution is on by default, so any new field in the body
becomes a new column.

```bash
curl -sS -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: default" \
  -H "X-Table: hello" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"_timestamp":"2026-05-02T10:00:00Z","service_name":"demo","message":"hello kyma"}
EOF
```

Response:

```json
{
  "snapshot_id": "...",
  "extent_count": 1,
  "rows_ingested": 1,
  "bytes_written": 4321,
  "replayed": false
}
```

`snapshot_id` is the catalog snapshot the row is now visible in. `replayed`
is `true` only when the same `X-Idempotency-Key` is replayed within the TTL.

## Step 3: Query it

`POST /v1/query` accepts SQL by default and KQL when you set
`Content-Type: application/x-kql`.

```bash
curl -sS -X POST http://localhost:8080/v1/query \
  -H "X-Database: default" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'hello | where service_name == "demo" | take 10'
```

Response is NDJSON — one JSON row per line:

```json
{"_timestamp":"2026-05-02T10:00:00Z","service_name":"demo","message":"hello kyma"}
```

## What just happened

**Ingest.** The REST frontend coerced the NDJSON line into an Arrow
RecordBatch against the table's catalog-stored schema, the staging buffer
group-committed it, and the commit coordinator wrote a snapshot.

**Storage.** The row lives in a columnar extent on MinIO, addressed by
`default.hello.*`. Postgres holds the manifest — per-column stats, time
range, present columns — that the planner uses to prune.

**Query.** The KQL string was parsed by `kyma-kql`, lowered to SQL, and
executed by DataFusion against the registered tables. Three levels of
pruning ran before any extent bytes were decoded.

## Step 4: Ask the agent

Open `http://localhost:5173/agent` (the web app, started by
`pnpm -C web dev`). Pick an engine in `/settings#engine` — Ollama if
you have it running locally, otherwise Claude Code CLI (auto-uses
your local OAuth) or paste an Anthropic / OpenAI API key.

Ask:

> what databases do we have, and how many rows in each?

You'll get a streamed answer.

From your terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash
kyma connect http://localhost:8080 --token "<bearer-token>"
kyma query "what databases do we have?"
```

Add the Kyma skill so Claude Code / Cursor / Aider can ask Kyma:

```bash
kyma install-skill --also-link-claude
```

## Step 5: Ingest a GitHub repo

One command stands up a data source that pulls a repo into a property
graph. The token comes from `$GITHUB_TOKEN`, `$GH_TOKEN`, or `gh auth
token` (whichever is set first):

```bash
kyma create-database github
kyma datasource add github shakedaskayo/kyma --start
```

Now visit `/graph` in the web app — your repo appears as a node-and-
edge graph (commits, branches, PRs, issues, contributors). See
[Data sources → GitHub](/data-sources/github) for the full surface.

## Next

- The mental model: [Concepts](/concepts/)
- The Kyma Agent in depth: [Agent](/agent/)
- One step deeper: [First real run](/quickstart/first-real-run)
- Other ingest paths: [Ingest](/ingest/)
