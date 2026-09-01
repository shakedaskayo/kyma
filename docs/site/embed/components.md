---
title: Components — @pensieve-ai/react
description: Full prop tables for PensieveProvider, PensieveGraph, PensieveQueryEditor, PensieveDiscover, PensieveDashboard, and PensieveAgentChat.
---

# Components

All components must be rendered inside a `PensieveProvider`. Each component wraps
its inner content in a `PensieveErrorBoundary`, so a render error inside the
component never crashes the host application.

---

## `PensieveProvider`

**Import:** `import { PensieveProvider } from "@pensieve-ai/react"`

Root provider. Creates a `PensieveClient`, optionally a `QueryClient`, applies the
theme as CSS custom properties, and renders a portal target for Radix UI
popovers/dialogs.

| Prop | Type | Required | Default | Description |
|---|---|---|---|---|
| `endpoint` | `string` | yes | — | Pensieve server URL, e.g. `https://pensieve.acme.internal`. Trailing slashes are stripped. |
| `auth` | `PensieveAuth` | yes | — | `{ token: string }` or `{ getToken: (opts?) => Promise<string> }`. See [Authentication](/embed/authentication). |
| `database` | `string` | no | — | Default database sent as `x-database` on every request. Per-component `database` props override this. |
| `theme` | `Partial<PensieveTheme> \| "inherit"` | no | `pensieveDark` | Theme tokens merged over `pensieveDark`, or `"inherit"` to use host CSS vars. See [Theming](/embed/theming). |
| `queryClient` | `QueryClient` | no | isolated instance | Reuse the host app's TanStack Query client. The provider creates its own (with `staleTime: 30 000`, `refetchOnWindowFocus: false`, `retry: 1`) when omitted. |
| `onError` | `(err: unknown) => void` | no | — | Error handler propagated via context for components that bubble non-render errors (e.g. auth failures). |
| `children` | `ReactNode` | yes | — | Component tree. |

---

## `PensieveGraph`

**Import:** `import { PensieveGraph } from "@pensieve-ai/react/graph"`

**Bundle note:** pulls in `react-force-graph-2d` (WebGL force-directed graph
canvas). If you never render `PensieveGraph`, this library is not bundled.

Self-contained interactive property graph. Fetches graph data from the Pensieve
server, merges multi-database graphs onto a single canvas, and renders a
force-directed layout with a sidebar showing controls, an inspector, and a
legend.

Each mounted `PensieveGraph` owns its own isolated Zustand store — two instances
on the same page never share selection or layout state.

| Prop | Type | Default | Description |
|---|---|---|---|
| `graphs` | `Array<{ database?: string; graph: string }>` | — | Explicit list of (database, graph) pairs to load. Omit to load all graphs from the provider's default database. |
| `discover` | `"all-databases"` | — | When set, discovers every graph across every database via catalog fan-out. Overrides `graphs`. |
| `height` | `number \| string` | `"100%"` | Height of the container div. |
| `layout` | `LayoutAlgorithm` | `"force"` | Initial layout algorithm seeded into the graph store. |
| `toolbar` | `boolean` | `true` | Show the sidebar/toolbar control surface. Setting either `toolbar` or `sidebar` to `false` hides the entire control surface (they share the same DOM region). |
| `sidebar` | `boolean` | `true` | Alias for `toolbar`. |
| `focusQuery` | `string` | — | Initial search text seeded into the store's search field on first mount. Not a controlled value — subsequent prop changes are ignored. |
| `realm` | `string` | — | Override the realm passed to the graph data layer. |
| `className` | `string` | — | Extra CSS class on the container div. |
| `style` | `React.CSSProperties` | — | Inline styles for the container div. |
| `fallback` | `ReactNode` | — | Custom content rendered by the error boundary on a render error. |
| `onNodeClick` | `(node: GraphNode) => void` | — | Called when a node is clicked in the canvas. Receives the full `GraphNode` record. |
| `onSelectionChange` | `(nodes: GraphNode[]) => void` | — | Called when the selection changes. Receives an array of 0 or 1 node (current design is single-select). |

---

## `PensieveQueryEditor`

**Import:** `import { PensieveQueryEditor } from "@pensieve-ai/react/query"`

**Bundle note:** pulls in `@monaco-editor/react` and `monaco-editor`. If you
never render `PensieveQueryEditor`, Monaco is not bundled.

Full-featured embedded query surface: KQL/SQL editor with syntax highlighting
and completions, a schema browser, a results grid, optional chart panel, and
a time range picker. Uses `usePensieveQuery` internally.

| Prop | Type | Default | Description |
|---|---|---|---|
| `language` | `"kql" \| "sql"` | `"kql"` | Query language. SQL mode disables KQL completions and time-range injection. |
| `defaultQuery` | `string` | `""` | Initial query text. Uncontrolled — prop changes after mount are ignored. |
| `showSchemaBrowser` | `boolean` | `true` | Show the schema browser panel on the left. |
| `showResults` | `boolean` | `true` | Show the results grid below the editor. |
| `showChart` | `boolean` | `true` | Show an auto-detected chart panel below results when the result shape is plottable. |
| `showTimeRange` | `boolean` | `true` | Show the time range picker in the toolbar (KQL mode only). |
| `timeRange` | `TimeRange` | `{ preset: "1h" }` | Initial time range; re-applied when the prop changes (controlled-ish). |
| `readOnly` | `boolean` | `false` | Make the editor read-only and hide the Run button. |
| `database` | `string` | — | Override the database for execution and schema loading. Falls back to the provider's default database. |
| `className` | `string` | — | CSS class on the outermost container div. |
| `style` | `React.CSSProperties` | — | Inline styles for the outermost container div. |
| `fallback` | `ReactNode` | — | Error boundary fallback. |
| `onResults` | `(r: { columns: Column[]; rows: Record<string, unknown>[] }) => void` | — | Called after each successful query with the full result set. |
| `onQueryChange` | `(q: string) => void` | — | Called on each editor keystroke with the current query text. |

**`TimeRange`** is `{ preset: "5m" | "15m" | "1h" | "6h" | "24h" | "7d" | "30d" }`.

In KQL mode, the time range is spliced into the query via `prependTimeFilter()`
immediately before execution — the editor's displayed text is never mutated.
In SQL mode, time-range injection is skipped; write your own `WHERE` clause.

---

## `PensieveDiscover`

**Import:** `import { PensieveDiscover } from "@pensieve-ai/react/discover"`

Streaming log/event search surface with a scope picker, histogram timeline,
field stats rail, and expandable row detail drawer. The discover grammar
accepts free-text and field filters (`service_name == "api"`, `level != "debug"`,
`status_code > 400`, full-text `error`).

When `database` is set, `PensieveDiscover` calls `client.withDatabase(database)`
internally and overrides the `PensieveContext` client for all child components.
Every search request and the scope picker list both use the scoped client.

| Prop | Type | Default | Description |
|---|---|---|---|
| `defaultQuery` | `string` | — | Initial discover-grammar search text. |
| `scope` | `Scope` | — | Initial scope (set of source tables to search). |
| `timeRange` | `TimeRange` | — | Initial time range. |
| `database` | `string` | — | Override the database for all search requests. |
| `className` | `string` | — | CSS class on the container div. |
| `style` | `React.CSSProperties` | — | Inline styles for the container div. |
| `fallback` | `ReactNode` | — | Error boundary fallback. |
| `onRowOpen` | `(row: Record<string, unknown>, source: string) => void` | — | Called when the user clicks a row to open its detail drawer. `source` is the source table key. |
| `onExportKql` | `(kql: string) => void` | — | Called when the user clicks "Export to KQL". Receives the compiled KQL string. |
| `onSearchChange` | `(search: string) => void` | — | Called on each keystroke in the search bar. Useful for persisting search state externally. |
| `onScopeChange` | `(scope: Scope) => void` | — | Called when the scope changes. |
| `onTimeRangeChange` | `(timeRange: TimeRange) => void` | — | Called when the time range changes. |

---

## `PensieveDashboard`

**Import:** `import { PensieveDashboard } from "@pensieve-ai/react/dashboards"`

**Bundle note:** pulls in `react-grid-layout` for the drag-resize grid and
`echarts` for panel charts.

Embeds a Pensieve dashboard by ID. Read-only by default; `editable={true}` enables
drag-to-reorder, panel resize, add/edit/delete, and inline name editing. Saves
go directly to the Pensieve API.

When `database` is set, all panel queries, the dashboard load, and the schema
fetch in the add-panel modal all use a scoped client (same token cache,
different `x-database` default). New panels are pre-seeded with the database so
their stored `database_name` matches the override.

| Prop | Type | Default | Description |
|---|---|---|---|
| `dashboardId` | `string` | yes | ID of the dashboard to load. |
| `editable` | `boolean` | `false` | Allow panel add/edit/delete and drag-to-reorder. |
| `timeRange` | `TimeRange` | — | Override the dashboard's stored time range. When omitted, uses `time_range_preset` from the server. |
| `database` | `string` | — | Override the database for all panel queries and dashboard operations. |
| `className` | `string` | — | CSS class on the root element. |
| `style` | `React.CSSProperties` | — | Inline styles for the root element. |
| `fallback` | `ReactNode` | — | Content rendered while the dashboard is loading. |
| `onPanelClick` | `(panelId: string) => void` | — | Called when a panel title is clicked (read-only mode only). |
| `onEditChange` | `(editable: boolean) => void` | — | Called when the edit/view toggle is clicked. If provided, the host controls edit mode (the prop drives it); otherwise edit mode is managed internally. |
| `onSaveSuccess` | `(dashboard: DashboardWithPanels) => void` | — | Called after a successful save with the updated dashboard object. |
| `onSaveError` | `(err: unknown) => void` | — | Called when a save fails. |

Auto-refresh: when `refresh_interval_seconds > 0` on the loaded dashboard and
the component is not in edit mode, a `setInterval` fires at that cadence and
invalidates the React Query cache for this dashboard.

---

## `PensieveAgentChat`

**Import:** `import { PensieveAgentChat } from "@pensieve-ai/react/agent"`

Embedded AI assistant chat backed by the Pensieve agent (`POST /v1/agent/ask`).
The component wraps `AgentConsole` in a `CapabilityGate` on the `agent`
capability flag: servers without the inline agent surface render a "not
available" card rather than 404-ing. The gate fails **open** while capabilities
load (pre-capabilities servers are treated as capable).

**Note:** `PensieveAgentChat` requires a full-access token (no `pensieve_databases`
scope). Database-scoped tokens receive 403 from the server because the agent's
tool loop can address any database internally. See [Authentication](/embed/authentication#fail-closed-surfaces).

| Prop | Type | Default | Description |
|---|---|---|---|
| `placeholder` | `string` | `"Ask a question about <database or 'your data'>…"` | Placeholder text in the input area. |
| `database` | `string` | — | Override the database sent with each request. Defaults to the transport's `database`. |
| `className` | `string` | — | CSS class on the root container. |
| `style` | `React.CSSProperties` | — | Inline styles for the root container. |
| `fallback` | `ReactNode` | — | Error boundary fallback. |
| `onMessage` | `(m: { role: string; text: string }) => void` | — | Called once per completed assistant turn (when the SSE stream finishes). |

The agent uses the Vercel AI SDK UI Message Stream v1 protocol over SSE.
Multi-turn conversation state (session ID) is tracked internally; the session
resets when the component unmounts.
