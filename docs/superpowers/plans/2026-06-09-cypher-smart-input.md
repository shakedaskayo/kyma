# Cypher in the Smart Input — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. `- [ ]` checkboxes.

**Goal:** Translate a read-only Cypher subset to KQL graph ops, wired into the smart input (detect → `application/x-cypher` → translate → KQL → SQL). Single-hop v1.

**Spec:** `docs/superpowers/specs/2026-06-09-cypher-smart-input-design.md`.

**Co-working note:** the user is actively editing `PensieveExplore.tsx` — Task 4 touches it minimally + carefully; Tasks 1–3 avoid it. Never stage other uncommitted files.

---

### Task 1: `pensieve-kql` Cypher → KQL translator (the core; pure + fully unit-testable)

**Files:** `crates/pensieve-kql/src/cypher.rs` (new), `crates/pensieve-kql/src/lib.rs` (re-export).

- [ ] **Step 1** — Read `crates/pensieve-kql/src/parser.rs` `op_make_graph` (~777) and `op_graph_match` (~804) to learn the EXACT `make-graph`/`graph-match`/`project` syntax to emit, and `crates/pensieve-kql/src/lib.rs` for `ParseError`, `kql_to_sql_with_schemas`, `SchemaMap`.
- [ ] **Step 2** — Failing tests in `cypher.rs` `#[cfg(test)]`. Define a fixture `GraphBinding { edge_table:"github_edges", node_table:"github_nodes", id_col:"id", src_col:"src", dst_col:"dst", type_col:"type", label_col:"labels" }`. Assert `cypher_to_kql(...)` emits the expected KQL for:
  - `MATCH (a)-[r:HAS_ARTIFACT]->(b) RETURN a.name, b.name` → `github_edges | make-graph src --> dst with github_nodes on id | graph-match (a)-[r:HAS_ARTIFACT]->(b) project a.name as a_name, b.name as b_name`.
  - `:Label` filter, `WHERE a.x = 'v' AND b.n > 3`, `RETURN a.name AS n`, `LIMIT 10`, undirected `-[r]-`, bare-var `RETURN a`.
  - Unsupported → precise `Err(ParseError(...))`: multi-hop `(a)-->(b)-->(c)`, `CREATE (...)`, var-length `-[*]->`.
  - For ≥2 representative cases, also assert the emitted KQL compiles: `kql_to_sql_with_schemas(&kql, &SchemaMap::default()).is_ok()` (or with a minimal schema map). Run `cargo test -p pensieve-kql cypher` → FAIL.
- [ ] **Step 3** — Implement `cypher.rs`: a small Cypher tokenizer (idents, `(`,`)`,`[`,`]`,`-`,`>`,`<`,`:`,`.`,`,`,`=`,`<`,`>`,`<=`,`>=`,`<>`, string/number literals, keywords MATCH/OPTIONAL/WHERE/RETURN/AS/AND/LIMIT) + recursive-descent parse of the v1 subset (spec §1) → emit the KQL string. Map: node `:Label` → `where <var>.<label_col> == 'Label'`; rel `:TYPE` → graph-match `[r:TYPE]`; `WHERE` comparisons → KQL `where <var>.<col> <op> <lit>` (`=`→`==`, `<>`→`!=`); `RETURN var.col [AS alias]` → `project var.col as alias` (default alias `var_col`); bare `var` → `project var.<id_col> as var_id, var.<label_col> as var_label`; `LIMIT n` → `take n`. Reject deferred constructs with `ParseError`. `pub use cypher::{cypher_to_kql, GraphBinding};` in `lib.rs`.
- [ ] **Step 4** — `cargo test -p pensieve-kql cypher` (pass) + `cargo build -p pensieve-kql`. Commit: `feat(kql): Cypher → KQL translator (single-hop MATCH subset)`.

---

### Task 2: Server `application/x-cypher` routing + graph resolution

**Files:** `crates/pensieve-server/src/lib.rs` (the `query_handler` content-type branch ~810).

- [ ] **Step 1** — Read the content-type branch + how `x-database` is read (~684) + how graphs are resolved (`catalog.get_graph` / `graph_handler::resolve_with`). Read `pensieve_core::catalog::GraphSpec` for the column-role field names.
- [ ] **Step 2** — Failing integration test (`crates/pensieve-server/tests/`, mirroring an existing `/v1/query` test if present; else a focused handler test): seed a stored graph; POST `/v1/query` with `content-type: application/x-cypher`, `x-graph: <db>/<graph>`, body `MATCH (a)-[r]->(b) RETURN a.name LIMIT 5` → 200 with rows; missing `x-graph` + ambiguous → 400; bad Cypher → 400 with the ParseError message. (If full HTTP harness is heavy, test the extracted resolution+translate fn directly.)
- [ ] **Step 3** — Implement: extract a helper `fn resolve_graph_binding(catalog, tenant, x_graph, in_scope_graphs) -> Result<GraphBinding, Resp>`; in `query_handler` add `else if content_type.starts_with("application/x-cypher") { let binding = resolve_graph_binding(...)?; let kql = pensieve_kql::cypher_to_kql(&raw, &binding).map_err(400)?; let sql = pensieve_kql::kql_to_sql_with_schemas(&kql, &schemas).map_err(400)?; ("cypher", sql) }`. Map `GraphSpec` → `GraphBinding` (id_col, label_col, src_col, dst_col, type_col, node_table, edge_table).
- [ ] **Step 4** — `cargo test -p pensieve-server` (new test passes; existing query tests pass) + build. Commit: `feat(server): route application/x-cypher through cypher_to_kql + graph resolution`.

---

### Task 3: Smart-input detection + client wiring (no PensieveExplore yet)

**Files:** `packages/react/src/explore/detectMode.ts` (+ test), `packages/client/src/query.ts`.

- [ ] **Step 1** — Failing test (`detectMode.test.ts` if present, else add one): `detectMode("MATCH (a)-[r]->(b) RETURN a")` → `"cypher"`; `detectMode("OPTIONAL MATCH (a) RETURN a")` → `"cypher"`; a leading-`match`-table edge case still works; `modeLabel("cypher")==="Cypher"`.
- [ ] **Step 2** — `detectMode.ts`: `ExploreMode` += `"cypher"`; before the KQL/search checks, `if (/^\s*(optional\s+)?match\b/i.test(input)) return "cypher";`; `modeLabel` case. `query.ts`: `QueryArgs.language` += `"cypher"`; `runQuery` sets `content-type: application/x-cypher` for cypher and adds an `x-graph` header from a new optional `graph?: string` arg (thread it through `encodeTicket`/the request). Keep kql/sql unchanged.
- [ ] **Step 3** — `pnpm --filter @pensieve-ai/react test` + `pnpm --filter @pensieve-ai/client typecheck`. Commit: `feat(explore+client): detect Cypher in the smart input + x-cypher wiring`.

---

### Task 4: `PensieveExplore.tsx` dispatch + badge (minimal, careful, on co-working WIP)

**Files:** `packages/react/src/explore/PensieveExplore.tsx`.

- [ ] **Step 1** — `git stash list` / `git diff --stat PensieveExplore.tsx` first to SEE the user's current WIP; work on top of it, do not revert it. Read the current `run()` dispatch + mode badge.
- [ ] **Step 2** — Add the `cypher` case to the `run()` dispatch: `mode === "cypher"` → `execute({ database, query: text, language: "cypher", graph: <current graph for the page> })` (reuse the page's graph/database scope; if no graph context exists, pass the database and let the server resolve a single in-scope graph). Add `Cypher` to the mode badge cycle + label. Preserve `autoRun` and all existing behavior.
- [ ] **Step 3** — `pnpm --filter @pensieve-ai/react test` + typecheck. Manual: typing `MATCH (a)-[r]->(b) RETURN a` shows the `Cypher` badge and runs. Commit (ONLY PensieveExplore.tsx, and ONLY the cypher additions — do not sweep the user's unrelated WIP into the commit; `git add -p` if needed): `feat(explore): Cypher mode in the smart input`.

---

## Self-Review
- Cypher→KQL reuse → Task 1. Server routing + graph binding → Task 2. Detection + client → Task 3. UI → Task 4. ✓
- Single-hop v1 + precise errors for deferred constructs → Task 1 Step 3. ✓
- Co-working safety (PensieveExplore last, `git add -p`) → Task 4. ✓
- No multi-hop / graph-match change (contained) → per spec. ✓
