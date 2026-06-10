# Cypher in the Smart Input — Design

**Date:** 2026-06-09
**Status:** Approved (autonomous — shared-substrate program, piece 3)
**Scope:** A new Cypher parser that translates a read-only Cypher subset into KQL graph ops, wired into the smart input (detect → `application/x-cypher` → translate → KQL → SQL). Single-hop v1; multi-hop deferred.

## Program context
Piece 3 of the shared-substrate program (4→1→3→2). Pieces 4 + 1 landed. The user's framing: "Cypher in the smart input (new parser → KQL graph ops)."

## Problem
`kyma-kql` already has graph ops (`make-graph`, `graph-match` single-hop, `graph-traverse`), and the smart input detects `search`/`kql`/`sql` (`detectMode.ts`) routing by `content-type`. There is **no Cypher**. Users who know Cypher can't query the graph in the familiar `MATCH (a)-[r]->(b) RETURN …` form.

## Decisions (locked)
| Decision | Choice |
|---|---|
| Translation target | **Cypher → KQL** (reuse `graph-match`), not Cypher → SQL. A new `cypher_to_kql(cypher, binding)` in `kyma-kql` emits a KQL string; the existing `kql_to_sql_with_schemas` finishes the job. |
| Subset (v1) | `MATCH (a[:Label])-[r[:TYPE]]->(b[:Label]) [WHERE <preds>] RETURN <proj> [LIMIT n]` — single-hop, directed or undirected (`-[r]-`), node labels, rel types, `WHERE` simple comparisons joined by `AND`, `RETURN var.prop [AS alias]` or bare `var`, `LIMIT`. Read-only. |
| Deferred (clear ParseError) | Multi-hop `(a)-->(b)-->(c)`, variable-length `-[*1..3]->`, `OPTIONAL MATCH`, multiple `MATCH`, `WITH`, aggregation, `ORDER BY`, mutations (`CREATE/SET/DELETE/MERGE`), `shortestPath`. Each yields a precise "unsupported in v1" error, not a silent wrong result. |
| Graph binding | A Cypher query runs against ONE registered graph. The server resolves it from an `x-graph` header (`"db/graph"`); if absent and exactly one graph is in scope, use it; if ambiguous, error. The resolved `GraphSpec` (node/edge tables + id/src/dst/type/label cols) becomes a `GraphBinding` passed to `cypher_to_kql`. |
| `graph-match` | Stays single-hop in v1 (no multi-hop extension) — keeps the change contained; multi-hop Cypher is the documented follow-up. |
| UI | `detectMode.ts` (separate file) gains `cypher`; the dispatch in `KymaExplore.tsx` (user is co-editing it) gets the **minimal** wiring, applied carefully on top of their WIP, as the last task. |

## Architecture

### 1. `kyma-kql`: Cypher → KQL translator
New `crates/kyma-kql/src/cypher.rs`, re-exported from `lib.rs`:
```rust
pub struct GraphBinding {
    pub edge_table: String,
    pub node_table: String,
    pub id_col: String,     // node id
    pub src_col: String,    // edge src
    pub dst_col: String,    // edge dst
    pub type_col: String,   // edge type
    pub label_col: String,  // node label(s)
}
pub fn cypher_to_kql(cypher: &str, g: &GraphBinding) -> Result<String, ParseError>;
```
- A small self-contained Cypher tokenizer + recursive-descent parser (don't overload the KQL lexer). Parse `MATCH` pattern → `(var [:Label])`, `-[var [:TYPE]]->` / `<-[..]-` / `-[..]-`, `(var [:Label])`; `WHERE` predicate list; `RETURN` projection list; optional `LIMIT`.
- Emit KQL:
  ```
  <edge_table>
  | make-graph <src_col> --> <dst_col> with <node_table> on <id_col>
  | graph-match (a)-[r:TYPE]->(b) project a.<col> as <alias>, ...
  | where <translated node/edge predicates>
  | take <n>
  ```
  - Node label filter (`:Label`) → `where <var>.<label_col> == 'Label'` (or `contains` if labels are multi-valued — match how the stored-graph encodes labels; single-string per the github schema → `==`).
  - Rel `:TYPE` → the `graph-match` `[r:TYPE]` edge-type filter (already supported).
  - Undirected `-[r]-` → emit graph-match without direction constraint if expressible, else translate to the forward form + a note (v1: treat undirected as forward; document).
  - `RETURN a, b` (bare vars) → `project a.*`-style: since KQL `project` needs columns, expand bare vars to a sensible default (id + label + name if present) OR require explicit `var.prop`; v1: bare `var` projects the node's id + label columns; `var.prop` projects that prop.
- Pure function: no catalog/IO. Unit-testable with hand-built `GraphBinding`s. Reuses `ParseError`.

### 2. Server: `application/x-cypher` routing
In `crates/kyma-server/src/lib.rs` `query_handler` (the content-type branch ~810):
- Add `else if content_type.starts_with("application/x-cypher")`:
  - Resolve the target graph: read `x-graph` header (`db/graph`), else the single in-scope graph, else 400 "specify a graph via x-graph".
  - `catalog.get_graph(db, name)` → `GraphSpec` → build `GraphBinding` (map the spec's column roles; `label_col` from the spec).
  - `cypher_to_kql(&raw, &binding)?` → KQL string → `kql_to_sql_with_schemas(&kql, &schemas)?` → execute (same path as KQL).
  - On `ParseError` → 400 with the message (mirrors the KQL error path).
- `language` label = `"cypher"` in the response/telemetry.

### 3. Smart input: detection + dispatch
- `packages/react/src/explore/detectMode.ts`: `ExploreMode` gains `"cypher"`. Detect when the trimmed input matches `^\s*(OPTIONAL\s+)?MATCH\b` (case-insensitive) → `"cypher"`. Add `modeLabel("cypher") = "Cypher"`.
- `packages/client/src/query.ts`: `language` gains `"cypher"` → `content-type: application/x-cypher`; pass the selected graph as the `x-graph` header (thread a `graph?` arg through `runQuery`/the ticket).
- `packages/react/src/explore/KymaExplore.tsx` (minimal, careful, last): route `mode === "cypher"` to `execute({ ..., language: "cypher", graph })`; show the `Cypher` badge; supply the current graph context (reuse the page's graph/database scope, or default). Keep the user's `autoRun` WIP intact.

## Error handling
- Unsupported construct → `ParseError("cypher: <feature> is not supported yet (v1 single-hop MATCH)")`. Never silently mistranslate.
- No graph resolvable → 400 with guidance.
- Unknown node/edge prop → let the SQL layer surface it (same as KQL).

## Testing
- **kyma-kql unit (the core):** `cypher_to_kql` over a fixed `GraphBinding` — single-hop directed; `:Label`/`:TYPE` filters; `WHERE a.p = 'x' AND b.q > 3`; `RETURN a.name AS n, b.id`; `LIMIT`; undirected; bare-var RETURN. Each asserts the emitted KQL string (and that it then compiles via `kql_to_sql_with_schemas`). Unsupported constructs (multi-hop, `CREATE`, `*`) return the precise error.
- **Server integration:** POST `/v1/query` with `content-type: application/x-cypher` + `x-graph` over a seeded graph returns rows; missing-graph → 400; bad Cypher → 400 with message.
- **Client/React:** `detectMode("MATCH (a)-[r]->(b) RETURN a")` → `"cypher"`; `runQuery({language:"cypher", graph})` sends `application/x-cypher` + `x-graph`. KymaExplore renders cypher results (reuses the table renderer).

## Out of scope / deferred
- Multi-hop / variable-length paths (+ extending `graph-match` to N-hop).
- `OPTIONAL MATCH`, `WITH`, aggregation, `ORDER BY`, mutations, `shortestPath`.
- Cross-graph Cypher (one graph per query in v1).

## File touch list
- `crates/kyma-kql/src/cypher.rs` (new) + `crates/kyma-kql/src/lib.rs` (re-export).
- `crates/kyma-server/src/lib.rs` (`x-cypher` branch + graph resolution).
- `packages/react/src/explore/detectMode.ts` (+ test).
- `packages/client/src/query.ts` (`cypher` language + `x-graph`).
- `packages/react/src/explore/KymaExplore.tsx` (minimal dispatch + badge — careful, on top of co-working WIP).
