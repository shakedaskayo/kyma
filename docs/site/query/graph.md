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
  (foreign-key relationships, connector-produced `*_id` columns, and so on).
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
query, deduplicating append-only connector rows to one canonical node per id.

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

### CI logs (GitHub connector)

Every GitHub CI job log captured by the [GitHub connector](/connectors/github) produces an `Artifact` node in the `github` graph, linked from its `Job` node by a `HAS_ARTIFACT` edge. Node properties:

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

**Redaction note:** The GitHub connector redacts secrets from CI log text before writing the blob to object-store (`kyma-redact` runs at capture time; raw log text is never persisted). What you retrieve from the viewer is already redacted.

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

Full-text search across artifact blob contents is handled by the unified search endpoint (`POST /v1/search`), not the graph page.

---

## Where to go next

- KQL graph operators in detail: [KQL functions](/reference/kql-functions).
- Full graph HTTP endpoints: [HTTP API](/reference/api).
- How the agent uses the graph: [Agentic Memory](/agent/memory).
- Background workers that keep graphs current: [Workers & nodes](/agent/workers).
- Why traversals are fast: [The pruning cascade](/concepts/the-pruning-cascade).
