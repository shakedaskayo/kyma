# Artifacts as First-Class Graph Entities

**Date:** 2026-06-09
**Status:** Approved
**Scope:** Make every catalog artifact (CI logs, object-store blobs, fswatch/agent files) a first-class graph node, linked to its producer by a single `HAS_ARTIFACT` edge, visible + searchable + previewable on the graph page.

## Program context

This is **Piece 4 of a 4-piece "shared-substrate" program** that lets agents and the
UI share one retrieval/graph substrate. Build order (dependency-first):

1. **Piece 4 — Artifacts as graph entities** ← *this spec*
2. **Piece 1 — Unified `/v1/search` substrate** (route memory-recall + graph + MCP through one engine)
3. **Piece 3 — Cypher in the smart input** (new parser → KQL graph ops)
4. **Piece 2 — Dreaming via the pensieve skill** (Claude-CLI skill-delivery design)

Each piece gets its own spec → plan → implementation cycle. This piece advances the
existing **Platform Enrichment (E-series)** program (E0 storage+catalog substrate is
built); cross-referenced here so we do not double-track.

## Problem

Artifacts exist in storage but are invisible as graph entities — the gap behind
"I don't see artifacts on the graph page."

- The `artifacts` catalog (`crates/pensieve-catalog/migrations/022_artifacts.sql`) is the
  system-of-record for object-store blobs: `id, tenant_id, object_path, source,
  artifact_class, table_ref, connector_id, size_bytes, sha256, created_at,
  expires_at, deleted_at`. The blob is retrievable via
  `GET /v1/artifacts/by-path?path=` (`crates/pensieve-server/src/artifacts_handler.rs:100`,
  byte-window capable via `offset`/`limit`).
- CI logs get graph representation **only inside the github graph**: the connector
  emits `Repository --HAS_RUN--> WorkflowRun --RUN_CONTAINS_JOB--> Job --JOB_HAS_LOG-->
  LogFile` (`crates/pensieve-connectors/src/github/transform.rs:161`). The `LogFile` node
  already carries `object_path`, `sha256`, `size_bytes`, `artifact_class:"log"`
  (`transform.rs:257-288`) — but is **not labeled `Artifact`**, is **not linked to its
  catalog `artifacts.id`**, and the edge type is the log-specific `JOB_HAS_LOG`.
- The artifact **blob itself is never a node**. Object-store / fswatch / agent-contributed
  artifacts (the migration's named cases) have **no graph node at all** and **no graph is
  registered for them**, so `/v1/graph` discovery (`graph_handler.rs` list) never lists
  them, the unified canvas (`packages/react/src/hooks/usePensieveGraph.ts`) never loads them,
  and they are not searchable via `/v1/graph/:graph/search` (`graph_handler.rs:~514`).

Net: artifacts cannot be seen, traversed, searched, or opened from the graph page.

## Decisions (locked)

| Decision | Choice |
|---|---|
| Node placement | **Nodes live in the producer graph.** The existing `LogFile` node *becomes* the Artifact node (enrich in place, reuse its id — no duplicate). |
| All-artifacts view | Unified canvas + an `Artifact` **label filter** (no dedicated graph needed for the cross-source view). |
| Source scope | **All sources now:** github CI logs + object-store standalone blobs + fswatch/agent files. |
| Edge model | **Single generic `HAS_ARTIFACT`** edge (producer → artifact). `JOB_HAS_LOG` is standardized to it. No `ARTIFACT_OF`. |
| Log-vs-file distinction | Carried on the **node**: a single `Artifact` label (the github `labels` column is a single Utf8 string — `transform.rs:13`) + an `artifact_class` prop (`log`/`file`/…). The all-artifacts view is the `Artifact` **label filter**; logs-only filtering is by the `artifact_class` prop, not a second label. (Refined from `["Artifact",<Class>]` after confirming the single-string labels schema.) |
| Materialization | **Approach A** — per-source emission into producer graphs + a catalog→node **catch-all sync** for artifacts with no producer node. (Not B/central+cross-namespace edges; not C/virtual Postgres provider.) |
| Graph UI | **Node + inline preview** — the preview already exists as `LogFileViewer` (`GraphSidebar.tsx:451`), today gated on the `LogFile` label; widen its trigger to the `Artifact` label. Click → fetch blob, render redacted content inline (paged) + metadata + download. |
| github historical rows | github relabel is **forward-only**. The stored-graph dedups nodes by id with an arbitrary pick (`stored_graph.rs:43`) and the tables are append-only, so already-ingested `LogFile` rows are *not* rewritten; new captures emit `Artifact` nodes. A full historical relabel would need a table rebuild — deferred. |
| Content search | **Out of scope** — full log/file content search across artifacts is Piece 1 (`/v1/search`). This piece is name/metadata search only. |

## Architecture

### 1. Artifact node contract (one shared shaping helper)

A single function turns a catalog `Artifact` record into a graph node row, so both
materialization paths (below) produce identical shapes. Lives where both the github
connector and the catch-all sync can reach it (proposed: `pensieve-connectors::artifacts`
module, alongside the `ArtifactStore` trait at `crates/pensieve-connectors/src/artifacts.rs`).

- **id:** stable, reusing the producer's existing node id where one exists (github:
  `log_file_node_id(owner, repo, job_id)`). Catch-all artifacts use `artifact::{uuid}`.
- **labels:** a single `Artifact` label (the github `labels` column is one Utf8 string,
  `transform.rs:13`; `make_node` at `:130` takes one label). Class lives in props.
- **props:** `artifact_id` (catalog uuid — the new link), `object_path`, `sha256`,
  `size_bytes`, `artifact_class`, `source`, `created_at`, `retrievable` (true when
  `object_path` present and not expired/deleted), plus source context already present on
  CI nodes (`truncated`, etc.).
- **name (display only):** human label, e.g. `build.log`. Note: stored-graph search keys
  off **id + labels**, not name (`stored_graph.rs:76`), so searchability comes from the
  `Artifact` label + the id's repo/job context (e.g. `log:acme/app#900`), not the name.
- **edge:** `HAS_ARTIFACT` (producer → artifact). Edge props may carry `size_bytes`.

### 2. Two materialization paths, one contract

**(a) Producer-attached** — the source's ingest enriches/links the Artifact node in its
own graph. github CI logs (the concrete gap) ship fully:

- `transform.rs::log_file_rows` (`:257`) emits the node via the contract: label
  `Artifact` (was `LogFile`), set `artifact_id` (the uuid from `register_artifact`,
  currently discarded at `joblogs.rs:176`), keep the existing retrieval props. The node
  id is unchanged. **Forward-only** — already-ingested `LogFile` rows are not rewritten
  (append-only + arbitrary id-dedup, `stored_graph.rs:43`).
- The edge becomes `HAS_ARTIFACT` instead of `JOB_HAS_LOG` (`transform.rs:281-287`).
  `HAS_RUN` / `RUN_CONTAINS_JOB` / `RUN_ON_BRANCH` are untouched.
- This path is the template fswatch/file ingest follows once its producer graph exists;
  until then those artifacts fall to (b).

**(b) Catch-all sync** — a catalog→node mirror materializes every artifact **not**
producer-attached (object-store standalone, agent-contributed files, fswatch snapshots
with no producer graph) into a registered `artifacts` graph. Runs forward (on
`register_artifact`) **and** as a one-time backfill. Reads `artifacts` rows, excludes
`deleted_at IS NOT NULL` / past-`expires_at`, shapes nodes via the same contract. These
nodes are edge-free (standalone) unless a `table_ref`/producer linkage is available.

### 3. Catch-all `artifacts` graph: tables, registration, provisioning

- **Tables (columnar, like memory):** `artifact_nodes` (`id, labels, name, props`) and
  `artifact_edges` (`id, src, dst, type, props`) — mirroring the github graph column
  shape (`github/mod.rs:9-11`).
- **Registration:** `graph_registrations` row (`migrations/008_graphs.sql`) via
  `GraphSpec::with_defaults` → name `artifacts`, `node_table=artifact_nodes`,
  `edge_table=artifact_edges`. Once registered, `/v1/graph` discovery lists it and the
  unified canvas loads it automatically (no UI change for discovery).
- **Provisioner:** a small `ensure_provisioned`-style routine that creates the two tables
  + the registration if absent, modeled on the memory writer
  (`crates/pensieve-memory/src/writer.rs` `ensure_provisioned` / `create_graph`). Invoked on
  first catch-all write and at startup.

### 4. Searchability

Rides existing infrastructure — no new search endpoint:

- Per-graph `/v1/graph/:graph/search` matches case-insensitively over **id + labels**
  (`stored_graph.rs:76`); Artifact nodes match on the `Artifact` label and the id's
  repo/job context (e.g. `log:acme/app#900`).
- All-DB discovery (`usePensieveGraph.ts`) already fans out across graphs, so artifacts in
  github / `artifacts` graphs are searchable together.
- **Content search is Piece 1** (`/v1/search` over the blobs), explicitly excluded here.

### 5. Graph UI — node + inline preview

- The `Artifact` label appears in the label-filter legend automatically (`SectionLabels`
  in `GraphSidebar.tsx` renders whatever labels the stats return); toggling it is the
  all-artifacts view.
- The inline preview **already exists** as `LogFileViewer` (`GraphSidebar.tsx:451`) — it
  fetches the blob via `client.artifacts.fetchArtifactByPath(path, {offset, limit:65536})`
  and pages through it with "Load more". Today it is gated at `:531-534` on the `LogFile`
  label. The change is to **widen that trigger** to fire for the `Artifact` label too (so
  relabeling does not break it). `objectPathOf` already reads `object_path` from props or
  the JSON `props` blob.
- An optional polish task adds an `Artifact` icon/colour to `graph-icons.tsx` /
  `getLabelColor` (unknown labels already fall back to a default).

### 6. Error handling / edge cases

- **Missing blob** (catalog row written, object not yet flushed — the row-first/blob-second
  order in `joblogs.rs`): preview shows "pending / unavailable", no crash.
- **Orphan artifacts** (no producer edge): render as standalone nodes; zero-edge safe.
- **Oversize blobs:** preview byte-window truncates; download serves full.
- **Expired / soft-deleted** (`expires_at`/`deleted_at`): excluded from materialization;
  a node whose artifact later expires is removed/marked on the next sync.
## Testing

**Rust (`pensieve-connectors`, `pensieve-server`, `pensieve-catalog`):**

- Unit: artifact node-shape contract — single `"Artifact"` label, `artifact_id` set
  (conditional), `artifact_class` prop, `retrievable` logic.
- Integration: ingesting a CI job log yields an `Artifact`-labeled node carrying
  `artifact_id` + an `HAS_ARTIFACT` edge (job→artifact) in `github_nodes`/`github_edges`;
  no `JOB_HAS_LOG` is emitted. (Forward-only: existing rows are not rewritten.)
- Catch-all: `ArtifactGraphWriter::sync` yields a node in the `artifacts` graph for a
  non-`github` artifact; `deleted_at`/expired rows are skipped; provisioning + re-sync are
  idempotent.
- `list_live_artifacts` returns only live rows, scoped to the tenant.

**Client / React (`packages/client`, `packages/react`):**

- `artifacts.byPath` client unit test (byte window).
- Preview-panel render test: content + metadata + download; **pending/unavailable** and
  **oversize-truncated** states.
- Label filter shows/hides `Artifact` (and `Log`/`File` sub-labels); discovery includes
  the `artifacts` graph.

## File touch list (anticipated)

- `crates/pensieve-connectors/src/artifacts.rs` — shared artifact node contract helper.
- `crates/pensieve-connectors/src/github/transform.rs` — `log_file_rows` enrich +
  `JOB_HAS_LOG`→`HAS_ARTIFACT`; `make_node` labels-array support.
- `crates/pensieve-connectors/src/github/joblogs.rs` — thread `artifact_id` from
  `register_artifact` into node props.
- New: catch-all sync + `artifacts`-graph provisioner that creates
  `artifact_nodes`/`artifact_edges` **programmatically** (mirroring the memory writer's
  `ensure_provisioned`, not a SQL migration) and registers the graph.
- `crates/pensieve-server/src/graph_handler.rs` — verify artifact nodes flow through
  existing search/overview (likely no change).
- `packages/client/src/artifacts.ts` — confirm `byPath` shape for preview.
- `packages/react/src/graph/*` — node detail artifact preview; label legend.
- Backfill task/command for existing artifacts + `JOB_HAS_LOG` edges.

## Out of scope (deferred)

- Cross-artifact **content** search and routing memory/graph/MCP through one engine —
  **Piece 1** (`/v1/search` substrate).
- Cypher in the smart input — **Piece 3**.
- Dreaming-as-skill / Claude-CLI skill delivery — **Piece 2**.
- A dedicated "Artifacts" navigation page (label filter on the unified canvas suffices
  for v1).
