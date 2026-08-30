/**
 * PensieveQueryEditor tests.
 *
 * Monaco is mocked at the @monaco-editor/react boundary as a <textarea
 * data-testid="monaco"> that captures value/onChange. Fetch is stubbed to
 * provide a /v1/catalog/schema fixture and a /v1/query NDJSON response.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, cleanup, waitFor, act, fireEvent } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { PensieveQueryEditor } from "./PensieveQueryEditor";
import type { SchemaDoc } from "@pensieve-ai/client";

// ── Monaco mock ───────────────────────────────────────────────────────────────

// KqlEditor imports MonacoEditor from "@monaco-editor/react". We replace it
// with a plain <textarea> that mirrors value/onChange. onMount is called with
// a stub monaco object that satisfies all the calls in KqlEditor's handleMount
// (registerKql, ensurePensieveDarkTheme, registerKqlCompletions, addCommand).
vi.mock("@monaco-editor/react", () => {
  const MockEditor = ({
    value,
    onChange,
    onMount,
    options,
  }: {
    value?: string;
    onChange?: (v: string) => void;
    onMount?: (editor: unknown, monaco: unknown) => void;
    options?: { readOnly?: boolean };
  }) => {
    const monacoStub = {
      KeyMod: { CtrlCmd: 2048 },
      KeyCode: { Enter: 3 },
      editor: {
        defineTheme: () => {},
      },
      // languages: a Proxy that returns no-op functions for any property access.
      // registerKql is guarded by getLanguages() — returning [{id:"kql"}] makes
      // it short-circuit. registerKqlCompletions calls several other methods
      // (registerCompletionItemProvider, registerSignatureHelpProvider, etc.);
      // the Proxy stubs them all to avoid enumerating every call site.
      languages: new Proxy(
        {
          getLanguages: () => [{ id: "kql" }],
          CompletionItemKind: {},
          CompletionItemInsertTextRule: { InsertAsSnippet: 4 },
        },
        {
          get(target, prop) {
            if (prop in target) return target[prop as keyof typeof target];
            // Default: return a no-op function that returns a disposable stub.
            return () => ({ dispose: () => {} });
          },
        },
      ),
    };
    const editorStub = {
      addCommand: () => {},
    };
    // Call onMount on the first render so KqlEditor wires its keybinding ref.
    React.useEffect(() => {
      onMount?.(editorStub, monacoStub);
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    return (
      <textarea
        data-testid="monaco"
        value={value ?? ""}
        readOnly={options?.readOnly}
        onChange={(e) => onChange?.(e.target.value)}
      />
    );
  };
  return { default: MockEditor };
});

// ── Fixtures ──────────────────────────────────────────────────────────────────

const SCHEMA_FIXTURE: SchemaDoc = {
  databases: [
    {
      name: "testdb",
      tables: [
        {
          name: "events",
          columns: [
            { name: "timestamp", type: "datetime", nullable: false },
            { name: "level", type: "string", nullable: true },
            { name: "msg", type: "string", nullable: true },
          ],
        },
      ],
    },
  ],
};

const QUERY_ROWS = [
  { timestamp: "2026-01-01T00:00:00Z", level: "info", msg: "hello" },
  { timestamp: "2026-01-01T00:01:00Z", level: "warn", msg: "world" },
];

function makeNdjsonResponse(rows: Record<string, unknown>[]) {
  const ndjson = rows.map((r) => JSON.stringify(r)).join("\n");
  return Promise.resolve(
    new Response(ndjson, {
      status: 200,
      headers: new Headers({ "content-type": "application/x-ndjson" }),
    }),
  );
}

function makeSchemaResponse() {
  return Promise.resolve(
    new Response(JSON.stringify(SCHEMA_FIXTURE), {
      status: 200,
      headers: new Headers({ "content-type": "application/json" }),
    }),
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("PensieveQueryEditor", () => {
  it("1. smoke: renders editor, schema browser, and run button inside PensieveProvider", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
        return makeNdjsonResponse([]);
      }),
    );

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <PensieveQueryEditor defaultQuery="events | take 10" />
      </PensieveProvider>,
    );

    // Editor textarea is present
    expect(screen.getByTestId("monaco")).toBeTruthy();

    // Run button is visible
    expect(screen.getByTestId("run-btn")).toBeTruthy();

    // Schema browser renders once schema loads
    await waitFor(() => {
      expect(screen.queryByText("events")).toBeTruthy();
    });
  });

  it("2. typing + run: POST /v1/query with correct body and content-type; results appear; onResults called", async () => {
    const qc = makeQC();
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
      if (url.includes("/v1/query")) return makeNdjsonResponse(QUERY_ROWS);
      return Promise.resolve(new Response("", { status: 404 }));
    });
    vi.stubGlobal("fetch", fetchMock);

    const onResults = vi.fn();

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <PensieveQueryEditor
          defaultQuery=""
          showTimeRange={false}
          database="testdb"
          onResults={onResults}
        />
      </PensieveProvider>,
    );

    const textarea = screen.getByTestId("monaco");

    // Type a query (fireEvent.change is sync and flushes React state)
    act(() => {
      fireEvent.change(textarea, { target: { value: "events | take 5" } });
    });

    // Click run and wait for the async execute() to complete
    const runBtn = screen.getByTestId("run-btn");
    await act(async () => {
      fireEvent.click(runBtn);
      // Small yield to let the async query promise settle
      await new Promise((r) => setTimeout(r, 0));
    });

    // Wait for fetch to be called with /v1/query
    await waitFor(() => {
      const calls = fetchMock.mock.calls.filter((args: unknown[]) =>
        (args[0] as string).includes("/v1/query"),
      );
      expect(calls.length).toBeGreaterThan(0);
    });

    // Verify the POST had correct content-type and body
    const queryCalls = fetchMock.mock.calls.filter((args: unknown[]) =>
      (args[0] as string).includes("/v1/query"),
    );
    const queryCallInit = queryCalls[0][1] as RequestInit & { headers: Headers; body: string };
    expect(queryCallInit.headers.get("content-type")).toBe("application/x-kql");
    expect(queryCallInit.body).toContain("events");

    // onResults should have been called with the fixture data
    await waitFor(() => {
      expect(onResults).toHaveBeenCalledWith(
        expect.objectContaining({
          columns: expect.arrayContaining([
            expect.objectContaining({ name: "timestamp" }),
          ]),
          rows: expect.arrayContaining([
            expect.objectContaining({ msg: "hello" }),
          ]),
        }),
      );
    });

    // Results section should be visible (RowFilter appears when hasResults is true)
    await waitFor(() => {
      // The RowFilter placeholder "Filter rows…" appears once results arrive
      const filterInput = document.querySelector('input[placeholder="Filter rows…"]');
      expect(filterInput).toBeTruthy();
    });
  });

  it("3a. showSchemaBrowser=false hides the browser panel", () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
        return makeNdjsonResponse([]);
      }),
    );

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <PensieveQueryEditor showSchemaBrowser={false} />
      </PensieveProvider>,
    );

    // The schema browser filter input is a good sentinel — it's always rendered
    // when the browser is visible (even before schema loads).
    const filterInput = document.querySelector('input[placeholder="Filter tables + columns…"]');
    expect(filterInput).toBeNull();
  });

  it("3b. readOnly=true hides the run button", () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
        return makeNdjsonResponse([]);
      }),
    );

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <PensieveQueryEditor readOnly />
      </PensieveProvider>,
    );

    expect(screen.queryByTestId("run-btn")).toBeNull();
  });

  it("5. surfaces a query error banner when /v1/query fails (no more silent failures)", async () => {
    const qc = makeQC();
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
      if (url.includes("/v1/query")) {
        return Promise.resolve(
          new Response(
            JSON.stringify({ error: { code: "sql_parse_error", message: "No field named timestamp" } }),
            { status: 400, headers: new Headers({ "content-type": "application/json" }) },
          ),
        );
      }
      return Promise.resolve(new Response("", { status: 404 }));
    });
    vi.stubGlobal("fetch", fetchMock);

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <PensieveQueryEditor defaultQuery="events | take 5" showTimeRange={false} database="testdb" />
      </PensieveProvider>,
    );

    await act(async () => {
      fireEvent.click(screen.getByTestId("run-btn"));
      await new Promise((r) => setTimeout(r, 0));
    });

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("Query failed");
    });
    expect(screen.getByRole("alert").textContent).toContain("No field named timestamp");
  });

  it("4. two instances: independent query text (type in one, other unchanged)", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/v1/catalog/schema")) return makeSchemaResponse();
        return makeNdjsonResponse([]);
      }),
    );

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
        <div>
          <div data-testid="editor-a">
            <PensieveQueryEditor defaultQuery="query-alpha" showSchemaBrowser={false} showTimeRange={false} />
          </div>
          <div data-testid="editor-b">
            <PensieveQueryEditor defaultQuery="query-beta" showSchemaBrowser={false} showTimeRange={false} />
          </div>
        </div>
      </PensieveProvider>,
    );

    const textareas = screen.getAllByTestId("monaco");
    expect(textareas).toHaveLength(2);

    // Each textarea should have its own initial query
    expect((textareas[0] as HTMLTextAreaElement).value).toBe("query-alpha");
    expect((textareas[1] as HTMLTextAreaElement).value).toBe("query-beta");

    // Type in the first editor
    fireEvent.change(textareas[0], { target: { value: "new-query-a" } });

    // Second should be unchanged
    expect((textareas[1] as HTMLTextAreaElement).value).toBe("query-beta");
    expect((textareas[0] as HTMLTextAreaElement).value).toBe("new-query-a");
  });
});
