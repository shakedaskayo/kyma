# Graph Explorer Redesign — Design

**Date:** 2026-06-11
**Status:** Approved
**Scope:** `packages/react/src/graph/*`, `packages/client` (layout/types), `web/src/routes/_app.graph.tsx`, `kyma-server` graph endpoints

## Goal

Make the graph explorer render the **full graph every time** (tens of thousands of nodes), at 60fps, while making it dramatically easier to read (direction arrows, relationship references), travel (minimap, fly-to, breadcrumbs, keyboard), and understand (zoom-based level of detail, focus neighborhoods). Evolve — don't replace — the dark "galaxy" visual identity.

## Decisions (user-validated)

| Question | Decision |
|---|---|
| Scale target | Tens of thousands of nodes (10k–500k design ceiling) |
| Visual identity | Evolve the galaxy look; keep gradient nodes, brand icons, family colors |
| Renderer | **Sigma.js v3 + graphology** (WebGL), replacing `react-force-graph-2d` |
| Edge style | **Quiet arrows + loud focus**: subtle arrowheads at rest; hover/selection makes incident edges bold with arrowheads, relationship label pills, flow particles |
| Navigation aids | All four: minimap, fly-to search, breadcrumb trail, keyboard edge-walking |
| Page chrome | **Full-bleed canvas + floating glass panels**: ⌘K command bar, slide-in inspector, bottom legend dock |

## 1. Rendering engine

- Replace `react-force-graph-2d` with **Sigma.js v3** rendering a **graphology** graph instance in `packages/react/src/graph/`.
- **Custom node program**: radial-gradient sphere look, brand-mark/kind-glyph icon textures (texture atlas built from existing `graph-icons.tsx` sources), degree-scaled radius (95th-percentile cap, existing behavior), focus ring + glow halo on selection/landmark.
- **Custom edge program** (quiet/loud):
  - At rest: relationship-family color (existing 7-family palette), low alpha, small arrowhead at target.
  - Hover/selection: incident edges at high alpha, bold arrowheads, relationship-type label pills, animated flow particles; non-incident edges dimmed.
  - Curvature toggle preserved (parallel/reciprocal edges always curve to stay distinguishable).
- **Zoom level-of-detail (LOD)**:
  - Far: nodes as colored dots, edges thin color-only, community hulls visible, labels only for top-degree landmarks.
  - Mid: arrowheads appear, node icons render, landmark + selected labels.
  - Near: all node labels, edge label pills on hover, full detail.
- **Community hulls**: keep label-propagation (or upgrade to graphology Louvain) hulls as a toggle, drawn on a background layer.
- **Fallback**: existing canvas renderer (`GraphCanvas.tsx`) is retained. WebGL detection failure (or `renderer="canvas"` prop) falls back to it so SDK embedders never hard-fail.

## 2. Full graph, every time — data & layout

- **Remove** the 800-node default limit and the >280-node landmarks-only overview mode.
- **New chunked export endpoint** on kyma-server: `GET /v1/graph/{graph}/export?cursor=<c>&realm=<r>` returning ~10k nodes + incident edges per page, **including precomputed layout positions** per node. Client renders progressively as chunks arrive (graph appears within the first chunk, fills in over subsequent ones, camera stable).
- **Server-side layout** in `kyma-server` (Rust): ForceAtlas2-style force layout plus the tree/radial/grid algorithms currently in `@kyma-ai/client`, moved server-side.
  - Computed once per (graph, layout-algorithm, realm) version; cached; invalidated on graph mutation.
  - Deterministic: the same graph version always produces identical positions.
  - First request after a mutation may compute synchronously for small graphs and via background job for large ones; the endpoint reports `layout_status: ready | computing` so the client can show progress and poll.
- **Cross-DB "all databases & graphs" mode**: unchanged semantics — fan out per (db, graph) coordinate, one chunk stream each, merged with composite ids (existing dedup scheme).
- `useKymaGraph` reworked to drive the chunk loop via React Query infinite queries, building one shared graphology instance.

## 3. Page chrome (full-bleed + floating panels)

- Canvas owns the entire viewport (`_app.graph.tsx` route, and default for embedded `KymaGraph`).
- **⌘K command bar** (top-center, glass): search nodes (server `searchNodes` + local graphology index), fly-to result, quick filters (node type / relationship type / namespace), layout switch, style toggles as commands.
- **Inspector panel** (right, slides in on selection only): node id + labels, properties, metadata, incident edges grouped by relationship type with direction glyphs (→ outgoing, ← incoming), per-edge "fly to other end", copy id, focus-neighborhood button.
- **Legend dock** (bottom-center chip): relationship-family color swatches + node-type counts at rest; expands into the full filter panel (node types, namespaces/graphs, relationship-type isolation) and the six style toggles (glow, flow, curve, clusters, size, labels).
- **Minimap** (bottom-left): whole-graph density map with viewport rectangle, drag-to-jump.
- **Zoom controls** (bottom-right): +/−, fit-to-graph, reset.
- **Breadcrumb trail** (top-left): see §4.
- All panels are floating glass (translucent dark, blur, `ky-` tailwind tokens), consistent with existing design language (IBM Plex Sans, existing radius/shadow scale).

## 4. Traveling the graph

- **Minimap**: rendered from node positions (downsampled density), viewport rect synced to camera; dragging the rect or clicking the map moves the camera.
- **Fly-to search**: choosing a ⌘K result animates the camera (eased zoom-out → pan → zoom-in) to the node and pulses its 1-hop neighborhood on arrival.
- **Breadcrumb trail**: every selected/visited node appends `db/graph::node` to a top-left trail (deduped, capped with overflow menu); clicking a crumb flies back and reselects.
- **Keyboard edge-walking**: with a node selected — Tab / arrow keys cycle its neighbors sorted by relationship type then degree (highlighting the candidate edge + node), Enter selects + flies to the candidate, Esc steps back along the trail. Shortcuts surfaced in a `?` overlay.
- **Hover**: 1-hop highlight, rest dimmed (kept from current).
- **Double-click → focus neighborhood**: since the full graph is loaded, double-click no longer fetches; it zooms to and isolates the 1–2-hop neighborhood (rest of graph faded), Esc exits focus.
- Deep-link `?focus=<nodeId>` preserved (fly-to on mount); `?graph=` param preserved.

## 5. State & public API

- `graph-store.ts` (zustand) extended: camera state, trail stack, focus-neighborhood state, LOD thresholds, command-bar open state. Existing toggles/filters kept.
- `KymaGraph` public props remain backward-compatible (`graphs`, `discover`, `limit` — now meaning "no limit" when unset, `onNodeClick`, `onSelectionChange`, etc.). New opt-ins: `renderer`, `chrome` (full/minimal for embedders).

## 6. Error handling

- Chunk fetch failure: per-stream retry with backoff; if a stream dies, render what arrived + non-blocking "partial graph — retry" banner.
- `layout_status: computing`: progress indicator over the canvas; poll until ready; graph appears when positions land.
- WebGL unavailable: automatic canvas-renderer fallback (silent, telemetry-logged).
- Search/expand API errors: toast in command bar, never blanks the canvas.

## 7. Performance budget

| Metric | Target |
|---|---|
| 50k nodes / 100k edges pan-zoom | 60fps |
| First paint (cached layout) | < 2s |
| Hover 1-hop highlight | < 16ms (graphology neighbor index) |
| Full 50k-node load (chunked) | progressively visible from first chunk |

## 8. Testing

- **Unit (vitest)**: store logic (trail, focus, LOD thresholds), neighbor-sort for keyboard walking, chunk-merge/dedup in `useKymaGraph`, LOD reducer functions.
- **Rust**: export endpoint pagination + position inclusion, layout cache compute/invalidate, `layout_status` lifecycle.
- **Benchmark fixture**: synthetic generator (50k/200k nodes) behind a dev route for manual fps verification.
- **Manual**: browser verification of arrows/labels/hover/minimap/fly-to/keyboard on a real seeded deployment.

## Out of scope

- Multi-node selection, edge editing/creation, graph mutations from the canvas.
- 3D rendering.
- Mobile/touch-first optimization (basic pinch-zoom only).
- Replacing the search backend (existing `searchNodes` reused).

## Amendment — reference-image visual direction (2026-06-11)

User-supplied reference images (radial brand-icon arcs on light bg; dark starburst communities with neutral edge fans) refine §1 and the layout:

1. **Edges at rest are neutral hairlines** — thin translucent slate (dark mode ≈ rgba(148,163,184,~0.16), light mode darker equivalent), NOT family-colored. Relationship-family color, bold arrowheads, and label pills appear only in the loud state (hover / selection / relationship-type isolation) and in the legend. The canvas at rest reads as colored constellation clusters connected by faint fans.
2. **Node color carries the story** — vivid label/kind colors on plain dots for the mass of nodes; icons render only at mid/near LOD and always on landmark hubs. Hub size contrast increases (degree-scaled, 95th-percentile cap retained, but wider radius range) so each community reads as a big labeled hub with small satellites.
3. **Starburst orbit post-pass (server layout)** — after force layout converges, degree-1 leaves are re-placed on evenly-spaced rings orbiting their sole neighbor (ring radius grows with leaf count; deterministic angle order). Hubs become crisp starbursts instead of fuzzy blobs. Implemented in `kyma-graph::layout` as a force-layout post-process, before the overlap pass is re-run for orbit collisions only.
