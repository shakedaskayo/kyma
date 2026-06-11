import { beforeEach, expect, test, vi } from "vitest";
import {
  listDataSources,
  getDataSource,
  createDataSource,
  patchDataSource,
  deleteDataSource,
  pauseDataSource,
  resumeDataSource,
  triggerDataSource,
  listGitHubRepos,
  deriveStatus,
  type DataSourceDetail,
} from "./datasources";
import { createTransport } from "./transport";

const mockFetch = vi.fn();
beforeEach(() => {
  mockFetch.mockReset();
});

function makeTransport(fetchMock: typeof fetch, database?: string) {
  return createTransport({ endpoint: "http://localhost:7070", auth: { token: "tok" }, database, fetch: fetchMock });
}

function ok(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function empty(status: number) {
  return new Response(null, { status });
}

function detail(over: Partial<DataSourceDetail> = {}): DataSourceDetail {
  return {
    id: "c1",
    name: "gh",
    type: "github",
    target_database: "obs",
    target_table: "github_nodes",
    schedule_ms: 60_000,
    drive_model: "poll",
    enabled: true,
    disabled_reason: null,
    last_run_at: null,
    last_success_at: null,
    last_error: null,
    last_rows_ingested: null,
    config: {},
    ...over,
  };
}

// ── list ────────────────────────────────────────────────────────────────────────

test("listDataSources unwraps the .items envelope", async () => {
  mockFetch.mockResolvedValue(
    ok({ items: [{ id: "c1", name: "gh", type: "github", enabled: true }] }),
  );
  const items = await listDataSources(makeTransport(mockFetch));
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources");
  expect(init.headers.get("authorization")).toBe("Bearer tok");
  expect(items).toHaveLength(1);
  expect(items[0].id).toBe("c1");
});

test("listDataSources tolerates a missing items array", async () => {
  mockFetch.mockResolvedValue(ok({}));
  const items = await listDataSources(makeTransport(mockFetch));
  expect(items).toEqual([]);
});

// ── get ─────────────────────────────────────────────────────────────────────────

test("getDataSource GETs the detail endpoint", async () => {
  mockFetch.mockResolvedValue(ok(detail()));
  await getDataSource(makeTransport(mockFetch), { id: "c1" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources/c1");
});

// ── create ──────────────────────────────────────────────────────────────────────

test("createDataSource POSTs the create body and returns the id", async () => {
  mockFetch.mockResolvedValue(ok({ id: "new1" }, 201));
  const body = {
    name: "my-gh",
    type: "github",
    target_database: "obs",
    target_table: "github_nodes",
    schedule_ms: 300_000,
    config: { token: "pat", repos: ["owner/repo"] },
  };
  const res = await createDataSource(makeTransport(mockFetch), { body });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body)).toEqual(body);
  expect(res.id).toBe("new1");
});

// ── mutations (204 / 202 empty bodies) ───────────────────────────────────────────

test("patchDataSource PATCHes and accepts a 204 empty body", async () => {
  mockFetch.mockResolvedValue(empty(204));
  await expect(
    patchDataSource(makeTransport(mockFetch), { id: "c1", patch: { name: "renamed" } }),
  ).resolves.toBeUndefined();
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources/c1");
  expect(init.method).toBe("PATCH");
  expect(JSON.parse(init.body)).toEqual({ name: "renamed" });
});

test("deleteDataSource accepts a 204 empty body", async () => {
  mockFetch.mockResolvedValue(empty(204));
  await expect(deleteDataSource(makeTransport(mockFetch), { id: "c1" })).resolves.toBeUndefined();
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources/c1");
  expect(init.method).toBe("DELETE");
});

test("pauseDataSource / resumeDataSource POST to the right paths", async () => {
  mockFetch.mockResolvedValue(empty(204));
  await pauseDataSource(makeTransport(mockFetch), { id: "c1" });
  expect(mockFetch.mock.calls[0][0]).toBe("http://localhost:7070/v1/data-sources/c1/pause");
  await resumeDataSource(makeTransport(mockFetch), { id: "c1" });
  expect(mockFetch.mock.calls[1][0]).toBe("http://localhost:7070/v1/data-sources/c1/resume");
});

test("triggerDataSource accepts a 202 empty body", async () => {
  mockFetch.mockResolvedValue(empty(202));
  await expect(triggerDataSource(makeTransport(mockFetch), { id: "c1" })).resolves.toBeUndefined();
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources/c1/trigger");
  expect(init.method).toBe("POST");
});

// ── github repos ──────────────────────────────────────────────────────────────────

test("listGitHubRepos POSTs the PAT in the body and unwraps .repos", async () => {
  mockFetch.mockResolvedValue(
    ok({
      repos: [
        {
          full_name: "octocat/hello",
          name: "hello",
          owner: "octocat",
          private: false,
          default_branch: "main",
          description: null,
        },
      ],
    }),
  );
  const repos = await listGitHubRepos(makeTransport(mockFetch), { pat: "ghp_secret" });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/data-sources/github/repos");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body)).toEqual({ token: "ghp_secret" });
  expect(init.headers.get("authorization")).toBe("Bearer tok");
  expect(repos).toHaveLength(1);
  expect(repos[0].full_name).toBe("octocat/hello");
});

// ── auth / errors ─────────────────────────────────────────────────────────────────

test("throws on 401", async () => {
  mockFetch.mockResolvedValue(new Response("no", { status: 401 }));
  await expect(listDataSources(makeTransport(mockFetch))).rejects.toThrow(/unauthorized|token rejected/i);
});

test("handleEmpty throws on a 500", async () => {
  mockFetch.mockResolvedValue(new Response("boom", { status: 500 }));
  await expect(triggerDataSource(makeTransport(mockFetch), { id: "c1" })).rejects.toThrow(/request failed: 500/);
});

// ── x-database header ─────────────────────────────────────────────────────────────

test("sends x-database header when database is set", async () => {
  mockFetch.mockResolvedValue(ok({ items: [] }));
  await listDataSources(makeTransport(mockFetch, "obs"));
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers.get("x-database")).toBe("obs");
});

test("omits x-database header when database is absent", async () => {
  mockFetch.mockResolvedValue(ok({ items: [] }));
  await listDataSources(makeTransport(mockFetch));
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers.get("x-database")).toBeNull();
});

// ── deriveStatus mapping ───────────────────────────────────────────────────────────

test("deriveStatus: disabled wins over everything", () => {
  expect(deriveStatus(detail({ enabled: false, last_error: "x", last_success_at: "t" }))).toBe(
    "disabled",
  );
});

test("deriveStatus: error when last_error present", () => {
  expect(deriveStatus(detail({ last_error: "boom", last_success_at: "t" }))).toBe("error");
});

test("deriveStatus: synced when a success exists and no error", () => {
  expect(deriveStatus(detail({ last_run_at: "t", last_success_at: "t" }))).toBe("synced");
});

test("deriveStatus: syncing when a run started but never succeeded", () => {
  expect(deriveStatus(detail({ last_run_at: "t", last_success_at: null }))).toBe("syncing");
});

test("deriveStatus: idle when never run", () => {
  expect(deriveStatus(detail())).toBe("idle");
});
