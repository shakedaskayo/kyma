---
title: Headless hooks — @kyma-ai/react
description: Low-level data hooks for building custom UIs on top of the Kyma data layer without the bundled components.
---

# Headless hooks

All five Kyma data surfaces are available as plain React hooks that return raw
data, loading state, and imperative actions. Use them when you want to build
a custom UI on top of the Kyma data layer without the bundled component chrome.

All hooks must be called inside a `KymaProvider`.

---

## `useKymaGraph`

```ts
import { useKymaGraph } from "@kyma-ai/react";
import type { UseKymaGraphArgs, UseKymaGraphResult } from "@kyma-ai/react";
```

Loads one or more (database, graph) pairs in parallel via React Query, merges
them into a unified node/edge set, and exposes expansion and search helpers.

```ts
function useKymaGraph(args?: UseKymaGraphArgs): UseKymaGraphResult
```

**`UseKymaGraphArgs`**

| Field | Type | Default | Description |
|---|---|---|---|
| `graphs` | `Array<{ database?: string; graph: string }>` | — | Explicit list of graph coordinates. |
| `discover` | `"all-databases"` | — | Discover all graphs across all databases (catalog fan-out). Overrides `graphs`. |
| `realm` | `string` | — | Passed to `getOverview` for realm-scoped fetches. |
| `limit` | `number` | `800` | Max nodes per graph request. |

**`UseKymaGraphResult`**

| Field | Type | Description |
|---|---|---|
| `nodes` | `GraphNode[]` | Merged nodes from all loaded graphs, deduplicated by composite id. |
| `edges` | `GraphRelationship[]` | Merged edges from all loaded graphs, deduplicated. |
| `stats` | `GraphStats \| null` | Aggregate counts (total nodes/edges, per-label and per-type breakdowns). |
| `namespaceCounts` | `Record<string, number>` | Node count per `database/graph` namespace key. |
| `coords` | `GraphCoord[]` | Resolved (database, graph) coordinates being loaded. |
| `progress` | `GraphProgress` | `{ total, settled, hasAny }` — per-namespace fetch progress. |
| `isLoading` | `boolean` | True while discovery or any graph fetch is pending. |
| `isError` | `boolean` | True when all fetches have settled and no data arrived. |
| `error` | `unknown` | First error from any fetch, or null. |
| `expandNode` | `(compositeId: string) => Promise<void>` | Fetch and append the neighbours of a node identified by its composite id (`database/graph::nodeId`). |
| `searchNodes` | `(text: string) => Promise<SearchHits>` | Search nodes in the first loaded graph by text. |
| `refetch` | `() => void` | Invalidate all graph query cache entries and refetch. |

**Custom graph UI example**

```tsx
import { useKymaGraph } from "@kyma-ai/react";

function GraphStats() {
  const { nodes, edges, isLoading, error } = useKymaGraph({
    discover: "all-databases",
  });

  if (isLoading) return <p>Loading…</p>;
  if (error) return <p>Error: {String(error)}</p>;

  return (
    <ul>
      <li>{nodes.length} nodes</li>
      <li>{edges.length} edges</li>
    </ul>
  );
}
```

---

## `useKymaQuery`

```ts
import { useKymaQuery } from "@kyma-ai/react";
import type { KymaQueryArgs, UseKymaQueryResult } from "@kyma-ai/react";
```

Runs KQL or SQL queries via `POST /v1/query` (NDJSON streaming). Accumulates
columns and rows as chunks arrive. An `AbortController` per `execute()` call
allows mid-stream cancellation; unmount aborts automatically.

```ts
function useKymaQuery(): UseKymaQueryResult
```

**`UseKymaQueryResult`**

| Field | Type | Description |
|---|---|---|
| `columns` | `Column[]` | Columns from the last/current query (name + kind). |
| `rows` | `Record<string, unknown>[]` | Accumulated rows from the last/current query. |
| `isRunning` | `boolean` | True while the stream is active. |
| `error` | `unknown` | Error from the last `execute()`, or null. |
| `execute` | `(args: KymaQueryArgs) => Promise<void>` | Run a query. Aborts any previously running query. Resolves when the stream completes. |
| `cancel` | `() => void` | Abort the current in-flight query. No-op if idle. |

**`KymaQueryArgs`**

| Field | Type | Description |
|---|---|---|
| `database` | `string` | Database to query. |
| `query` | `string` | KQL or SQL query text. |
| `language` | `"kql" \| "sql"` | Query language. |
| `walMs` | `number?` | Optional WAL durability hint. |
| `memBytes` | `number?` | Optional memory budget hint. |

**Custom query UI example**

```tsx
import { useKymaQuery } from "@kyma-ai/react";

function LiveQuery() {
  const { rows, isRunning, execute, cancel } = useKymaQuery();

  return (
    <div>
      <button onClick={() => execute({ database: "prod", query: "errors | take 20", language: "kql" })}>
        Run
      </button>
      {isRunning && <button onClick={cancel}>Cancel</button>}
      <pre>{JSON.stringify(rows.slice(0, 5), null, 2)}</pre>
    </div>
  );
}
```

---

## `useKymaDiscover`

```ts
import { useKymaDiscover } from "@kyma-ai/react";
import type { UseKymaDiscoverResult } from "@kyma-ai/react";
```

Streaming discover search (`POST /v1/explore/search`) that yields typed frames
(`plan`, `source_progress`, `rows`, `histogram`, `source_done`, `done`, etc.).
Use this to build a custom log/event browsing UI.

```ts
function useKymaDiscover(): UseKymaDiscoverResult
```

**`UseKymaDiscoverResult`**

| Field | Type | Description |
|---|---|---|
| `frames` | `Frame[]` | All frames received from the current/last search stream. |
| `isStreaming` | `boolean` | True while the stream generator is running. |
| `error` | `unknown` | Error from the last `search()`, or null. |
| `search` | `(req: SearchRequest) => Promise<void>` | Start a new search. Aborts any running search first. |
| `abort` | `() => void` | Abort the current in-flight stream. No-op if idle. |

**Custom discover UI example** (from `examples/embed-demo` headless view)

```tsx
import { useKymaDiscover } from "@kyma-ai/react";

function DiscoverHeadless() {
  const { frames, isStreaming, search, abort } = useKymaDiscover();

  const rows = frames
    .filter((f) => f.type === "rows")
    .flatMap((f) => f.rows ?? []);

  return (
    <div>
      <button onClick={() => search({ query: "level == \"error\"", databases: ["prod"] })}>
        Search errors
      </button>
      {isStreaming && <button onClick={abort}>Stop</button>}
      <p>{rows.length} rows</p>
    </div>
  );
}
```

---

## `useKymaDashboards`

```ts
import { useKymaDashboards } from "@kyma-ai/react";
import type { UseKymaDashboardsResult } from "@kyma-ai/react";
```

React Query wrapper over `client.dashboards.listDashboards()`. Exposes the
dashboard list and a `getDashboard(id)` helper for on-demand detail loads.
CRUD mutations (create, update, delete) are intentionally not in this hook —
drive them directly via `useKymaClient().dashboards.*`.

```ts
function useKymaDashboards(): UseKymaDashboardsResult
```

**`UseKymaDashboardsResult`**

| Field | Type | Description |
|---|---|---|
| `dashboards` | `Dashboard[]` | All dashboards for the current database. |
| `isLoading` | `boolean` | True while the list is loading. |
| `error` | `unknown` | Error from the list fetch, or null. |
| `getDashboard` | `(id: string) => Promise<DashboardWithPanels>` | Fetch a single dashboard with panels. Uses the React Query cache when available. |
| `refetch` | `() => void` | Invalidate and refetch the dashboard list. |

---

## `useKymaAgent`

```ts
import { useKymaAgent } from "@kyma-ai/react";
import type { UseKymaAgentArgs, UseKymaAgentResult, AgentMessage, AgentStatus } from "@kyma-ai/react";
```

Headless agent hook over `POST /v1/agent/ask` (SSE stream, Vercel AI UI Message
Stream v1 protocol). Manages the message list, streaming state, and multi-turn
session resume internally.

```ts
function useKymaAgent(args?: UseKymaAgentArgs): UseKymaAgentResult
```

**`UseKymaAgentArgs`**

| Field | Type | Description |
|---|---|---|
| `database` | `string?` | Database sent with each ask request. Defaults to the transport's database. |
| `includeThinking` | `boolean?` | Request extended thinking tokens (model-dependent). |

**`UseKymaAgentResult`**

| Field | Type | Description |
|---|---|---|
| `messages` | `AgentMessage[]` | All messages in the conversation (user + assistant). |
| `status` | `AgentStatus` | `"idle" \| "streaming" \| "error"` |
| `send` | `(text: string) => Promise<void>` | Send a user message and stream the assistant reply. Aborts any prior in-flight stream. |
| `stop` | `() => void` | Abort the current stream. No-op if idle. |

**`AgentMessage`**

```ts
interface AgentMessage {
  id: string;
  role: "user" | "assistant";
  text: string;                  // completed text
  streamingText?: string;        // partially accumulated while streaming
}
```

**Custom agent UI example**

```tsx
import { useKymaAgent } from "@kyma-ai/react";

function ChatWidget() {
  const { messages, status, send } = useKymaAgent();
  const [input, setInput] = useState("");

  return (
    <div>
      {messages.map((m) => (
        <p key={m.id}><strong>{m.role}</strong>: {m.streamingText ?? m.text}</p>
      ))}
      <input value={input} onChange={(e) => setInput(e.target.value)} />
      <button disabled={status === "streaming"} onClick={() => { send(input); setInput(""); }}>
        Send
      </button>
    </div>
  );
}
```

---

## `useKymaCapabilities`

```ts
import { useKymaCapabilities } from "@kyma-ai/react";
```

React Query wrapper over `GET /v1/capabilities`. Returns the server's feature
flag set. Used internally by `CapabilityGate` (e.g. to guard `KymaAgentChat`).

```ts
function useKymaCapabilities(): UseQueryResult<Capabilities>
```

`Capabilities` is a flat object with boolean fields
(`mode`, `data_sources`, `credentials`, `oauth`, `saved_views`, `users_admin`,
`explore_live`). `staleTime` is 5 minutes.

---

## `useKymaClient`

```ts
import { useKymaClient } from "@kyma-ai/react";
import type { KymaContextValue } from "@kyma-ai/react";
```

Returns the `KymaClient` from context. Use this to call the full client API
(graph, query, discover, dashboards, data sources, etc.) from any component
inside the provider.

```tsx
import { useKymaClient } from "@kyma-ai/react";

function ListDatabases() {
  const client = useKymaClient();

  useEffect(() => {
    client.catalog.fetchSchema().then((schema) => {
      console.log(schema.databases.map((d) => d.name));
    });
  }, [client]);
}
```
