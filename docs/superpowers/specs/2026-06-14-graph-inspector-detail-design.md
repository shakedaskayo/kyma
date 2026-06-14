# Graph Inspector — readable properties, markdown content, source-file viewer

Date: 2026-06-14
Status: approved (design)
Area: `packages/react/src/graph` (web graph explorer)

## Problem

When a node is selected in the graph explorer, the docked `InspectorPanel`
(`packages/react/src/graph/InspectorPanel.tsx`) renders every property as a
single `ky-truncate` line in a 320px strip. Long values (a memory's `content`,
`provenance` JSON, `topic_key`, timestamps) are cut off with no way to read the
full value, copy it, or render markdown. There is also no way to open the
underlying source file for nodes that reference one.

Three asks:

1. **See properties clearly** — full text, expand, copy.
2. **Full markdown rendering** — render a node's `content` as markdown, not a
   truncated mono line.
3. **View source files** — open the actual file a node references.

## Constraints / decisions (user-locked)

- **Detail layout:** keep the dock as a quick-scan summary with *inline-expandable*
  property rows, plus a **"View details" button that opens a wide modal** for the
  deep dive (full markdown content, all untruncated properties, source-file viewer).
- **Source scope:** in-graph `content` rendered as markdown **+** stored
  artifacts fetched through the **existing** `GET /v1/artifacts/by-path` endpoint
  (client method `client.artifacts.fetchArtifactByPath`). **No new server
  endpoint.** Reading arbitrary repo files from local disk is explicitly out of
  scope (file-candidate `File` nodes store a `path`/`sha256` in provenance but not
  the bytes; only the synopsis `content` is shown for them).
- Relationships stay in the dock only — not duplicated in the modal.
- Markdown content gets a **raw/rendered toggle**.
- No graph-store changes, no server changes.

## Data facts (verified against latest `main`)

- `GraphNode` (`packages/client/src/graph.ts`): `{ id, labels[], properties:
  Record<string, unknown>, metadata, namespace?, database? }`.
- **Memory** nodes carry the full text in `properties.content` (plus a 280-char
  `content_preview`). File-candidate **File** nodes carry a synopsis in
  `properties.content`.
- **LogFile / Artifact** nodes carry `properties.object_path` (object-store key).
  `client.artifacts.fetchArtifactByPath(path, { offset, limit })` returns
  `{ object_path, size_bytes, offset, returned_bytes, eof, content }` — a paged,
  tenant-scoped, lossy-UTF-8 window (default 64 KiB, server-capped at 4 MiB).
  → **Source-file availability = node has a string `object_path` property.**
- Components read the client via `useKymaClient()`
  (`packages/react/src/provider/context.ts`) and fetch with `@tanstack/react-query`.
- A zero-dependency markdown renderer already exists inside
  `packages/react/src/dashboards/panels/MarkdownPanelViz.tsx` (`renderMarkdown`).
- Reusable primitives: Radix `Dialog` (`packages/react/src/internal/ui/dialog.tsx`,
  portal-scoped to `.kyma-root`), `RowDetailDrawer` pattern (wrapped values + per-value
  copy/filter), `Button`.
- Test runner: `vitest` (`packages/react` → `vitest run`).

## Components

Each unit is small, single-purpose, and independently testable.

### 1. `packages/react/src/internal/ui/markdown.tsx` *(new)*
Extract the existing `renderMarkdown()` from `MarkdownPanelViz.tsx` into a shared
`renderMarkdown(md: string): React.ReactNode[]` and a `<Markdown source={...} />`
wrapper component. Refactor `MarkdownPanelViz` to import from here with **no
behavior change** (it keeps its current output). One job: markdown string → React
nodes. Supports headings, bold/italic/code/links, lists, fenced code blocks (the
current feature set — no new markdown features in this change).

### 2. `packages/react/src/graph/node-detail.ts` *(new — pure helpers)*
- `nodeContent(node): string | null` — returns `properties.content` when it's a
  non-empty string, else null. Drives the Content section.
- `nodeSourcePath(node): string | null` — returns `properties.object_path` when
  it's a non-empty string, else null. Drives the Source-file section.
- `formatValue(v: unknown): { text: string; kind: "scalar" | "json" | "array" }`
  — pretty-prints objects/arrays as indented JSON; for large numeric arrays
  (e.g. `embedding`) returns a collapsed summary like `float[1536]` so the modal
  never dumps thousands of floats. Scalars pass through via `String(v)`.
- `orderedProps(node): [string, unknown][]` — stable display order (content,
  title, then the rest alphabetically; embedding last). Keeps panel + modal
  consistent.

### 3. `packages/react/src/graph/ArtifactSourceViewer.tsx` *(new)*
Props: `{ path: string }`. Uses `useKymaClient()` + react-query to page through
the artifact:
- Manual `offset` state; "Load more" calls `fetchArtifactByPath(path, { offset })`
  and appends `content`, advancing `offset` by `returned_bytes` until `eof`.
- Renders accumulated text in a monospace `<pre>` with a **wrap toggle**,
  **copy-all** button, and a byte counter (`offset / size_bytes`).
- States: loading (first window), error + **Retry**, `eof` hides "Load more".

### 4. `packages/react/src/graph/NodeDetailModal.tsx` *(new)*
Props: `{ node: GraphNode | null; open: boolean; onClose: () => void }`. Built on
the Radix `Dialog` primitive, wide (`max-w-3xl`, scrollable). Sections:
- **Header** — node name, `labels.join(" · ") · namespace`, close.
- **Content** — when `nodeContent(node)` is non-null: `<Markdown>` with a
  **Raw / Rendered** toggle (raw shows the text in a `<pre>`).
- **Properties** — `orderedProps(node)` rendered untruncated in a
  `[label | value]` grid; values wrap (`whitespace-pre-wrap break-words`), JSON
  pretty-printed via `formatValue`, embedding/large arrays collapsed; per-value
  **copy** button on hover.
- **Source file** — only when `nodeSourcePath(node)` is non-null:
  `<ArtifactSourceViewer path={...} />`.

### 5. `packages/react/src/graph/InspectorPanel.tsx` *(edit)*
- Add a **"View details"** button (lucide `Maximize2`/`FileText`) to the existing
  actions row; local `useState(false)` controls the modal.
- Property rows become **expandable**: a chevron toggles a row between the current
  truncated line and a full wrapped value (`whitespace-pre-wrap break-words`);
  a copy button appears on hover. Default collapsed (panel stays compact).
- Mount `<NodeDetailModal node={node} open={open} onClose={() => setOpen(false)} />`.
- Relationships section unchanged.

## Data flow

`InspectorPanel` (already receives `node`) → local `open` state →
`NodeDetailModal(node)` → `ArtifactSourceViewer(object_path)` →
`useKymaClient()` + react-query → `/v1/artifacts/by-path`. No store or server
changes.

## Error / edge handling

- Artifact fetch failure → inline error with **Retry**; never throws to the modal.
- `nodeContent` null → Content section omitted. `nodeSourcePath` null → Source
  section omitted (e.g. the screenshot's Memory node shows Content + Properties,
  no Source section).
- Empty artifact → "No content."
- Non-UTF-8 bytes already lossy-decoded server-side.
- `embedding` and large arrays collapsed by default to avoid dumping floats.

## Testing

- **markdown util** (`markdown.test.tsx`): headings, bold, inline code, link,
  unordered list, fenced code block render to the expected elements; assert
  `MarkdownPanelViz` still renders unchanged (import smoke).
- **node-detail helpers** (`node-detail.test.ts`): `nodeContent` /
  `nodeSourcePath` presence/absence; `formatValue` scalar vs JSON vs collapsed
  array; `orderedProps` ordering.
- **ArtifactSourceViewer** (`ArtifactSourceViewer.test.tsx`, mocked client):
  loads first window; "Load more" advances `offset` and appends; `eof` hides the
  button; error path shows Retry.
- **NodeDetailModal** (`NodeDetailModal.test.tsx`): renders `content` as markdown;
  lists all properties untruncated; Source section present only when `object_path`
  exists.
- **InspectorPanel** (`InspectorPanel.test.tsx`): "View details" opens the modal;
  a property row expands/collapses on chevron click.

## Out of scope

- Reading arbitrary repo files from local disk (no `/v1/files/by-path`).
- New markdown features beyond the existing renderer's set.
- Editing properties; relationships in the modal; graph-store changes.
