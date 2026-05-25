# Graphs in kyma

kyma supports graph data and traversal end-to-end. The model is **tabular
graphs over Arrow** — edges are rows in a regular kyma table, traversal is
SQL recursive CTEs or the first-class KQL operators `graph-traverse` and
`graph-shortest-path`. Every hop of a traversal benefits from kyma's
pruning cascade: time-range, equality-index (on `src`/`dst`), and
text-index all apply.

Why tabular-graph, not native-graph? Two reasons:

1. **One engine.** Neo4j-style native graph storage is blazing fast for
   traversal but terrible at everything else (time-range scans, high-
   cardinality aggregations, mixing with metrics/logs). kyma's whole
   premise is a single unified engine; the tabular model makes graphs
   compose with the rest of the language.
2. **Your graph is already tabular.** A distributed-trace edge list, a
   service call graph, an audit log of user→resource permissions, an
   agent-memory entity-relationship table — all of these arrive as rows
   with a source and destination. Storing them as anything else is
   ceremony.

Azure Data Explorer (Kusto) took the same path (`make-graph | graph-match`).
TigerGraph benchmarks show native graph wins at traversal-heavy workloads;
for observability + agent-memory use cases where the graph is mixed with
metrics and logs, tabular is the right answer.

---

## The model

**Pick two columns** as `src` and `dst`. Everything else is edge
properties.

```bash
kyma-cli create-table --db default --name service_calls \
    --schema 'timestamp:timestamp,caller:string,callee:string,latency_ms:int,trace_id:string'
```

That's a graph. Every row is a directed edge `caller → callee` with
properties `timestamp`, `latency_ms`, and `trace_id`.

For **property graphs** (nodes with their own attributes), use two
tables: `nodes(id, label, attributes_json)` and `edges(src, dst, type,
attributes_json)`. The traversal operators join from edges to nodes at
each hop.

For **temporal graphs** (edges valid only during a time window), add
`valid_from:timestamp, valid_to:timestamp` columns and apply time
filters at each hop.

---

## KQL graph operators

### `graph-traverse`

```kql
Edges
| graph-traverse
    source <starting-value>           // single seed: "a"
    source (<v1>, <v2>, …)            // multi-seed: ("a","b","c")
    from   <src-column>
    to     <dst-column>
    max-hops N
    [direction forward|backward|both]
    [edge-type <type-value>]          // prune per hop: only edges where e."type" = <value>
| where depth > 0
| distinct <dst-column>
```

Emits one row per node reachable from `source` within `N` hops. Columns:
`<dst-column>` (the reached node) and `depth` (min hops to reach it).

**Multi-source** — pass a parenthesized list to seed the traversal from several
nodes at once; each seed starts at `depth=0`:
```kql
service_calls | graph-traverse source ("api","worker") from caller to callee max-hops 5
```

**Edge-type filter** — restrict each hop to edges whose `type` column equals
the given value; pruning happens inside the recursive CTE (not on the final result):
```kql
service_calls | graph-traverse source "api" from caller to callee max-hops 5 edge-type "CALLS"
```

**Typical patterns:**

```kql
// All services api transitively depends on
service_calls
| graph-traverse source "api" from caller to callee max-hops 10
| where depth > 0
| distinct callee

// Who called 'billing'? (backward traversal)
service_calls
| graph-traverse source "billing" from caller to callee max-hops 5 direction backward
| where depth > 0

// Per-hop fan-out
service_calls
| graph-traverse source "api" from caller to callee max-hops 5
| summarize n = count() by depth

// 2-hop neighbors only (depth == 2)
service_calls
| graph-traverse source "api" from caller to callee max-hops 2
| where depth == 2
```

### `graph-shortest-path`

```kql
Edges
| graph-shortest-path
    source  <start>
    target  <end>
    from    <src-column>
    to      <dst-column>
    max-hops N
```

Returns a single-row result with `depth` (min hops if reachable) and
`found` (boolean). Unreachable targets produce `found=false, depth=null`.

```kql
service_calls
| graph-shortest-path source "api" target "s3-archive" from caller to callee max-hops 10
```

### Composability

All graph operators produce ordinary tabular results — the full rest of
KQL is available downstream. Chain `where`, `summarize`, `extend`,
`sort`, `take`, `join`, `make-series`.

---

## Performance

### How pruning engages

Every hop is a scan of the edges table filtered by the current frontier:
`WHERE src = <node>` or `WHERE src IN (<frontier>)`. kyma's equality
index (task #38) prunes these to exactly the extents containing rows
with those `src` values. For a service graph with one extent per service
(common: a month of data bucketed by service), a 3-hop traversal from
one service touches 3 services' worth of extents out of the entire
table.

### Current honest limitation

DataFusion's recursive-CTE execution doesn't propagate the frontier
predicate back into the `KymaTable::scan` call for each iteration — the
CTE body re-reads the whole edges table each step. For workloads where
the edges table is small (< 10 GiB, fits comfortably in object-store
cache), this is fine: every recursive step is a second-or-so full scan.
For very large edge sets, we need one of:

- **Frontier-materialized traversal** — compile `graph-traverse` into an
  iterative plan that materializes each hop's frontier, then does one
  filtered scan. Requires a custom physical plan node; a few weeks of
  work.
- **Per-block src/dst indices** — enable block-level pruning so a scan
  with `src IN (frontier)` reads only relevant blocks within each
  extent. Pairs with task #29.
- **Native adjacency-list format** — a `SegmentFormat` that stores
  per-source-node neighbors together. Fastest traversal per hop but
  forfeits columnar-analytics speed on the same data. Opt-in
  per-table, tracked as follow-on Phase D work.

The common case (trace topology within a tenant, service graph for a
cluster, agent memory for one session, dependency graph for one repo)
is small enough that recursive CTE is the right default. The heavy
cases get the native format later.

### Index recommendations

For graph-heavy tables, make sure `src` and `dst` are both typed string
or int columns (not buried in a `dynamic` column). kyma's writer
auto-indexes these, so the query `WHERE src = 'X'` prunes extents
without touching object storage.

---

## Deep-complexity patterns

### Property-graph join (Cypher-equivalent)

```kql
// "Which API calls went from a high-tier user to a payment service?"
edges
| where action == "call"
| graph-traverse source "user:premium-tier" from src_id to dst_id max-hops 3
| where depth > 0
| join kind=inner nodes on $left.dst_id == $right.id
| where label == "payment_service"
| project user_path=source_id, service=id, hops=depth
```

### Cycle detection

```kql
// Edges that form a self-loop (depth > 0 but path returns to source)
edges
| graph-traverse source "A" from src to dst max-hops 10
| where dst == "A" and depth > 0
```

### Subgraph around an incident

```kql
// Service graph restricted to traces from the outage window
service_calls
| where timestamp between (datetime(2026-04-20 02:15) .. datetime(2026-04-20 02:25))
| graph-traverse source "billing" from caller to callee max-hops 3 direction both
```

The `timestamp between` filter runs BEFORE the graph-traverse — our
time-range pushdown limits the edges scanned at each hop to just the
outage window. That's the power of tabular-graph + our pruning cascade
composed.

### Agent memory walk

```kql
// From a conversation turn, find every entity the agent ever interacted
// with, within 4 relationship hops
agent_memory_edges
| where session_id == "sess_a1b2"
| graph-traverse source "prompt:turn-1" from from_entity to to_entity max-hops 4
| where depth > 0
| summarize first_mentioned = min(timestamp) by entity = to_entity
| order by first_mentioned asc
```

This is precisely the ADR-class memory retrieval an LLM agent wants
from its "what did I know about this user earlier in the conversation?"
side-channel. kyma handles it natively because agent memory is, at
heart, a temporal property graph over JSON — which is exactly what
our existing operators cover.

---

## Under the hood: what `graph-traverse` generates

For reference. Given:

```kql
edges | graph-traverse source "api" from caller to callee max-hops 5
```

the generated SQL is:

```sql
WITH RECURSIVE
  _gt(node, depth) AS (
    SELECT CAST('api' AS VARCHAR) AS node, 0 AS depth
    UNION ALL
    SELECT e.callee, t.depth + 1
    FROM _gt t
    JOIN edges e ON e.caller = t.node
    WHERE t.depth < 5
  ),
  _gt_result AS (
    SELECT node AS callee, MIN(depth) AS depth FROM _gt GROUP BY node
  )
SELECT * FROM _gt_result
```

Raw `WITH RECURSIVE` SQL (via HTTP `POST /v1/query` with
`Content-Type: application/sql`) is also supported directly for users
who need Cypher-level expressiveness our KQL operators don't yet
surface.

---

## Not yet supported (future work)

- **Variable-length pattern matching with property predicates at each
  hop** (ADX-style `graph-match (a)-[e*1..5]->(b) where e.status==200`).
  The current `graph-traverse` applies filters only on the preceding
  table; per-hop predicates require richer syntax.
- **Weighted shortest path** (Dijkstra). The current `shortest-path` is
  BFS — treats every edge as weight 1.
- **Pagerank, centrality, community detection** — iterative graph
  analytics. These benefit most from a native adjacency format.
- **Bidirectional search** for shortest-path on large graphs. Current
  implementation is single-direction BFS.

All tracked as follow-on tasks under Phase E.
