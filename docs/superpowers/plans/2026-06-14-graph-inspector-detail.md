# Graph Inspector Detail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the graph node inspector readable — inline-expandable property rows, a wide "View details" modal with markdown content + untruncated properties, and a paged source-file viewer for nodes that reference a stored artifact.

**Architecture:** Five small, focused frontend units in `packages/react/src`. A shared `<Markdown>` is extracted from the existing dashboard renderer; pure `node-detail` helpers decide content/source/formatting; `ArtifactSourceViewer` pages the existing `/v1/artifacts/by-path` API via `usePensieveClient()`; `NodeDetailModal` composes them in a Radix dialog; `InspectorPanel` gains expandable rows + a button that opens the modal. No server changes, no graph-store changes.

**Tech Stack:** React + TypeScript, `@pensieve-ai/react` package (`pv-` Tailwind prefix), Radix Dialog primitive, `@pensieve-ai/client` (`client.artifacts.fetchArtifactByPath`), Vitest + Testing Library.

**Reference spec:** `docs/superpowers/specs/2026-06-14-graph-inspector-detail-design.md`

**Conventions for every task:**
- Run a single test file from the repo root with:
  `pnpm --filter @pensieve-ai/react exec vitest run <path-relative-to-packages/react>`
- All class names use the `pv-` prefix (package CSS isolation).
- Commit after each task. Branch: `worktree-graph-inspector-detail`.

---

## File Structure

| File | Responsibility | Action |
| --- | --- | --- |
| `packages/react/src/internal/ui/markdown.tsx` | Shared markdown string → React nodes (`renderMarkdown`, `<Markdown>`) | Create |
| `packages/react/src/internal/ui/markdown.test.tsx` | Markdown render tests | Create |
| `packages/react/src/dashboards/panels/MarkdownPanelViz.tsx` | Dashboard markdown panel | Modify (import shared renderer) |
| `packages/react/src/graph/node-detail.ts` | Pure helpers: `nodeContent`, `nodeSourcePath`, `formatValue`, `orderedProps` | Create |
| `packages/react/src/graph/node-detail.test.ts` | Helper unit tests | Create |
| `packages/react/src/graph/ArtifactSourceViewer.tsx` | Paged source-file viewer (artifacts API) | Create |
| `packages/react/src/graph/ArtifactSourceViewer.test.tsx` | Viewer component tests | Create |
| `packages/react/src/graph/NodeDetailModal.tsx` | Wide dialog: content + properties + source | Create |
| `packages/react/src/graph/NodeDetailModal.test.tsx` | Modal component tests | Create |
| `packages/react/src/graph/InspectorPanel.tsx` | Docked panel: expandable rows + "View details" | Modify |
| `packages/react/src/graph/InspectorPanel.test.tsx` | Panel interaction tests | Create |

---

## Task 1: Extract shared `<Markdown>` from MarkdownPanelViz

**Files:**
- Create: `packages/react/src/internal/ui/markdown.tsx`
- Create: `packages/react/src/internal/ui/markdown.test.tsx`
- Modify: `packages/react/src/dashboards/panels/MarkdownPanelViz.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/internal/ui/markdown.test.tsx`:

```tsx
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Markdown } from "./markdown";

afterEach(cleanup);

describe("Markdown", () => {
  it("renders headings, bold, inline code, links, lists and fenced code", () => {
    const md = [
      "# Title",
      "",
      "Some **bold** and `code` and a [link](https://x.test).",
      "",
      "- one",
      "- two",
      "",
      "```",
      "fenced text",
      "```",
    ].join("\n");
    render(<Markdown source={md} />);

    expect(screen.getByRole("heading", { level: 1, name: "Title" })).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(screen.getByText("code").tagName).toBe("CODE");
    const link = screen.getByRole("link", { name: "link" });
    expect(link.getAttribute("href")).toBe("https://x.test");
    expect(screen.getByText("one")).toBeTruthy();
    expect(screen.getByText("two")).toBeTruthy();
    expect(screen.getByText("fenced text")).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/internal/ui/markdown.test.tsx`
Expected: FAIL — `Failed to resolve import "./markdown"`.

- [ ] **Step 3: Create the shared markdown module**

Create `packages/react/src/internal/ui/markdown.tsx`. Move the `renderMarkdown` function **verbatim** out of `MarkdownPanelViz.tsx` (same feature set: headings, bold/italic/code/links, lists, fenced code) and add the `<Markdown>` wrapper:

```tsx
/**
 * Shared minimal markdown renderer — headings, paragraphs, bold, italic, code,
 * links, unordered lists, fenced code blocks. Zero external dependency (avoids
 * bundle bloat). Used by the dashboard markdown panel and the graph inspector.
 */
import * as React from "react";

export function renderMarkdown(md: string): React.ReactNode[] {
  const lines = md.split("\n");
  const nodes: React.ReactNode[] = [];
  let i = 0;

  const renderInline = (text: string, key: string): React.ReactNode => {
    const parts: React.ReactNode[] = [];
    let remaining = text;
    let pIdx = 0;

    while (remaining.length > 0) {
      const boldMatch = remaining.match(/^(.*?)\*\*(.+?)\*\*(.*)/s);
      const italicMatch = remaining.match(/^(.*?)(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)(.*)/s);
      const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)/s);
      const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)/s);

      const candidates: Array<{ idx: number; type: string; match: RegExpMatchArray }> = [];
      if (boldMatch) candidates.push({ idx: boldMatch[1].length, type: "bold", match: boldMatch });
      if (italicMatch) candidates.push({ idx: italicMatch[1].length, type: "italic", match: italicMatch });
      if (codeMatch) candidates.push({ idx: codeMatch[1].length, type: "code", match: codeMatch });
      if (linkMatch) candidates.push({ idx: linkMatch[1].length, type: "link", match: linkMatch });

      if (candidates.length === 0) {
        parts.push(remaining);
        break;
      }

      candidates.sort((a, b) => a.idx - b.idx);
      const best = candidates[0];

      if (best.match[1]) parts.push(best.match[1]);
      const subKey = `${key}-${pIdx++}`;
      if (best.type === "bold") {
        parts.push(<strong key={subKey}>{best.match[2]}</strong>);
        remaining = best.match[3];
      } else if (best.type === "italic") {
        parts.push(<em key={subKey}>{best.match[2]}</em>);
        remaining = best.match[4];
      } else if (best.type === "code") {
        parts.push(
          <code key={subKey} className="pv-rounded pv-bg-muted pv-px-1 pv-font-mono pv-text-[0.85em]">
            {best.match[2]}
          </code>,
        );
        remaining = best.match[3];
      } else if (best.type === "link") {
        parts.push(
          <a
            key={subKey}
            href={best.match[3]}
            target="_blank"
            rel="noopener noreferrer"
            className="pv-text-primary pv-underline"
          >
            {best.match[2]}
          </a>,
        );
        remaining = best.match[4];
      }
    }
    return <>{parts}</>;
  };

  while (i < lines.length) {
    const line = lines[i];

    if (line.startsWith("```")) {
      const fenceLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        fenceLines.push(lines[i]);
        i++;
      }
      i++;
      nodes.push(
        <pre
          key={`fence-${i}`}
          className="pv-my-2 pv-overflow-x-auto pv-rounded pv-bg-muted pv-p-3 pv-font-mono pv-text-xs"
        >
          <code>{fenceLines.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    if (line.startsWith("### ")) {
      nodes.push(
        <h3 key={`h3-${i}`} className="pv-mt-3 pv-mb-1 pv-text-sm pv-font-semibold">
          {renderInline(line.slice(4), `h3-${i}`)}
        </h3>,
      );
      i++;
      continue;
    }
    if (line.startsWith("## ")) {
      nodes.push(
        <h2 key={`h2-${i}`} className="pv-mt-4 pv-mb-1 pv-text-base pv-font-semibold">
          {renderInline(line.slice(3), `h2-${i}`)}
        </h2>,
      );
      i++;
      continue;
    }
    if (line.startsWith("# ")) {
      nodes.push(
        <h1 key={`h1-${i}`} className="pv-mt-2 pv-mb-2 pv-text-lg pv-font-bold">
          {renderInline(line.slice(2), `h1-${i}`)}
        </h1>,
      );
      i++;
      continue;
    }

    if (line.startsWith("- ") || line.startsWith("* ")) {
      const listItems: React.ReactNode[] = [];
      while (i < lines.length && (lines[i].startsWith("- ") || lines[i].startsWith("* "))) {
        listItems.push(
          <li key={`li-${i}`} className="pv-ml-4 pv-list-disc">
            {renderInline(lines[i].slice(2), `li-${i}`)}
          </li>,
        );
        i++;
      }
      nodes.push(
        <ul key={`ul-${i}`} className="pv-my-1 pv-space-y-0.5 pv-text-sm">
          {listItems}
        </ul>,
      );
      continue;
    }

    if (line.trim() === "") {
      i++;
      continue;
    }

    nodes.push(
      <p key={`p-${i}`} className="pv-my-1 pv-text-sm pv-leading-relaxed">
        {renderInline(line, `p-${i}`)}
      </p>,
    );
    i++;
  }

  return nodes;
}

/** Render a markdown string as React nodes. Callers control the wrapping layout. */
export function Markdown({ source }: { source: string }) {
  return <>{renderMarkdown(source)}</>;
}
```

- [ ] **Step 4: Refactor MarkdownPanelViz to use the shared renderer**

Replace the entire contents of `packages/react/src/dashboards/panels/MarkdownPanelViz.tsx` with:

```tsx
/**
 * MarkdownPanelViz — renders a markdown-type dashboard panel using the shared
 * minimal markdown renderer (packages/react/src/internal/ui/markdown.tsx).
 */

import type { DashboardPanel } from "@pensieve-ai/client";
import { renderMarkdown } from "../../internal/ui/markdown";

interface Props {
  panel: DashboardPanel;
}

export function MarkdownPanelViz({ panel }: Props) {
  const markdown = (panel.config.markdown as string | undefined) ?? "";

  if (!markdown.trim()) {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground">
        No content.
      </div>
    );
  }

  return <div className="pv-h-full pv-overflow-y-auto pv-px-3 pv-py-2">{renderMarkdown(markdown)}</div>;
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/internal/ui/markdown.test.tsx src/dashboards`
Expected: PASS — new markdown test passes and existing dashboard/markdown-panel tests still pass (unchanged output).

- [ ] **Step 6: Commit**

```bash
git add packages/react/src/internal/ui/markdown.tsx \
        packages/react/src/internal/ui/markdown.test.tsx \
        packages/react/src/dashboards/panels/MarkdownPanelViz.tsx
git commit -m "refactor(react): extract shared <Markdown> renderer from MarkdownPanelViz"
```

---

## Task 2: `node-detail` pure helpers

**Files:**
- Create: `packages/react/src/graph/node-detail.ts`
- Create: `packages/react/src/graph/node-detail.test.ts`

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/graph/node-detail.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import type { GraphNode } from "@pensieve-ai/client";
import { nodeContent, nodeSourcePath, formatValue, orderedProps } from "./node-detail";

function node(properties: Record<string, unknown>, labels: string[] = ["Memory"]): GraphNode {
  return {
    id: "memory:abc",
    labels,
    properties,
    metadata: { created_at: "", updated_at: "", realm: "default" },
    namespace: "memory",
  };
}

describe("nodeContent", () => {
  it("returns a non-empty string content, else null", () => {
    expect(nodeContent(node({ content: "hello" }))).toBe("hello");
    expect(nodeContent(node({ content: "  " }))).toBeNull();
    expect(nodeContent(node({}))).toBeNull();
    expect(nodeContent(node({ content: 42 }))).toBeNull();
  });
});

describe("nodeSourcePath", () => {
  it("returns object_path when a non-empty string, else null", () => {
    expect(nodeSourcePath(node({ object_path: "artifacts/t/logs/x.log" }))).toBe("artifacts/t/logs/x.log");
    expect(nodeSourcePath(node({}))).toBeNull();
    expect(nodeSourcePath(node({ object_path: "" }))).toBeNull();
  });
});

describe("formatValue", () => {
  it("passes scalars through", () => {
    expect(formatValue("plain")).toEqual({ text: "plain", kind: "scalar" });
    expect(formatValue(0.7)).toEqual({ text: "0.7", kind: "scalar" });
    expect(formatValue(null)).toEqual({ text: "null", kind: "scalar" });
  });

  it("pretty-prints object values and JSON-string values", () => {
    expect(formatValue({ source: "claude" })).toEqual({
      text: '{\n  "source": "claude"\n}',
      kind: "json",
    });
    expect(formatValue('{"source":"claude"}')).toEqual({
      text: '{\n  "source": "claude"\n}',
      kind: "json",
    });
  });

  it("collapses large numeric arrays and embedding-like strings", () => {
    const arr = Array.from({ length: 40 }, (_, i) => i / 100);
    expect(formatValue(arr)).toEqual({ text: "float[40]", kind: "array" });
    const embStr = arr.map((n) => String(n)).join(",");
    expect(formatValue(embStr)).toEqual({ text: "float[40]", kind: "array" });
  });
});

describe("orderedProps", () => {
  it("puts content first, title second, embedding last, rest alphabetical", () => {
    const keys = orderedProps(
      node({ status: "active", embedding: "x", title: "T", content: "C", importance: 0.7 }),
    ).map(([k]) => k);
    expect(keys).toEqual(["content", "title", "importance", "status", "embedding"]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/node-detail.test.ts`
Expected: FAIL — `Failed to resolve import "./node-detail"`.

- [ ] **Step 3: Write the implementation**

Create `packages/react/src/graph/node-detail.ts`:

```ts
/**
 * Pure helpers for the graph node inspector. No React — easy to unit test.
 * Decide which property is the renderable "content", whether a node points at a
 * fetchable source artifact (`object_path`), and how to format property values
 * for display (pretty JSON, collapsed embeddings).
 */
import type { GraphNode } from "@pensieve-ai/client";

/** The node's renderable long-text content (`properties.content`) or null. */
export function nodeContent(node: GraphNode): string | null {
  const c = node.properties?.content;
  return typeof c === "string" && c.trim() !== "" ? c : null;
}

/** The object-store key of a fetchable source file (`properties.object_path`) or null. */
export function nodeSourcePath(node: GraphNode): string | null {
  const p = node.properties?.object_path;
  return typeof p === "string" && p.trim() !== "" ? p : null;
}

export type FormattedValue = { text: string; kind: "scalar" | "json" | "array" };

const COLLAPSE_LEN = 24; // collapse numeric arrays/embeddings longer than this

function allNumeric(parts: string[]): boolean {
  return parts.every((p) => p.trim() !== "" && !Number.isNaN(Number(p)));
}

/**
 * Format a property value for display:
 * - large numeric arrays / comma-joined number strings (embeddings) → `float[N]`
 * - objects and JSON-looking strings → pretty-printed JSON
 * - everything else → `String(v)`
 */
export function formatValue(v: unknown): FormattedValue {
  if (Array.isArray(v)) {
    if (v.length > COLLAPSE_LEN && v.every((x) => typeof x === "number")) {
      return { text: `float[${v.length}]`, kind: "array" };
    }
    return { text: JSON.stringify(v, null, 2), kind: "json" };
  }
  if (v !== null && typeof v === "object") {
    return { text: JSON.stringify(v, null, 2), kind: "json" };
  }
  if (typeof v === "string") {
    const t = v.trim();
    // Embedding rendered as a bare comma-separated number list.
    const commaParts = t.split(",");
    if (commaParts.length > COLLAPSE_LEN && allNumeric(commaParts)) {
      return { text: `float[${commaParts.length}]`, kind: "array" };
    }
    // JSON-looking string (e.g. provenance is stored as a JSON string).
    if ((t.startsWith("{") && t.endsWith("}")) || (t.startsWith("[") && t.endsWith("]"))) {
      try {
        const parsed = JSON.parse(t);
        if (Array.isArray(parsed) && parsed.length > COLLAPSE_LEN && parsed.every((x) => typeof x === "number")) {
          return { text: `float[${parsed.length}]`, kind: "array" };
        }
        return { text: JSON.stringify(parsed, null, 2), kind: "json" };
      } catch {
        // not valid JSON — fall through to scalar
      }
    }
    return { text: v, kind: "scalar" };
  }
  return { text: String(v), kind: "scalar" };
}

/** Property entries in a stable display order: content, title, …alpha…, embedding last. */
export function orderedProps(node: GraphNode): Array<[string, unknown]> {
  const rank = (k: string): number => {
    if (k === "content") return 0;
    if (k === "title") return 1;
    if (k === "embedding") return 99;
    return 50;
  };
  return Object.entries(node.properties ?? {}).sort((a, b) => {
    const ra = rank(a[0]);
    const rb = rank(b[0]);
    return ra !== rb ? ra - rb : a[0].localeCompare(b[0]);
  });
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/node-detail.test.ts`
Expected: PASS — all helper assertions pass.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/node-detail.ts packages/react/src/graph/node-detail.test.ts
git commit -m "feat(graph): node-detail helpers (content/source/formatValue/orderedProps)"
```

---

## Task 3: `ArtifactSourceViewer` — paged source-file viewer

**Files:**
- Create: `packages/react/src/graph/ArtifactSourceViewer.tsx`
- Create: `packages/react/src/graph/ArtifactSourceViewer.test.tsx`

Background: `client.artifacts.fetchArtifactByPath(path, { offset })` returns
`{ object_path, size_bytes, offset, returned_bytes, eof, content }`. We page by
advancing `offset` by `returned_bytes` until `eof`.

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/graph/ArtifactSourceViewer.test.tsx`:

```tsx
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { ArtifactSourceViewer } from "./ArtifactSourceViewer";

function windowResponse(offset: number, content: string, eof: boolean, size: number) {
  return new Response(
    JSON.stringify({
      object_path: "artifacts/t/logs/build.log",
      size_bytes: size,
      offset,
      returned_bytes: content.length,
      eof,
      content,
    }),
    { status: 200, headers: new Headers({ "content-type": "application/json" }) },
  );
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("ArtifactSourceViewer", () => {
  it("loads the first window and pages with Load more until eof", async () => {
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/v1/artifacts/by-path")) {
        const m = url.match(/offset=(\d+)/);
        const offset = m ? Number(m[1]) : 0;
        return Promise.resolve(
          offset === 0
            ? windowResponse(0, "first-chunk", false, 23)
            : windowResponse(11, "second-chunk", true, 23),
        );
      }
      return Promise.resolve(new Response("", { status: 404 }));
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <ArtifactSourceViewer path="artifacts/t/logs/build.log" />
      </PensieveProvider>,
    );

    await waitFor(() =>
      expect(screen.getByText((c) => c.includes("first-chunk"))).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: /load more/i }));

    await waitFor(() =>
      expect(screen.getByText((c) => c.includes("first-chunksecond-chunk"))).toBeTruthy(),
    );
    expect(screen.queryByRole("button", { name: /load more/i })).toBeNull();
  });

  it("shows an error with retry when the fetch fails", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("nope", { status: 500 })));

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <ArtifactSourceViewer path="artifacts/t/logs/build.log" />
      </PensieveProvider>,
    );

    await waitFor(() => expect(screen.getByRole("button", { name: /retry/i })).toBeTruthy());
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/ArtifactSourceViewer.test.tsx`
Expected: FAIL — `Failed to resolve import "./ArtifactSourceViewer"`.

- [ ] **Step 3: Write the implementation**

Create `packages/react/src/graph/ArtifactSourceViewer.tsx`:

```tsx
/**
 * ArtifactSourceViewer — read the source file a graph node references, by paging
 * the stored artifact at its `object_path` through the existing
 * `/v1/artifacts/by-path` API (client.artifacts.fetchArtifactByPath). Monospace,
 * wrap toggle, copy-all, byte counter, "Load more" until EOF, error + retry.
 */
import { useEffect, useState } from "react";
import { Copy, WrapText } from "lucide-react";
import { Button } from "../internal/ui/button";
import { usePensieveClient } from "../provider/context";

export function ArtifactSourceViewer({ path }: { path: string }) {
  const client = usePensieveClient();
  const [text, setText] = useState("");
  const [offset, setOffset] = useState(0);
  const [size, setSize] = useState<number | null>(null);
  const [eof, setEof] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wrap, setWrap] = useState(true);

  async function fetchWindow(atOffset: number, append: boolean) {
    setLoading(true);
    setError(null);
    try {
      const w = await client.artifacts.fetchArtifactByPath(path, { offset: atOffset });
      setText((prev) => (append ? prev + w.content : w.content));
      setOffset(w.offset + w.returned_bytes);
      setSize(w.size_bytes);
      setEof(w.eof);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load source file.");
    } finally {
      setLoading(false);
    }
  }

  // Reset and load the first window whenever the path changes.
  useEffect(() => {
    setText("");
    setOffset(0);
    setSize(null);
    setEof(false);
    setError(null);
    void fetchWindow(0, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, client]);

  return (
    <div className="pv-space-y-2">
      <div className="pv-flex pv-items-center pv-justify-between pv-gap-2">
        <div className="pv-truncate pv-font-mono pv-text-2xs pv-text-muted-foreground" title={path}>
          {path}
        </div>
        <div className="pv-flex pv-items-center pv-gap-1">
          <Button variant="ghost" size="xs" onClick={() => setWrap((w) => !w)}>
            <WrapText className="pv-h-3.5 pv-w-3.5" /> {wrap ? "No wrap" : "Wrap"}
          </Button>
          <Button
            variant="ghost"
            size="xs"
            disabled={!text}
            onClick={() => void navigator.clipboard.writeText(text)}
          >
            <Copy className="pv-h-3.5 pv-w-3.5" /> Copy
          </Button>
        </div>
      </div>

      {error ? (
        <div className="pv-rounded pv-border pv-border-destructive/40 pv-bg-destructive/10 pv-p-2 pv-text-xs pv-text-destructive">
          <div className="pv-mb-1">{error}</div>
          <Button variant="outline" size="xs" onClick={() => void fetchWindow(offset, true)}>
            Retry
          </Button>
        </div>
      ) : (
        <>
          <pre
            className={`pv-max-h-80 pv-overflow-auto pv-rounded pv-bg-muted pv-p-2 pv-font-mono pv-text-2xs ${
              wrap ? "pv-whitespace-pre-wrap pv-break-words" : "pv-whitespace-pre"
            }`}
          >
            {text || (loading ? "Loading…" : "No content.")}
          </pre>
          <div className="pv-flex pv-items-center pv-justify-between pv-text-2xs pv-text-muted-foreground">
            <span>{size != null ? `${Math.min(offset, size)} / ${size} bytes` : ""}</span>
            {!eof && !!text && (
              <Button
                variant="outline"
                size="xs"
                disabled={loading}
                onClick={() => void fetchWindow(offset, true)}
              >
                {loading ? "Loading…" : "Load more"}
              </Button>
            )}
          </div>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/ArtifactSourceViewer.test.tsx`
Expected: PASS — first window renders, "Load more" appends and disappears at eof, error path shows Retry.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/ArtifactSourceViewer.tsx packages/react/src/graph/ArtifactSourceViewer.test.tsx
git commit -m "feat(graph): ArtifactSourceViewer — paged source-file viewer via artifacts API"
```

---

## Task 4: `NodeDetailModal` — wide content + properties + source modal

**Files:**
- Create: `packages/react/src/graph/NodeDetailModal.tsx`
- Create: `packages/react/src/graph/NodeDetailModal.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/graph/NodeDetailModal.test.tsx`:

```tsx
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { GraphNode } from "@pensieve-ai/client";
import { PensieveProvider } from "../provider/PensieveProvider";
import { NodeDetailModal } from "./NodeDetailModal";

// jsdom can't run Radix portals/animations — replace the dialog primitive with
// simple pass-through elements (same approach as PensieveDashboard.test.tsx).
vi.mock("@radix-ui/react-dialog", () => {
  const Root = ({ open, children }: { open?: boolean; children: React.ReactNode }) =>
    open ? <div data-testid="dialog-root">{children}</div> : null;
  const Trigger = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Portal = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Overlay = () => <div data-testid="dialog-overlay" />;
  const Content = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="dialog-content">{children}</div>
  );
  const Header = ({ children }: { children: React.ReactNode }) => <div>{children}</div>;
  const Title = ({ children }: { children: React.ReactNode }) => <h2>{children}</h2>;
  const Close = ({ children }: { children: React.ReactNode }) => <button>{children}</button>;
  return { Root, Trigger, Portal, Overlay, Content, Header, Title, Close };
});

function makeNode(properties: Record<string, unknown>): GraphNode {
  return {
    id: "memory:abc",
    labels: ["Memory"],
    properties,
    metadata: { created_at: "", updated_at: "", realm: "default" },
    namespace: "memory",
  };
}

function renderModal(node: GraphNode) {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          object_path: "artifacts/t/logs/build.log",
          size_bytes: 5,
          offset: 0,
          returned_bytes: 5,
          eof: true,
          content: "hello",
        }),
        { status: 200, headers: new Headers({ "content-type": "application/json" }) },
      ),
    ),
  );
  return render(
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
      <NodeDetailModal node={node} open onClose={() => {}} />
    </PensieveProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("NodeDetailModal", () => {
  it("renders content as markdown and lists all properties untruncated", () => {
    renderModal(makeNode({ content: "# Heading\n\nbody text", importance: 0.7 }));
    expect(screen.getByRole("heading", { level: 1, name: "Heading" })).toBeTruthy();
    expect(screen.getByText("importance")).toBeTruthy();
    expect(screen.getByText("0.7")).toBeTruthy();
  });

  it("omits the source section when there is no object_path", () => {
    renderModal(makeNode({ content: "x" }));
    expect(screen.queryByText(/source file/i)).toBeNull();
  });

  it("shows the source section when the node has an object_path", () => {
    renderModal(makeNode({ content: "x", object_path: "artifacts/t/logs/build.log" }));
    expect(screen.getByText(/source file/i)).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/NodeDetailModal.test.tsx`
Expected: FAIL — `Failed to resolve import "./NodeDetailModal"`.

- [ ] **Step 3: Write the implementation**

Create `packages/react/src/graph/NodeDetailModal.tsx`:

```tsx
/**
 * NodeDetailModal — wide modal for a graph node. Renders the node's `content`
 * as markdown (raw/rendered toggle), every property untruncated with per-value
 * copy, and (when the node references a stored artifact via `object_path`) a
 * paged source-file viewer. Built on the internal Radix Dialog primitive.
 */
import { useState } from "react";
import { Copy } from "lucide-react";
import type { GraphNode } from "@pensieve-ai/client";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../internal/ui/dialog";
import { Button } from "../internal/ui/button";
import { Markdown } from "../internal/ui/markdown";
import { ArtifactSourceViewer } from "./ArtifactSourceViewer";
import { nodeContent, nodeSourcePath, orderedProps, formatValue } from "./node-detail";

export function NodeDetailModal({
  node,
  open,
  onClose,
}: {
  node: GraphNode | null;
  open: boolean;
  onClose: () => void;
}) {
  const [raw, setRaw] = useState(false);
  if (!node) return null;

  const name = (node.properties?.name as string) || (node.properties?.title as string) || node.id;
  const content = nodeContent(node);
  const sourcePath = nodeSourcePath(node);

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
    >
      <DialogContent className="pv-max-w-3xl pv-max-h-[85vh] pv-overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="pv-truncate pv-pr-8">{name}</DialogTitle>
          <div className="pv-text-2xs pv-text-muted-foreground">
            {node.labels.join(" · ")} · {node.namespace}
          </div>
        </DialogHeader>

        {content != null && (
          <section className="pv-space-y-1">
            <div className="pv-flex pv-items-center pv-justify-between">
              <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Content</div>
              <Button variant="ghost" size="xs" onClick={() => setRaw((r) => !r)}>
                {raw ? "Rendered" : "Raw"}
              </Button>
            </div>
            {raw ? (
              <pre className="pv-overflow-x-auto pv-rounded pv-bg-muted pv-p-3 pv-font-mono pv-text-xs pv-whitespace-pre-wrap pv-break-words">
                {content}
              </pre>
            ) : (
              <div className="pv-rounded pv-border pv-border-border pv-p-3">
                <Markdown source={content} />
              </div>
            )}
          </section>
        )}

        <section className="pv-space-y-1">
          <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Properties</div>
          <dl className="pv-space-y-1.5">
            {orderedProps(node).map(([k, v]) => {
              const f = formatValue(v);
              return (
                <div key={k} className="pv-group pv-grid pv-grid-cols-[140px_1fr] pv-gap-2 pv-text-xs">
                  <dt className="pv-truncate pv-text-muted-foreground" title={k}>
                    {k}
                  </dt>
                  <dd className="pv-flex pv-min-w-0 pv-items-start pv-gap-1">
                    <span className="pv-min-w-0 pv-flex-1 pv-whitespace-pre-wrap pv-break-words pv-font-mono pv-text-foreground">
                      {f.text}
                    </span>
                    <button
                      type="button"
                      onClick={() => void navigator.clipboard.writeText(f.text)}
                      className="pv-text-muted-foreground pv-opacity-0 group-hover:pv-opacity-100 hover:pv-text-foreground"
                      aria-label={`copy ${k}`}
                    >
                      <Copy className="pv-h-3 pv-w-3" />
                    </button>
                  </dd>
                </div>
              );
            })}
          </dl>
        </section>

        {sourcePath != null && (
          <section className="pv-space-y-1">
            <div className="pv-text-2xs pv-uppercase pv-text-muted-foreground">Source file</div>
            <ArtifactSourceViewer path={sourcePath} />
          </section>
        )}
      </DialogContent>
    </Dialog>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/NodeDetailModal.test.tsx`
Expected: PASS — markdown heading renders, properties listed, source section gated on `object_path`.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/NodeDetailModal.tsx packages/react/src/graph/NodeDetailModal.test.tsx
git commit -m "feat(graph): NodeDetailModal — markdown content + full properties + source viewer"
```

---

## Task 5: `InspectorPanel` — expandable rows + "View details" button

**Files:**
- Modify: `packages/react/src/graph/InspectorPanel.tsx`
- Create: `packages/react/src/graph/InspectorPanel.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `packages/react/src/graph/InspectorPanel.test.tsx`:

```tsx
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { GraphNode } from "@pensieve-ai/client";
import { InspectorPanel } from "./InspectorPanel";
import { GraphStoreContext, createGraphStore } from "./graph-store";

// Pass-through dialog primitive (same as NodeDetailModal.test.tsx).
vi.mock("@radix-ui/react-dialog", () => {
  const Root = ({ open, children }: { open?: boolean; children: React.ReactNode }) =>
    open ? <div data-testid="dialog-root">{children}</div> : null;
  const Trigger = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Portal = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Overlay = () => <div data-testid="dialog-overlay" />;
  const Content = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="dialog-content">{children}</div>
  );
  const Header = ({ children }: { children: React.ReactNode }) => <div>{children}</div>;
  const Title = ({ children }: { children: React.ReactNode }) => <h2>{children}</h2>;
  const Close = ({ children }: { children: React.ReactNode }) => <button>{children}</button>;
  return { Root, Trigger, Portal, Overlay, Content, Header, Title, Close };
});

const NODE: GraphNode = {
  id: "memory:abc",
  labels: ["Memory"],
  properties: {
    title: "The 3-bug chain",
    content: "long body content here",
    topic_key: "claude-md:-Users-shaked-very-long-topic-key-value-that-truncates",
  },
  metadata: { created_at: "", updated_at: "", realm: "default" },
  namespace: "memory",
};

function setup() {
  const store = createGraphStore({});
  return render(
    <GraphStoreContext.Provider value={store}>
      <InspectorPanel node={NODE} edges={[]} nodesByCompositeId={new Map()} />
    </GraphStoreContext.Provider>,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("InspectorPanel", () => {
  it("opens the detail modal when 'View details' is clicked", () => {
    setup();
    expect(screen.queryByTestId("dialog-content")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /view details/i }));
    expect(screen.getByTestId("dialog-content")).toBeTruthy();
  });

  it("expands a property row to reveal its full value", () => {
    setup();
    // topic_key value is truncated until expanded.
    const expandBtn = screen.getByRole("button", { name: /expand topic_key/i });
    fireEvent.click(expandBtn);
    expect(screen.getByRole("button", { name: /collapse topic_key/i })).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/InspectorPanel.test.tsx`
Expected: FAIL — no "View details" button / no "expand topic_key" button exists yet.

- [ ] **Step 3: Edit InspectorPanel — imports + hooks**

In `packages/react/src/graph/InspectorPanel.tsx`, update the import block at the top. Replace:

```tsx
import { useMemo } from "react";
import { ArrowLeft, ArrowRight, Copy, Crosshair, X } from "lucide-react";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";
import { getRelationshipFamilyColor } from "./graph-style";
```

with:

```tsx
import { useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, ChevronDown, ChevronRight, Copy, Crosshair, Maximize2, X } from "lucide-react";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";
import { getRelationshipFamilyColor } from "./graph-style";
import { NodeDetailModal } from "./NodeDetailModal";
import { orderedProps, formatValue } from "./node-detail";
```

- [ ] **Step 4: Edit InspectorPanel — add a `PropertyRow` subcomponent**

Add this component at the bottom of `packages/react/src/graph/InspectorPanel.tsx`, after the `InspectorPanel` function:

```tsx
/** One property row in the docked panel: truncated by default, expandable to the
 * full wrapped value, with a hover copy button. */
function PropertyRow({ propKey, value }: { propKey: string; value: unknown }) {
  const [open, setOpen] = useState(false);
  const full = formatValue(value).text;
  return (
    <div className="pv-group pv-flex pv-items-start pv-gap-1 pv-text-xs">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="pv-mt-0.5 pv-text-muted-foreground hover:pv-text-foreground"
        aria-label={`${open ? "collapse" : "expand"} ${propKey}`}
      >
        {open ? <ChevronDown className="pv-h-3 pv-w-3" /> : <ChevronRight className="pv-h-3 pv-w-3" />}
      </button>
      <dt className="pv-w-24 pv-shrink-0 pv-truncate pv-text-muted-foreground" title={propKey}>
        {propKey}
      </dt>
      <dd
        className={`pv-min-w-0 pv-flex-1 pv-font-mono pv-text-foreground ${
          open ? "pv-whitespace-pre-wrap pv-break-words" : "pv-truncate"
        }`}
      >
        {full}
      </dd>
      <button
        type="button"
        onClick={() => void navigator.clipboard.writeText(full)}
        className="pv-text-muted-foreground pv-opacity-0 group-hover:pv-opacity-100 hover:pv-text-foreground"
        aria-label={`copy ${propKey}`}
      >
        <Copy className="pv-h-3 pv-w-3" />
      </button>
    </div>
  );
}
```

- [ ] **Step 5: Edit InspectorPanel — detail state, "View details" button, rows, modal mount**

5a. Add the modal open state alongside the existing hooks. After this line (near the top of the `InspectorPanel` function body, with the other `useGraphStore` selectors):

```tsx
  const setFocusMode = useGraphStore((s) => s.setFocusMode);
```

add:

```tsx
  const [detailOpen, setDetailOpen] = useState(false);
```

5b. Add a "View details" button to the actions row. Find the actions `<div className="pv-flex pv-gap-2 pv-p-3">` block containing the Focus and Copy-id buttons, and add this button as the **first** child inside that div (before the Focus button):

```tsx
        <button
          type="button"
          onClick={() => setDetailOpen(true)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Maximize2 className="pv-h-3 pv-w-3" /> View details
        </button>
```

5c. Replace the Properties `<dl>` block. Find:

```tsx
          <dl className="pv-space-y-1">
            {Object.entries(node.properties).slice(0, 30).map(([k, v]) => (
              <div key={k} className="pv-flex pv-gap-2 pv-text-xs">
                <dt className="pv-w-28 pv-shrink-0 pv-truncate pv-text-muted-foreground">{k}</dt>
                <dd className="pv-min-w-0 pv-truncate pv-font-mono pv-text-foreground">{String(v)}</dd>
              </div>
            ))}
          </dl>
```

and replace it with:

```tsx
          <dl className="pv-space-y-1">
            {orderedProps(node).map(([k, v]) => (
              <PropertyRow key={k} propKey={k} value={v} />
            ))}
          </dl>
```

5d. Mount the modal. Find the panel's outermost closing `</div>` (the one that closes `<div className="pv-h-full pv-w-80 …">`) and add the modal just before it, after the Relationships block's closing `</div>`:

```tsx
      <NodeDetailModal node={node} open={detailOpen} onClose={() => setDetailOpen(false)} />
```

- [ ] **Step 6: Run test to verify it passes**

Run: `pnpm --filter @pensieve-ai/react exec vitest run src/graph/InspectorPanel.test.tsx`
Expected: PASS — "View details" opens the (mocked) dialog; chevron toggles a row from expand→collapse.

- [ ] **Step 7: Commit**

```bash
git add packages/react/src/graph/InspectorPanel.tsx packages/react/src/graph/InspectorPanel.test.tsx
git commit -m "feat(graph): InspectorPanel — expandable property rows + View details modal"
```

---

## Task 6: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Typecheck the package**

Run: `pnpm --filter @pensieve-ai/react typecheck`
Expected: no errors. (If `formatValue`/`orderedProps`/`Markdown` signatures drift from their call sites, fix here.)

- [ ] **Step 2: Run the full react test suite**

Run: `pnpm --filter @pensieve-ai/react test`
Expected: all tests pass, including the 5 new test files and the unchanged dashboard/markdown tests.

- [ ] **Step 3: Build the package**

Run: `pnpm --filter @pensieve-ai/react build`
Expected: build succeeds (no unused-import or TS build errors).

- [ ] **Step 4: Manual smoke (optional but recommended)**

Use the `verify` or `run` skill to launch the app, open the graph, click a Memory
node, confirm: (a) property rows expand and copy; (b) "View details" opens the
modal with markdown content + raw toggle + full properties; (c) for a LogFile /
Artifact node, the "Source file" section pages through content. Capture a
screenshot.

- [ ] **Step 5: Commit any verification fixes**

```bash
git add -A
git commit -m "chore(graph): inspector detail — typecheck/build fixes"
```

(Skip if Steps 1–3 were already green with nothing to fix.)

---

## Self-Review notes (for the implementer)

- **Spec coverage:** ask #1 (readable/full text) → Task 5 PropertyRow + Task 4 properties grid; ask #2 (markdown) → Task 1 `<Markdown>` + Task 4 Content section; ask #3 (source files) → Task 3 `ArtifactSourceViewer` + Task 4 Source section. Inline-expand + modal layout and artifacts-only source scope are both honored.
- **Type consistency:** `formatValue` returns `{ text, kind }` and is consumed via `.text` in Tasks 4 and 5; `orderedProps` returns `Array<[string, unknown]>` consumed by `.map(([k, v]) => …)` in both; `nodeContent`/`nodeSourcePath` return `string | null`, gated with `!= null`. `client.artifacts.fetchArtifactByPath(path, { offset })` matches the bound client method shape.
- **No new deps, no server changes, no graph-store changes.**
