---
title: Graph
description: kyma's native graph layer — schema graph (synthetic, always on) and stored graphs (registered). Query with KQL operators, MCP tools, or the HTTP API. All graphs render together on the unified /graph canvas.
---

# Graph

kyma includes a native graph layer over the same columnar store. Entities and
relationships are first-class — indexed, pruned, and served through the same
Arrow execution path as every other query surface. There is no separate graph
database; the graph is a view over tables you already have.

Two kinds of graph exist side-by-side in every deployment.

## The schema graph

The schema graph is synthetic and always available — nothing to register, nothing
to configure. kyma derives it from the catalog at query time:

- Every table in a database becomes a node.
- Inferred `REFERENCES` edges connect tables whose columns share value domains
  (foreign-key relationships, data-source-produced `*_id` columns, and so on).
- Column details are available behind a depth toggle in the web UI.

The **realm** of a schema-graph node is the database name, so a multi-database
deployment produces one schema graph per database, merged on the
[unified canvas](#the-graph-in-the-web-ui).

The schema graph has no storage tables of its own. The `<graph>_nodes` and
`<graph>_edges` tables that stored graphs use are deliberately excluded from
the schema graph to keep the view clean.

## Stored graphs

A stored graph is a property-graph you register against two tables — one for
nodes, one for edges. Once registered, kyma reads those tables on every graph
query, deduplicating append-only data source rows to one canonical node per id.

### Register a graph

```bash
kyma create-graph \
  --db <database-name> \
  --name <graph-name> \
  --nodes <node-table> \
  --edges <edge-table>
```

Key flags and their defaults:

| Flag | Default | Purpose |
| --- | --- | --- |
| `--id-col` | `id` | Node identity column in the nodes table. |
| `--label-col` | `labels` | Node label / kind column. |
| `--src-col` | `src` | Edge source column in the edges table. |
| `--dst-col` | `dst` | Edge destination column. |
| `--type-col` | `type` | Edge type / relationship column. |
| `--realm-col` | *(none)* | Optional column used to scope nodes into sub-realms. |

If your tables already use those column names, you only need the four required
flags. Override individual names if they differ.

### List and drop

```bash
kyma list-graphs --db <database-name>
kyma drop-graph  --db <database-name> --name <graph-name>
```

Drop removes the registration; it does not delete the underlying tables.

### What node and edge tables look like

A minimal nodes table:

| `id` | `labels` | … any other columns … |
| --- | --- | --- |
| `svc-a` | `service` | … |

A minimal edges table:

| `src` | `dst` | `type` | … |
| --- | --- | --- | --- |
| `svc-a` | `svc-b` | `CALLS` | … |

Any extra columns on both tables are carried through as properties and are
accessible in KQL `graph-match` predicates and in the HTTP response.

## Querying graphs

### KQL graph operators

KQL exposes three graph operators documented in full at
[KQL functions](/reference/kql-functions): `make-graph`, `graph-match`,
and `graph-traverse` (with optional edge-type filtering), plus
`graph-shortest-path`.

A quick `graph-traverse` example — walk outbound edges from `svc-a` up to
two hops:

```kql
context_edges
| graph-traverse source "svc-a" from src to dst max-hops 2
```

The result is a row per reachable node with its hop depth. Add `| where` and
`| project` downstream to filter or reshape the output.

See [KQL](/query/kql) for the broader operator reference.

### Cypher

For graph-shaped queries you can send **Cypher** instead of KQL. `POST /v1/query`
with `Content-Type: application/x-cypher`, the raw Cypher as the body, and an
`X-Graph` header naming the graph (`"<db>/<graph>"`, or just `"<graph>"` when it
falls back to `X-Database`). Cypher is translated to KQL and runs through the
same Arrow execution path — so it inherits the [pruning cascade](/concepts/the-pruning-cascade)
and the persisted CSR snapshot for deep traversals.

```bash
curl -sS localhost:8080/v1/query \
  -H 'Content-Type: application/x-cypher' \
  -H 'X-Graph: obs/kg' \
  --data-binary "MATCH (a:service)-[:CALLS*1..3]->(b)
                 WHERE a.name STARTS WITH 'api-' AND (b.tier = 'db' OR NOT b.healthy = 'true')
                 RETURN a.name, b.name ORDER BY a.name LIMIT 50"
```

Supported surface:

| Area | Supported |
| --- | --- |
| **Patterns** | `MATCH` with forward `-->`, backward `<--`, and undirected hops; multi-hop chains; comma-separated and multiple `MATCH` clauses (star / tree / disjoint, joined on shared variables); relationship type + node label filters (`-[r:CALLS]->`, `(a:service)`). |
| **Variable length** | `-[r*min..max]->` (bounded), lowered to a depth-bounded recursive traversal that preserves both endpoints. |
| **Paths** | `shortestPath((a)-[*..n]->(b))` with `length(p)`; endpoints pinned by `WHERE` id equality. |
| **Optional** | `OPTIONAL MATCH` (lowered as a LEFT JOIN). |
| **WHERE** | `=`, `<>`, `<`, `>`, `<=`, `>=`; `IN [..]`; `STARTS WITH` / `ENDS WITH` / `CONTAINS`; `AND` / `OR` / `NOT` with parentheses and correct precedence. |
| **RETURN** | properties, `AS` aliases, `DISTINCT`, aggregates `count(*)` / `count(x)` / `sum` / `min` / `max`, `ORDER BY … [ASC\|DESC]`, `LIMIT`. |

Not yet supported (returns `400` with the parse error): `WITH`, write clauses
(`CREATE` / `MERGE` / `SET` / `DELETE`), `RETURN *`, and arbitrary expressions in
`RETURN`. Use KQL for anything outside this surface.

### MCP tools

When querying through the MCP server, two graph tools are available:

- **`graph_traverse`** — walks from a source node across an edge table with a
  configurable hop limit.
- **`find_references_to`** — locates every node that references a given value
  across all columns in the catalog.

Both tools are wired to the same graph engine as KQL; the MCP layer adds
natural-language routing on top.

### HTTP API

All graph endpoints are documented at [HTTP API](/reference/api). The database
is selected with the `X-Database` header on every request. The short list:

- `GET /v1/graph` — list registered graphs.
- `GET /v1/graph/:graph/overview|stats|schema` — metadata.
- `GET /v1/graph/:graph/nodes/:id` — single node.
- `GET /v1/graph/:graph/nodes/:id/subgraph` — node + neighbourhood.
- `POST /v1/graph/:graph/search` — predicate-filtered node search.
- `POST /v1/graph/:graph/neighbors` — neighbours of a node.
- `GET /v1/graph/:graph/analytics` — whole-graph analytics over the persisted
  CSR topology (stored graphs only). `?kind=pagerank|communities|components`;
  `pagerank` also takes `&seeds=<id,id,…>` (empty ⇒ global PageRank) and
  `&limit=N` (top-N by score, default 100). Returns
  `{"kind", "count", "results":[{"id", "value"}]}` — `value` is the PageRank
  score, community id, or component id.

## The graph in the web UI

The `/graph` route in the web app renders a **unified canvas** that merges
every source at once:

- The schema graph for every connected database.
- Every registered stored graph across all databases.
- The agentic memory layer (see below), which stores its own node and edge
  tables and appears as a registered graph with cross-graph edges.

The canvas is deliberately not scoped to a single database or session — the
value of the graph view is seeing the full topology of your deployment in one
place.

## How memory uses the graph

[Agentic Memory](/agent/memory) persists `memory_nodes` and `memory_edges`
tables and links individual memories to catalog graph nodes via `REFERENCES`
and `RESOLVES_TO` edges. This means memories surface on the graph canvas
alongside your service and data topology — a memory about a table appears
adjacent to that table's node, and traversals can cross the memory–catalog
boundary in both directions.

See [Agentic Memory](/agent/memory) for how memories are created, searched,
and linked, and [Workers & nodes](/agent/workers) for the background processes
that populate and maintain the memory graph.

## Artifacts on the graph

Artifacts — CI job logs, object-store blobs, agent-contributed files, filesystem-watch snapshots — are first-class graph nodes labeled `Artifact`.

### CI logs (GitHub data source)

Every GitHub CI job log captured by the [GitHub data source](/data-sources/github) produces an `Artifact` node in the `github` graph, linked from its `Job` node by a `HAS_ARTIFACT` edge. Node properties:

| Property | Value |
| --- | --- |
| `object_path` | Object-store path of the log blob. |
| `sha256` | SHA-256 hex digest of the blob. |
| `size_bytes` | Blob size in bytes. |
| `artifact_class` | `log` |
| `source` | `github` |
| `retrievable` | `true` — set at capture (a blob was stored for this log). |
| `artifact_id` | Catalog row id (UUID). |

**Forward-only relabel note:** CI logs captured before this change keep their old `LogFile` label in the append-only tables; only newly captured logs carry the `Artifact` label. Re-capture a job to get the new label.

**Redaction note:** The GitHub data source redacts secrets from CI log text before writing the blob to object-store (`kyma-redact` runs at capture time; raw log text is never persisted). What you retrieve from the viewer is already redacted.

### Other artifact sources (server / Postgres mode)

Object-store blobs, agent-contributed files, and filesystem-watch snapshots that have no matching producer node appear as `Artifact` nodes in a dedicated `artifacts` graph. This graph is materialized on startup and kept current by a periodic sync running in the server process.

**Availability:** server + Postgres mode only. `kyma local` has no artifact catalog and no `artifacts` graph.

### Viewing artifacts on the graph page

1. Open `/graph` (the unified canvas).
2. Filter by the `Artifact` node type using the label filter in the sidebar — this narrows the canvas to all artifact nodes across both the `github` graph and the `artifacts` graph.
3. Click any `Artifact` node to open the sidebar viewer. The viewer streams the stored blob bytes as-is (via `GET /v1/artifacts/by-path?path=<object_path>`, paged with **Load more**).

### Searching for artifacts

Graph search matches on node id and label. Searching `artifact` returns all `Artifact` nodes. Searching a repo name, job name, or log path substring surfaces matching nodes if they appear in the node id.

### Content search

Artifact blob **contents** are searchable through the unified search endpoint (`POST /v1/search`) and the Explore page, not the graph page.

On the same cadence as the artifact-graph sync (`KYMA_ARTIFACT_GC_POLL_SECS`, default 300s), the server indexes text-like artifacts into the `artifacts.artifact_chunks` table:

- **Which artifacts:** classes `log`, `file`, `text`, `doc`, `config`; up to 4 MB per blob; binary content is sniffed and skipped.
- **How:** the blob is split into line-aligned ~1500-char chunks (max 64 per artifact), embedded with the process-shared embedding backend, and appended with the artifact's `artifact_id`, `object_path`, `source`, and `table_ref` for provenance.
- **Search:** `artifact_chunks` is a regular table in the `artifacts` database, so default data-mode search (`{"query": "..."}`) hits it with both the lexical and the vector (cosine) leg. Hits carry `"source": "artifacts.artifact_chunks"` and the chunk row, including `artifact_id` to retrieve the full blob.
- **Idempotent:** each artifact is indexed once per content hash (`sha256`); re-syncs skip already-indexed artifacts before fetching the blob.

**Availability:** server + Postgres mode only, same as the artifact catalog.

---

## Where to go next

- KQL graph operators in detail: [KQL functions](/reference/kql-functions).
- Full graph HTTP endpoints: [HTTP API](/reference/api).
- How the agent uses the graph: [Agentic Memory](/agent/memory).
- Background workers that keep graphs current: [Workers & nodes](/agent/workers).
- Why traversals are fast: [The pruning cascade](/concepts/the-pruning-cascade).
