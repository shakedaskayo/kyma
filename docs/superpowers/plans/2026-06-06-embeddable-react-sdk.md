# Embeddable React SDK Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `@kyma-ai/client` + `@kyma-ai/react` npm packages so external apps can embed Kyma's five views (Graph, Discover, Query, Dashboards, Agent) against any Kyma server URL, with OIDC auth, theme/props/headless customization, an example app, Storybook, docs, and a publish pipeline — while the `web/` app dogfoods the packages.

**Architecture:** Approach B from the spec (`docs/superpowers/specs/2026-06-06-embeddable-react-sdk-design.md`): extract `web/src/sdk/` into a framework-agnostic client package with an instance-scoped transport (no global fetch patching); build a React package (provider + headless hooks + components) with `ky-`-prefixed Tailwind scoped under `.kyma-root`; add `OidcAuthBackend`, per-database scoping, and CORS allow-list to the Rust server.

**Tech Stack:** pnpm workspaces, Vite library mode + vite-plugin-dts, Tailwind 3 (prefix `ky-`), React Query, Zustand (per-instance stores via context), vitest + Testing Library + msw-free mock fetch, Rust (axum, `jsonwebtoken`), Storybook 8, Changesets.

**Conventions for every task:**
- Work on branch `feat/embed-react-sdk`.
- TDD where the task creates logic (test first, watch it fail, implement, watch it pass). Pure file-moves are verified by typecheck + existing tests passing from their new location.
- Commit after every task with the message given in the task.
- Run commands from the repo root unless stated.
- `pnpm` for everything JS; `cargo` for Rust.

---

## Phase 0 — Workspace scaffolding

### Task 0.1: Add packages/examples to the pnpm workspace

**Files:**
- Modify: `pnpm-workspace.yaml`
- Modify: `package.json` (repo root)

- [x] **Step 1: Extend workspace globs**

`pnpm-workspace.yaml` becomes:

```yaml
packages:
  - "web"
  - "web/src-tauri"
  - "docs/site"
  - "packages/*"
  - "examples/*"
allowBuilds:
  '@bufbuild/buf': true
  esbuild: true
```

- [x] **Step 2: Add root convenience scripts**

In the root `package.json`, merge into `"scripts"` (create the block if missing — read the file first):

```json
{
  "scripts": {
    "build:packages": "pnpm --filter @kyma-ai/client build && pnpm --filter @kyma-ai/react build",
    "test:packages": "pnpm --filter @kyma-ai/client test && pnpm --filter @kyma-ai/react test",
    "typecheck:all": "pnpm -r --filter './packages/*' --filter './web' --filter './examples/*' typecheck"
  }
}
```

- [x] **Step 3: Verify workspace resolves**

Run: `pnpm install`
Expected: exits 0; no new packages yet, no errors.

- [x] **Step 4: Commit**

```bash
git add pnpm-workspace.yaml package.json
git commit -m "chore: add packages/ and examples/ to pnpm workspace"
```

### Task 0.2: Scaffold @kyma-ai/client package

**Files:**
- Create: `packages/client/package.json`
- Create: `packages/client/tsconfig.json`
- Create: `packages/client/vite.config.ts`
- Create: `packages/client/src/index.ts` (placeholder export, replaced in Phase 1)

- [x] **Step 1: package.json**

```json
{
  "name": "@kyma-ai/client",
  "version": "0.1.0",
  "description": "Framework-agnostic TypeScript client for the Kyma context engine API",
  "license": "Apache-2.0",
  "type": "module",
  "main": "./dist/index.js",
  "module": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" }
  },
  "files": ["dist"],
  "sideEffects": false,
  "scripts": {
    "build": "vite build",
    "dev": "vite build --watch",
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "apache-arrow": "17.0.0"
  },
  "devDependencies": {
    "typescript": "5.5.4",
    "vite": "5.4.0",
    "vite-plugin-dts": "^4.3.0",
    "vitest": "2.0.5"
  }
}
```

(`apache-arrow` is required because `sdk/arrow.ts`/`sdk/query.ts` decode Arrow IPC results.)

- [x] **Step 2: tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "strict": true,
    "skipLibCheck": true,
    "declaration": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "outDir": "dist"
  },
  "include": ["src"]
}
```

- [x] **Step 3: vite.config.ts**

```ts
import { defineConfig } from "vite";
import dts from "vite-plugin-dts";

export default defineConfig({
  plugins: [dts({ rollupTypes: true })],
  build: {
    lib: { entry: "src/index.ts", formats: ["es"], fileName: "index" },
    sourcemap: true,
    rollupOptions: { external: ["apache-arrow"] },
  },
});
```

- [x] **Step 4: placeholder src/index.ts**

```ts
export const KYMA_CLIENT_VERSION = "0.1.0";
```

- [x] **Step 5: Verify build**

Run: `pnpm install && pnpm --filter @kyma-ai/client build && pnpm --filter @kyma-ai/client typecheck`
Expected: `dist/index.js` + `dist/index.d.ts` produced.

- [x] **Step 6: Commit**

```bash
git add packages/client pnpm-lock.yaml
git commit -m "chore(client): scaffold @kyma-ai/client package"
```

### Task 0.3: Scaffold @kyma-ai/react package

**Files:**
- Create: `packages/react/package.json`
- Create: `packages/react/tsconfig.json`
- Create: `packages/react/vite.config.ts`
- Create: `packages/react/tailwind.config.ts`
- Create: `packages/react/postcss.config.js`
- Create: `packages/react/src/index.ts` (placeholder)
- Create: `packages/react/src/styles/base.css`

- [x] **Step 1: package.json**

```json
{
  "name": "@kyma-ai/react",
  "version": "0.1.0",
  "description": "Embeddable React components for the Kyma context engine — Graph, Discover, Query, Dashboards, Agent",
  "license": "Apache-2.0",
  "type": "module",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./graph": { "types": "./dist/graph.d.ts", "import": "./dist/graph.js" },
    "./query": { "types": "./dist/query.d.ts", "import": "./dist/query.js" },
    "./discover": { "types": "./dist/discover.d.ts", "import": "./dist/discover.js" },
    "./dashboards": { "types": "./dist/dashboards.d.ts", "import": "./dist/dashboards.js" },
    "./agent": { "types": "./dist/agent.d.ts", "import": "./dist/agent.js" },
    "./styles.css": "./dist/style.css"
  },
  "files": ["dist"],
  "sideEffects": ["**/*.css"],
  "scripts": {
    "build": "vite build && pnpm run build:css",
    "build:css": "tailwindcss -i src/styles/base.css -o dist/style.css --minify",
    "dev": "vite build --watch",
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc --noEmit",
    "storybook": "storybook dev -p 6006",
    "build-storybook": "storybook build"
  },
  "peerDependencies": {
    "react": ">=18",
    "react-dom": ">=18"
  },
  "dependencies": {
    "@kyma-ai/client": "workspace:*",
    "@monaco-editor/react": "4.6.0",
    "@radix-ui/react-collapsible": "^1.1.12",
    "@radix-ui/react-dialog": "^1.1.15",
    "@radix-ui/react-dropdown-menu": "^2.1.16",
    "@radix-ui/react-label": "^2.1.8",
    "@radix-ui/react-scroll-area": "^1.2.10",
    "@radix-ui/react-slot": "^1.2.4",
    "@radix-ui/react-tooltip": "^1.2.8",
    "@tanstack/react-query": "^5.99.2",
    "@tanstack/react-table": "^8.21.3",
    "@tanstack/react-virtual": "^3.13.24",
    "apache-arrow": "17.0.0",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "echarts": "5.5.0",
    "echarts-for-react": "3.0.2",
    "lucide-react": "^1.8.0",
    "react-force-graph-2d": "^1.29.1",
    "react-grid-layout": "1.4.4",
    "tailwind-merge": "^3.5.0",
    "zustand": "^5.0.12"
  },
  "devDependencies": {
    "@testing-library/react": "^16.3.2",
    "@types/react": "18.3.3",
    "@types/react-dom": "18.3.0",
    "@types/react-grid-layout": "1.3.5",
    "@vitejs/plugin-react": "4.3.1",
    "autoprefixer": "10.4.20",
    "jsdom": "^29.0.2",
    "postcss": "8.4.41",
    "react": "18.3.1",
    "react-dom": "18.3.1",
    "tailwindcss": "3.4.10",
    "tailwindcss-animate": "^1.0.7",
    "typescript": "5.5.4",
    "vite": "5.4.0",
    "vite-plugin-dts": "^4.3.0",
    "vitest": "2.0.5"
  }
}
```

Notes: Monaco loads lazily inside `@monaco-editor/react`. The agent view's AI-SDK deps (`ai`, `@ai-sdk/react`, `streamdown`) get added in Task 5.5 when the agent component moves (keeps each task's diff reviewable).

- [x] **Step 2: tsconfig.json** — same as client's, plus `"jsx": "react-jsx"` and `"types": ["vitest/globals"]` if web's config uses globals (mirror `web/tsconfig.json` compiler options; read it first and copy strictness flags).

- [x] **Step 3: vite.config.ts (multi-entry library)**

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import dts from "vite-plugin-dts";

export default defineConfig({
  plugins: [react(), dts({ rollupTypes: false })],
  build: {
    lib: {
      entry: {
        index: "src/index.ts",
        graph: "src/graph/index.ts",
        query: "src/query/index.ts",
        discover: "src/discover/index.ts",
        dashboards: "src/dashboards/index.ts",
        agent: "src/agent/index.ts",
      },
      formats: ["es"],
    },
    sourcemap: true,
    rollupOptions: {
      external: (id) =>
        !id.startsWith(".") && !id.startsWith("/") && !id.includes("?"),
    },
  },
  test: { environment: "jsdom" },
});
```

(`external` keeps **all** bare imports out of the bundle — every dependency stays a real npm dep; only package source is bundled. Until Phase 5 fills them in, create each `src/<view>/index.ts` as `export {};` placeholders.)

- [x] **Step 4: tailwind.config.ts — the isolation contract**

```ts
import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

// Embeddable build: every utility is prefixed (`ky-flex`) and every rule is
// scoped under `.kyma-root` so host-app CSS and Kyma CSS cannot collide in
// either direction. Tokens are `--kyma-*`, set inline by <KymaProvider>.
export default {
  prefix: "ky-",
  important: ".kyma-root",
  corePlugins: { preflight: false },
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--kyma-border))",
        "border-strong": "hsl(var(--kyma-border-strong))",
        input: "hsl(var(--kyma-input))",
        ring: "hsl(var(--kyma-ring))",
        background: "hsl(var(--kyma-background))",
        surface: "hsl(var(--kyma-surface))",
        foreground: "hsl(var(--kyma-foreground))",
        primary:   { DEFAULT: "hsl(var(--kyma-primary))",   foreground: "hsl(var(--kyma-primary-foreground))" },
        secondary: { DEFAULT: "hsl(var(--kyma-secondary))", foreground: "hsl(var(--kyma-secondary-foreground))" },
        destructive: { DEFAULT: "hsl(var(--kyma-destructive))", foreground: "hsl(var(--kyma-destructive-foreground))" },
        muted:     { DEFAULT: "hsl(var(--kyma-muted))",     foreground: "hsl(var(--kyma-muted-foreground))" },
        accent:    { DEFAULT: "hsl(var(--kyma-accent))",    foreground: "hsl(var(--kyma-accent-foreground))" },
        card:      { DEFAULT: "hsl(var(--kyma-card))",      foreground: "hsl(var(--kyma-card-foreground))" },
        popover:   { DEFAULT: "hsl(var(--kyma-popover))",   foreground: "hsl(var(--kyma-popover-foreground))" },
        brand:     { from: "hsl(var(--kyma-brand-from))",   to: "hsl(var(--kyma-brand-to))" },
      },
      borderRadius: {
        xl: "calc(var(--kyma-radius) + 4px)",
        lg: "var(--kyma-radius)",
        md: "calc(var(--kyma-radius) - 2px)",
        sm: "calc(var(--kyma-radius) - 4px)",
      },
      fontFamily: {
        sans: ["var(--kyma-font-sans)"],
        mono: ["var(--kyma-font-mono)"],
      },
      // Copy fontSize, boxShadow, backgroundImage, keyframes, animation blocks
      // VERBATIM from web/tailwind.config.ts (they contain no var() references
      // except boxShadow/glow + backgroundImage which must switch to --kyma-*).
    },
  },
  plugins: [animate],
} satisfies Config;
```

- [x] **Step 5: src/styles/base.css**

```css
@tailwind components;
@tailwind utilities;
/* No @tailwind base — preflight is off; the host app owns global resets. */
```

- [x] **Step 6: postcss.config.js** — copy `web/postcss.config.js` verbatim.

- [x] **Step 7: Verify**

Run: `pnpm install && pnpm --filter @kyma-ai/react build && pnpm --filter @kyma-ai/react typecheck`
Expected: dist artifacts for all entries + `dist/style.css`.

- [x] **Step 8: Commit**

```bash
git add packages/react pnpm-lock.yaml
git commit -m "chore(react): scaffold @kyma-ai/react package with ky- prefixed scoped tailwind"
```

### Task 0.4: Changesets

**Files:**
- Create: `.changeset/config.json` (via `pnpm dlx @changesets/cli init`, then edit)

- [x] **Step 1:** Run `pnpm add -Dw @changesets/cli && pnpm changeset init`
- [x] **Step 2:** Edit `.changeset/config.json`: set `"access": "public"`, and ignore non-published workspaces:

```json
{
  "$schema": "https://unpkg.com/@changesets/config@3.0.0/schema.json",
  "changelog": "@changesets/cli/changelog",
  "commit": false,
  "access": "public",
  "baseBranch": "main",
  "ignore": ["kyma-web", "kyma-docs", "kyma-embed-demo"]
}
```

(Check the actual `name` fields of `web/package.json` and `docs/site/package.json` and use those; `kyma-embed-demo` is created in Phase 7 — add it to ignore now anyway.)

- [x] **Step 3: Commit**

```bash
git add .changeset package.json pnpm-lock.yaml
git commit -m "chore: add changesets for package versioning"
```

---

## Phase 1 — @kyma-ai/client

### Task 1.1: Typed errors

**Files:**
- Create: `packages/client/src/errors.ts`
- Test: `packages/client/src/errors.test.ts`

- [x] **Step 1: Write failing test**

```ts
import { describe, expect, it } from "vitest";
import { KymaApiError, KymaAuthError, errorFromResponse } from "./errors";

describe("errorFromResponse", () => {
  it("maps 4xx/5xx with JSON body to KymaApiError", async () => {
    const res = new Response(JSON.stringify({ error: "bad query" }), {
      status: 400,
      headers: { "content-type": "application/json", "x-request-id": "r-1" },
    });
    const err = await errorFromResponse(res);
    expect(err).toBeInstanceOf(KymaApiError);
    expect(err.status).toBe(400);
    expect(err.message).toContain("bad query");
    expect(err.requestId).toBe("r-1");
  });

  it("maps 401/403 to KymaAuthError (subtype of KymaApiError)", async () => {
    const err = await errorFromResponse(new Response("nope", { status: 401 }));
    expect(err).toBeInstanceOf(KymaAuthError);
    expect(err).toBeInstanceOf(KymaApiError);
  });

  it("truncates long text bodies", async () => {
    const err = await errorFromResponse(new Response("x".repeat(500), { status: 500 }));
    expect(err.message.length).toBeLessThan(300);
  });
});
```

- [x] **Step 2:** Run `pnpm --filter @kyma-ai/client test` — FAIL (module missing).

- [x] **Step 3: Implement**

```ts
/** Error from a Kyma API response. `status` is the HTTP status. */
export class KymaApiError extends Error {
  readonly status: number;
  readonly requestId?: string;
  constructor(status: number, message: string, requestId?: string) {
    super(message);
    this.name = "KymaApiError";
    this.status = status;
    this.requestId = requestId;
  }
}

/** 401/403 — token invalid, expired beyond refresh, or insufficient scope. */
export class KymaAuthError extends KymaApiError {
  constructor(status: number, message: string, requestId?: string) {
    super(status, message, requestId);
    this.name = "KymaAuthError";
  }
}

export async function errorFromResponse(res: Response): Promise<KymaApiError> {
  const requestId = res.headers.get("x-request-id") ?? undefined;
  let detail = "";
  try {
    const text = await res.text();
    if ((res.headers.get("content-type") ?? "").includes("application/json")) {
      const body = JSON.parse(text) as { error?: string; message?: string };
      detail = body.error ?? body.message ?? text;
    } else {
      detail = text;
    }
  } catch {
    /* body unreadable — fall through to status-only message */
  }
  detail = detail.slice(0, 200);
  const message = `kyma request failed: ${res.status}${detail ? ` — ${detail}` : ""}`;
  return res.status === 401 || res.status === 403
    ? new KymaAuthError(res.status, message, requestId)
    : new KymaApiError(res.status, message, requestId);
}
```

- [x] **Step 4:** Run tests — PASS.
- [x] **Step 5: Commit** `git add packages/client/src && git commit -m "feat(client): typed KymaApiError/KymaAuthError"`

### Task 1.2: Instance-scoped transport (replaces global installAuthFetch)

**Files:**
- Create: `packages/client/src/transport.ts`
- Test: `packages/client/src/transport.test.ts`
- Reference: `web/src/sdk/auth-fetch.ts` (behavior being ported: single-flight refresh, one retry, HTML-fallback-as-404)

- [x] **Step 1: Write failing tests** — the complete behavioral contract:

```ts
import { describe, expect, it, vi } from "vitest";
import { createTransport } from "./transport";
import { KymaAuthError } from "./errors";

const ok = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });

describe("createTransport", () => {
  it("resolves paths against endpoint and attaches bearer + x-database", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({
      endpoint: "https://kyma.example.com/",
      auth: { token: "tok-1" },
      database: "prod",
      fetch: fetchMock,
    });
    await t.request("/v1/graph");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://kyma.example.com/v1/graph");
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer tok-1");
    expect(new Headers(init.headers).get("x-database")).toBe("prod");
  });

  it("per-request database overrides the default", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { token: "t" }, database: "prod", fetch: fetchMock });
    await t.request("/v1/query", { database: "staging" });
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("x-database")).toBe("staging");
  });

  it("on 401 with getToken: re-mints once (single-flight) and retries", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response("expired", { status: 401 }))
      .mockResolvedValueOnce(new Response("expired", { status: 401 }))
      .mockResolvedValue(ok({ fine: true }));
    const getToken = vi.fn().mockResolvedValueOnce("t1").mockResolvedValueOnce("t1").mockResolvedValue("t2");
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });
    // two concurrent requests both hit 401 — refresh must dedupe
    const [r1, r2] = await Promise.all([t.request("/v1/a"), t.request("/v1/b")]);
    expect(r1.status).toBe(200);
    expect(r2.status).toBe(200);
    // initial mint (deduped) + ONE expired re-mint (deduped) = 2 calls
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken).toHaveBeenLastCalledWith({ reason: "expired" });
  });

  it("throws KymaAuthError when retry after re-mint still 401s", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({
      endpoint: "https://k",
      auth: { getToken: async () => "always-bad" },
      fetch: fetchMock,
    });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(KymaAuthError);
  });

  it("throws KymaAuthError immediately on 401 with static token (no re-mint possible)", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(KymaAuthError);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("converts HTML 200 from /v1/* into a 404 (legacy SPA fallback)", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("<!doctype html>", { status: 200, headers: { "content-type": "text/html" } }),
    );
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const res = await t.request("/v1/anything");
    expect(res.status).toBe(404);
  });

  it("does not throw for non-auth errors — returns the response for callers to map", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("boom", { status: 500 }));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const res = await t.request("/v1/a");
    expect(res.status).toBe(500);
  });

  it("propagates AbortSignal", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const ac = new AbortController();
    await t.request("/v1/a", { signal: ac.signal });
    expect(fetchMock.mock.calls[0][1].signal).toBe(ac.signal);
  });
});
```

- [x] **Step 2:** Run — FAIL.

- [x] **Step 3: Implement**

```ts
import { KymaAuthError } from "./errors";

export type GetToken = (opts?: { reason: "initial" | "expired" }) => Promise<string>;
export type KymaAuth = { token: string } | { getToken: GetToken };

export interface TransportConfig {
  /** Kyma server URL, e.g. https://kyma.acme.internal */
  endpoint: string;
  auth: KymaAuth;
  /** Default database; per-request override via RequestOpts.database. */
  database?: string;
  /** Injectable for tests / non-browser runtimes. Defaults to globalThis.fetch. */
  fetch?: typeof fetch;
}

export interface RequestOpts extends Omit<RequestInit, "headers"> {
  headers?: HeadersInit;
  database?: string;
}

export interface KymaTransport {
  readonly endpoint: string;
  readonly database?: string;
  /**
   * Authenticated fetch. Resolves `path` against `endpoint`, attaches
   * `Authorization: Bearer` + `x-database`. On 401 with `getToken` auth,
   * re-mints once (single-flight across concurrent requests) and retries
   * once; throws KymaAuthError if auth ultimately fails. Non-auth statuses
   * are returned as-is for the caller to map.
   */
  request(path: string, opts?: RequestOpts): Promise<Response>;
}

export function createTransport(cfg: TransportConfig): KymaTransport {
  const base = cfg.endpoint.replace(/\/$/, "");
  const fetchImpl = cfg.fetch ?? globalThis.fetch.bind(globalThis);

  let cachedToken: string | null = "token" in cfg.auth ? cfg.auth.token : null;
  let mintInFlight: Promise<string> | null = null;

  function mint(reason: "initial" | "expired"): Promise<string> {
    if (!("getToken" in cfg.auth)) return Promise.resolve(cachedToken ?? "");
    if (mintInFlight) return mintInFlight;
    mintInFlight = cfg.auth
      .getToken({ reason })
      .then((tok) => {
        cachedToken = tok;
        return tok;
      })
      .finally(() => {
        mintInFlight = null;
      });
    return mintInFlight;
  }

  async function token(): Promise<string> {
    if (cachedToken) return cachedToken;
    return mint("initial");
  }

  async function doFetch(path: string, opts: RequestOpts, tok: string): Promise<Response> {
    const url = path.startsWith("http") ? path : `${base}${path.startsWith("/") ? path : `/${path}`}`;
    const headers = new Headers(opts.headers);
    if (tok && !headers.has("authorization")) headers.set("authorization", `Bearer ${tok}`);
    const db = opts.database ?? cfg.database;
    if (db && !headers.has("x-database")) headers.set("x-database", db);
    const { database: _db, ...init } = opts;
    const res = await fetchImpl(url, { ...init, headers });
    // Legacy servers (≤0.0.2) served the SPA for unknown /v1/* paths.
    if (res.ok && url.includes("/v1/") && (res.headers.get("content-type") ?? "").includes("text/html")) {
      return new Response(JSON.stringify({ error: "endpoint not available on this server" }), {
        status: 404,
        headers: { "content-type": "application/json" },
      });
    }
    return res;
  }

  return {
    endpoint: base,
    database: cfg.database,
    async request(path, opts = {}) {
      const res = await doFetch(path, opts, await token());
      if (res.status !== 401) return res;
      if (!("getToken" in cfg.auth)) {
        throw new KymaAuthError(401, "kyma request failed: 401 — token rejected");
      }
      const fresh = await mint("expired");
      const retry = await doFetch(path, opts, fresh);
      if (retry.status === 401) {
        throw new KymaAuthError(401, "kyma request failed: 401 — token rejected after refresh");
      }
      return retry;
    },
  };
}
```

- [x] **Step 4:** Run tests — PASS. (The single-flight test asserts 2 mints: one deduped `initial`, one deduped `expired`.)
- [x] **Step 5: Commit** `git commit -am "feat(client): instance-scoped transport with single-flight token re-mint"`

### Task 1.3: Move SDK modules into the package (transport-first signatures)

**Files (move from `web/src/sdk/` → `packages/client/src/`):**
`graph.ts`, `query.ts`, `discover.ts`, `discover-live.ts`, `dashboards.ts`, `catalog.ts`, `capabilities.ts`, `agent-engine.ts`, `agent-skills.ts`, `memory.ts`, `connectors.ts`, `credentials.ts`, `setup.ts`, `auth.ts`, `oauth.ts`, `arrow.ts`, `graph-layout.ts`, `reconnect.ts` — **plus their `.test.ts` files**.
**Stays in `web/src/sdk/`:** `session.ts`, `session.test.ts`, `auth-fetch.ts` (deleted in Phase 6).

The transformation, applied uniformly to every moved module:

1. `git mv web/src/sdk/<file> packages/client/src/<file>` (preserves history).
2. Functions taking `BaseArgs & {...}` become `(t: KymaTransport, args: {...})`:

```ts
// BEFORE (graph.ts)
export async function listGraphs(args: BaseArgs): Promise<GraphRef[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph`, { headers: headers(args.token, args.database) });
  return handleResponse<GraphRef[]>(res);
}

// AFTER
import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function listGraphs(t: KymaTransport): Promise<GraphRef[]> {
  return handleResponse<GraphRef[]>(await t.request("/v1/graph"));
}

export async function getOverview(
  t: KymaTransport,
  args: { graph: string; realm?: string; limit?: number },
): Promise<GraphPayload> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  if (args.limit != null) q.set("limit", String(args.limit));
  const qs = q.toString();
  return handleResponse(await t.request(`/v1/graph/${args.graph}/overview${qs ? `?${qs}` : ""}`));
}
```

3. Local `headers()` / `base()` helpers are deleted (transport owns them). `content-type: application/json` moves to the call sites that POST: `t.request(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(...) })`.
4. Streaming modules (`discover.ts` async generators, `discover-live.ts`, `agent-engine.ts` SSE) keep their parsing logic; only the initial `fetch` call becomes `t.request(...)`, and `AbortSignal` keeps flowing through `RequestOpts.signal`.
5. Tests: update to build a transport with a mock fetch —

```ts
import { createTransport } from "./transport";
const t = (fetchMock: typeof fetch) =>
  createTransport({ endpoint: "https://kyma.test", auth: { token: "tok" }, fetch: fetchMock });
```

then change call sites from `listGraphs({ endpoint, token })` to `listGraphs(t(mock))`. Assertions about URLs/headers stay valid (transport reproduces them).

- [x] **Step 1:** Move + transform `graph.ts` + `graph.test.ts` first; run `pnpm --filter @kyma-ai/client test` until green.
- [x] **Step 2:** Repeat for `query.ts` + `arrow.ts` (query decodes Arrow via arrow.ts — they move together), then `catalog.ts`, `capabilities.ts`, `dashboards.ts`.
- [x] **Step 3:** Repeat for streaming modules: `discover.ts`, `discover-live.ts`, `reconnect.ts`, `agent-engine.ts`, `agent-skills.ts`, `memory.ts`.
- [x] **Step 4:** Repeat for `connectors.ts`, `credentials.ts`, `setup.ts`, `auth.ts`, `oauth.ts`. (`auth.ts` login/refresh take explicit `{endpoint}` and run unauthenticated — they accept a plain `endpoint: string` arg, NOT a transport; they're used to *obtain* tokens.)
- [x] **Step 5:** `graph-layout.ts` is pure computation — moves unchanged with its test.
- [x] **Step 6:** Run full suite: `pnpm --filter @kyma-ai/client test && pnpm --filter @kyma-ai/client typecheck` — PASS. (web/ is now broken — expected; it's fixed by the shim in Task 1.4.)
- [x] **Step 7: Commit** `git commit -am "feat(client): move sdk modules to @kyma-ai/client with transport-first signatures"`

### Task 1.4: createKymaClient factory + web/ compatibility

**Files:**
- Create: `packages/client/src/client.ts`
- Modify: `packages/client/src/index.ts` (real exports)
- Test: `packages/client/src/client.test.ts`
- Modify: `web/package.json` (add `"@kyma-ai/client": "workspace:*"`)
- Modify: `web/src/sdk/*` call sites (mechanical re-import)

- [x] **Step 1: Failing test**

```ts
import { describe, expect, it, vi } from "vitest";
import { createKymaClient } from "./client";

describe("createKymaClient", () => {
  it("exposes namespaced methods bound to one transport", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify([]), { status: 200, headers: { "content-type": "application/json" } }),
    );
    const client = createKymaClient({
      endpoint: "https://kyma.test",
      auth: { token: "tok" },
      fetch: fetchMock,
    });
    await client.graph.listGraphs();
    expect(fetchMock.mock.calls[0][0]).toBe("https://kyma.test/v1/graph");
  });

  it("withDatabase returns a scoped view sharing auth state", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("[]", { status: 200 }));
    const client = createKymaClient({ endpoint: "https://k", auth: { token: "t" }, fetch: fetchMock });
    await client.withDatabase("staging").graph.listGraphs();
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("x-database")).toBe("staging");
  });
});
```

- [x] **Step 2:** Run — FAIL.

- [x] **Step 3: Implement** `client.ts`: for each module, bind its functions to the transport:

```ts
import { createTransport, type KymaTransport, type TransportConfig } from "./transport";
import * as graph from "./graph";
import * as query from "./query";
import * as discover from "./discover";
import * as discoverLive from "./discover-live";
import * as dashboards from "./dashboards";
import * as catalog from "./catalog";
import * as capabilities from "./capabilities";
import * as agentEngine from "./agent-engine";
import * as agentSkills from "./agent-skills";
import * as memory from "./memory";
import * as connectors from "./connectors";
import * as credentials from "./credentials";

/** Binds `(t, args)` module functions to a fixed transport: `fn(args)`. */
type Bound<M> = {
  [K in keyof M]: M[K] extends (t: KymaTransport, ...a: infer A) => infer R ? (...a: A) => R : never;
};

function bind<M extends Record<string, unknown>>(t: KymaTransport, mod: M): Bound<M> {
  const out = {} as Record<string, unknown>;
  for (const [k, v] of Object.entries(mod)) {
    if (typeof v === "function") out[k] = (...a: unknown[]) => (v as Function)(t, ...a);
  }
  return out as Bound<M>;
}

export interface KymaClient {
  transport: KymaTransport;
  graph: Bound<typeof graph>;
  query: Bound<typeof query>;
  discover: Bound<typeof discover> & Bound<typeof discoverLive>;
  dashboards: Bound<typeof dashboards>;
  catalog: Bound<typeof catalog>;
  capabilities: Bound<typeof capabilities>;
  agent: Bound<typeof agentEngine> & Bound<typeof agentSkills>;
  memory: Bound<typeof memory>;
  connectors: Bound<typeof connectors>;
  credentials: Bound<typeof credentials>;
  /** Same client, different default database (shares token cache). */
  withDatabase(database: string): KymaClient;
}

export type KymaClientConfig = TransportConfig;

export function createKymaClient(cfg: KymaClientConfig): KymaClient {
  const t = createTransport(cfg);
  return fromTransport(t, cfg);
}

function fromTransport(t: KymaTransport, cfg: KymaClientConfig): KymaClient {
  return {
    transport: t,
    graph: bind(t, graph),
    query: bind(t, query),
    discover: { ...bind(t, discover), ...bind(t, discoverLive) },
    dashboards: bind(t, dashboards),
    catalog: bind(t, catalog),
    capabilities: bind(t, capabilities),
    agent: { ...bind(t, agentEngine), ...bind(t, agentSkills) },
    memory: bind(t, memory),
    connectors: bind(t, connectors),
    credentials: bind(t, credentials),
    withDatabase(database: string): KymaClient {
      // Wrap rather than re-create: share the underlying token cache.
      const scoped: KymaTransport = {
        ...t,
        database,
        request: (path, opts = {}) => t.request(path, { database, ...opts }),
      };
      return fromTransport(scoped, cfg);
    },
  };
}
```

Caveat for the executor: `withDatabase` must apply `database` as a *default* (`{ database, ...opts }` — caller's explicit `opts.database` wins). Note the spread order shown is correct for that.

- [x] **Step 4:** `index.ts` — export everything public: `createKymaClient`, `KymaClient`, types from every module (`export * from "./graph"` etc.), `createTransport`, errors, auth/setup functions.
- [x] **Step 5:** Tests pass; build passes.
- [x] **Step 6: Re-point web/ imports.** Add dep to `web/package.json`: `"@kyma-ai/client": "workspace:*"`. Then mechanically update **every** `from "@/sdk/<moved-module>"`/`from "../sdk/<moved-module>"` import in `web/src` to `from "@kyma-ai/client"`. Call sites still pass `{endpoint, token, database}` objects — they now need a transport. **Interim shim** (deleted in Phase 6): create `web/src/sdk/compat.ts`:

```ts
// Transitional: builds a one-shot transport from the session args shape the
// app still passes around. Phase 6 replaces this with a session-level client.
import { createTransport, type KymaTransport } from "@kyma-ai/client";

export function transportFor(args: { endpoint: string; token: string; database?: string }): KymaTransport {
  return createTransport({ endpoint: args.endpoint, auth: { token: args.token }, database: args.database });
}
```

and update web call sites from `listGraphs({ endpoint, token, database })` to `listGraphs(transportFor({ endpoint, token, database }))`. This is mechanical; do it file-by-file guided by `pnpm --filter kyma-web typecheck`.
- [x] **Step 7:** `pnpm --filter kyma-web typecheck && pnpm --filter kyma-web test && pnpm --filter kyma-web build` — all PASS.
- [x] **Step 8: Commit** `git commit -am "feat(client): createKymaClient factory; web consumes @kyma-ai/client"`

---

## Phase 2 — Server: OIDC, per-database scoping, CORS

### Task 2.1: `Principal.allowed_databases`

**Files:**
- Modify: `crates/kyma-server/src/auth/backend.rs:33-38`
- Modify: every `Principal { ... }` construction site (`rg "Principal \{" crates/` — middleware.rs, env_backend.rs, session_backend.rs, db_backend.rs, tests)

- [x] **Step 1:** Add the field:

```rust
#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant: TenantId,
    pub role: Role,
    pub subject: Option<String>,
    /// Databases this principal may touch. `None` = unrestricted (the default
    /// for all pre-existing backends; only OIDC claims populate this).
    pub allowed_databases: Option<Vec<String>>,
}
```

- [x] **Step 2:** Fix every construction site with `allowed_databases: None`. Run `cargo build -p kyma-server` until clean; `cargo test -p kyma-server --lib`.
- [x] **Step 3: Commit** `git commit -am "feat(server): Principal.allowed_databases (None = unrestricted)"`

### Task 2.2: OidcAuthBackend

**Files:**
- Modify: root `Cargo.toml` (workspace deps): add `jsonwebtoken = "9"`
- Modify: `crates/kyma-server/Cargo.toml`: add `jsonwebtoken.workspace = true`
- Create: `crates/kyma-server/src/auth/oidc_backend.rs`
- Modify: `crates/kyma-server/src/auth/mod.rs` (add `pub mod oidc_backend;`)
- Test: unit tests inline in `oidc_backend.rs` + integration test Task 2.5

- [x] **Step 1: Write the failing unit tests first** (inline `#[cfg(test)]`, using a locally-generated RSA key via `jsonwebtoken`):

Test list (write all, they fail to compile until implementation exists):
- `validates_rs256_jwt_and_maps_claims` — token with `kyma_role: "write"`, `kyma_databases: ["prod"]`, correct iss/aud → Principal{role: Write, allowed_databases: Some(["prod"]), subject: Some(sub)}
- `missing_role_claim_defaults_to_read`
- `rejects_expired`, `rejects_wrong_audience`, `rejects_wrong_issuer`, `rejects_unknown_kid` → `AuthError::UnknownToken`
- `non_jwt_token_falls_through` — `authenticate("opaque-token")` on the *chained* wrapper (see Step 3) delegates to inner backend

- [x] **Step 2: Implement** `oidc_backend.rs`:

```rust
//! Generic OIDC bearer-token validation. Configured with one or more trusted
//! issuers; JWKS are fetched via OIDC discovery and cached, refreshing when
//! an unknown `kid` is seen (rate-limited). Tokens that do not look like JWTs
//! fall through to the wrapped inner backend, so native Kyma tokens keep
//! working unchanged.

use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

/// Config parsed from env (see `from_env`).
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuers: Vec<String>,
    pub audience: String,
    pub role_claim: String,      // default "kyma_role"
    pub subject_claim: String,   // default "sub"
    pub databases_claim: String, // default "kyma_databases"
}

impl OidcConfig {
    /// `None` when KYMA_OIDC_ISSUERS is unset/empty (OIDC disabled).
    pub fn from_env() -> Option<Self> {
        let issuers: Vec<String> = std::env::var("KYMA_OIDC_ISSUERS")
            .ok()?
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if issuers.is_empty() {
            return None;
        }
        let audience = std::env::var("KYMA_OIDC_AUDIENCE").unwrap_or_else(|_| "kyma".into());
        Some(Self {
            issuers,
            audience,
            role_claim: std::env::var("KYMA_OIDC_ROLE_CLAIM").unwrap_or_else(|_| "kyma_role".into()),
            subject_claim: std::env::var("KYMA_OIDC_SUBJECT_CLAIM").unwrap_or_else(|_| "sub".into()),
            databases_claim: std::env::var("KYMA_OIDC_DATABASES_CLAIM")
                .unwrap_or_else(|_| "kyma_databases".into()),
        })
    }
}

struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
}

/// Validates JWTs from configured OIDC issuers; delegates non-JWT tokens to
/// `inner`. Auth is "enabled" if either side is enabled.
pub struct OidcAuthBackend {
    cfg: OidcConfig,
    inner: Arc<dyn AuthBackend>,
    http: reqwest::Client,
    jwks: RwLock<HashMap<String, CachedJwks>>, // issuer -> cached JWKS
}

const JWKS_TTL: Duration = Duration::from_secs(3600);
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30); // unknown-kid refresh rate limit

impl OidcAuthBackend {
    pub fn new(cfg: OidcConfig, inner: Arc<dyn AuthBackend>) -> Self {
        Self { cfg, inner, http: reqwest::Client::new(), jwks: RwLock::new(HashMap::new()) }
    }

    /// JWTs have exactly three dot-separated base64url segments.
    fn looks_like_jwt(token: &str) -> bool {
        token.split('.').count() == 3
    }

    async fn jwks_for(&self, issuer: &str, force: bool) -> Result<JwkSet, AuthError> {
        {
            let cache = self.jwks.read().await;
            if let Some(c) = cache.get(issuer) {
                let fresh = c.fetched_at.elapsed() < JWKS_TTL;
                let throttle = c.fetched_at.elapsed() < JWKS_MIN_REFRESH;
                if (fresh && !force) || (force && throttle) {
                    return Ok(c.set.clone());
                }
            }
        }
        let discovery: serde_json::Value = self
            .http
            .get(format!("{issuer}/.well-known/openid-configuration"))
            .send()
            .await
            .map_err(|e| AuthError::Backend(format!("oidc discovery: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Backend(format!("oidc discovery body: {e}")))?;
        let jwks_uri = discovery
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::Backend("oidc discovery missing jwks_uri".into()))?;
        let set: JwkSet = self
            .http
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| AuthError::Backend(format!("jwks fetch: {e}")))?
            .json()
            .await
            .map_err(|e| AuthError::Backend(format!("jwks body: {e}")))?;
        self.jwks
            .write()
            .await
            .insert(issuer.to_owned(), CachedJwks { set: set.clone(), fetched_at: Instant::now() });
        Ok(set)
    }

    async fn validate_jwt(&self, token: &str) -> Result<Principal, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::UnknownToken)?;
        let kid = header.kid.ok_or(AuthError::UnknownToken)?;
        // Untrusted peek at `iss` to pick the issuer (signature verified below).
        let claims_b64 = token.split('.').nth(1).ok_or(AuthError::UnknownToken)?;
        let claims_raw = base64_url_decode(claims_b64).ok_or(AuthError::UnknownToken)?;
        let unverified: serde_json::Value =
            serde_json::from_slice(&claims_raw).map_err(|_| AuthError::UnknownToken)?;
        let iss = unverified
            .get("iss")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_end_matches('/'))
            .ok_or(AuthError::UnknownToken)?;
        if !self.cfg.issuers.iter().any(|i| i == iss) {
            return Err(AuthError::UnknownToken);
        }

        let mut set = self.jwks_for(iss, false).await?;
        let mut jwk = set.find(&kid).cloned();
        if jwk.is_none() {
            set = self.jwks_for(iss, true).await?; // key rotation
            jwk = set.find(&kid).cloned();
        }
        let jwk = jwk.ok_or(AuthError::UnknownToken)?;
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| AuthError::UnknownToken)?;
        let alg = header.alg;
        if !matches!(alg, Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::ES256 | Algorithm::ES384) {
            return Err(AuthError::UnknownToken);
        }
        let mut validation = Validation::new(alg);
        validation.set_audience(&[&self.cfg.audience]);
        validation.set_issuer(&[iss]);
        let data = decode::<serde_json::Value>(token, &key, &validation)
            .map_err(|_| AuthError::UnknownToken)?;
        let claims = data.claims;

        let role = claims
            .get(&self.cfg.role_claim)
            .and_then(|v| v.as_str())
            .and_then(Role::parse)
            .unwrap_or(Role::Read);
        let subject = claims
            .get(&self.cfg.subject_claim)
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let allowed_databases = claims.get(&self.cfg.databases_claim).and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect::<Vec<_>>())
        });

        Ok(Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role,
            subject,
            allowed_databases,
        })
    }
}

fn base64_url_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

#[async_trait]
impl AuthBackend for OidcAuthBackend {
    fn enabled(&self) -> bool {
        true // configured issuers ⇒ auth is on (inner may add native tokens)
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if Self::looks_like_jwt(token) {
            match self.validate_jwt(token).await {
                Ok(p) => return Ok(p),
                // A JWT that fails OIDC validation might still be a native
                // token that happens to contain dots — fall through only if
                // the inner backend is enabled.
                Err(e) if !self.inner.enabled() => return Err(e),
                Err(_) => {}
            }
        }
        self.inner.authenticate(token).await
    }
}
```

(Tenant note: self-hosted OIDC maps to `DEFAULT_TENANT`; the cloud control plane keeps `DbAuthBackend`. Add a doc comment saying multi-tenant OIDC claim mapping is a cloud-track follow-up.)

- [x] **Step 3:** Unit tests from Step 1: generate an RSA keypair in tests, build a `JwkSet` from the public key, and inject it by writing into `jwks` cache directly (`backend.jwks.write().await.insert(...)` via a `#[cfg(test)] pub(crate) fn inject_jwks(...)` helper) so unit tests never hit the network. Sign tokens with `jsonwebtoken::encode` + matching `kid`.
- [x] **Step 4:** `cargo test -p kyma-server oidc` — PASS.
- [x] **Step 5: Commit** `git commit -am "feat(server): OidcAuthBackend — generic OIDC JWT validation with claim-mapped role/databases"`

### Task 2.3: Wire OIDC into backend selection + CORS allow-list

**Files:**
- Modify: `crates/kyma-bin/src/main.rs` (backend selection block, ~lines 200-244; CORS application ~line 942)
- Modify: `crates/kyma-server/src/lib.rs:256-271` (CORS helper)

- [x] **Step 1: Backend wiring.** After the existing selection produces `backend: Arc<dyn AuthBackend>`, wrap it:

```rust
let backend: Arc<dyn AuthBackend> = match kyma_server::auth::oidc_backend::OidcConfig::from_env() {
    Some(cfg) => {
        tracing::info!(issuers = ?cfg.issuers, audience = %cfg.audience, "OIDC auth enabled");
        Arc::new(kyma_server::auth::oidc_backend::OidcAuthBackend::new(cfg, backend))
    }
    None => backend,
};
```

- [x] **Step 2: CORS.** In `lib.rs`, add a configurable variant alongside the permissive helper:

```rust
/// CORS for production: explicit origin allow-list from
/// `KYMA_CORS_ALLOWED_ORIGINS` (comma-separated). Falls back to the
/// permissive mirror behavior when unset (dev default).
pub fn with_configured_cors(r: Router) -> Router {
    use tower_http::cors::{AllowOrigin, Any, CorsLayer};
    let origins = std::env::var("KYMA_CORS_ALLOWED_ORIGINS").ok().map(|v| {
        v.split(',')
            .filter_map(|s| s.trim().parse::<axum::http::HeaderValue>().ok())
            .collect::<Vec<_>>()
    });
    let cors = match origins {
        Some(list) if !list.is_empty() => CorsLayer::new()
            .allow_origin(AllowOrigin::list(list))
            .allow_methods(Any)
            .allow_headers(Any)
            .expose_headers(Any),
        _ => {
            tracing::warn!(
                "KYMA_CORS_ALLOWED_ORIGINS unset — using permissive CORS (dev only)"
            );
            return with_permissive_cors(r);
        }
    };
    r.layer(cors)
}
```

and switch `main.rs` to `with_configured_cors(app)`.

- [x] **Step 3:** `cargo build` clean; add a unit test in `lib.rs` asserting `with_configured_cors` parses a two-origin env value (use `temp_env`-style guard or set/remove var inside the test serially).
- [x] **Step 4: Commit** `git commit -am "feat(server): OIDC backend wiring + KYMA_CORS_ALLOWED_ORIGINS allow-list"`

### Task 2.4: Per-database scope enforcement

**Files:**
- Create: `crates/kyma-server/src/auth/scope.rs`
- Modify: `crates/kyma-server/src/auth/mod.rs`
- Modify: each handler that resolves a target database. Find them with `rg -l "x-database|X-Database" crates/kyma-server/src` and `rg -l "Path.*db" crates/kyma-server/src` — expected: query, explore/discover, graph, catalog/schema, cleanup, dashboards-data paths.

- [x] **Step 1: Failing test** (in `scope.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::{Principal, Role};
    use kyma_core::tenant::DEFAULT_TENANT;

    fn p(dbs: Option<Vec<&str>>) -> Principal {
        Principal {
            tenant: DEFAULT_TENANT,
            role: Role::Read,
            subject: None,
            allowed_databases: dbs.map(|v| v.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn unrestricted_allows_everything() {
        assert!(check_database_scope(&p(None), "prod").is_ok());
    }
    #[test]
    fn scoped_allows_listed() {
        assert!(check_database_scope(&p(Some(vec!["prod"])), "prod").is_ok());
    }
    #[test]
    fn scoped_rejects_unlisted() {
        assert!(check_database_scope(&p(Some(vec!["prod"])), "staging").is_err());
    }
}
```

- [x] **Step 2: Implement**

```rust
//! Per-database authorization. `Principal.allowed_databases == None` means
//! unrestricted (all pre-OIDC backends). Handlers call this at the moment
//! they resolve which database a request targets.

use super::backend::Principal;
use axum::http::StatusCode;

pub fn check_database_scope(principal: &Principal, database: &str) -> Result<(), (StatusCode, String)> {
    match &principal.allowed_databases {
        None => Ok(()),
        Some(allowed) if allowed.iter().any(|d| d == database) => Ok(()),
        Some(_) => Err((
            StatusCode::FORBIDDEN,
            format!("token not scoped to database `{database}`"),
        )),
    }
}
```

- [x] **Step 3:** Apply at every database-resolution point found in the rg sweep. Pattern: the handler already has `Extension(principal): Extension<Principal>` (or add it — middleware inserts it); right after the database name is determined, insert `check_database_scope(&principal, &db).map_err(IntoResponse::into_response)?;` adjusted to each handler's error style. Be exhaustive: every `/v1/query`, `/v1/explore/*`, `/v1/graph/*`, `/v1/catalog/*`, `/v1/database/:db/*` route.
- [x] **Step 4:** Integration test in `crates/kyma-server/tests/` following `catalog_http.rs` harness: spin the router with a stub backend returning a Principal scoped to `["allowed_db"]`; assert `GET /v1/catalog/schema` with `x-database: other_db` → 403, with `x-database: allowed_db` → 200.
- [x] **Step 5:** `cargo test -p kyma-server` — PASS.
- [x] **Step 6: Commit** `git commit -am "feat(server): enforce per-database token scope at database resolution points"`

### Task 2.5: OIDC end-to-end integration test (mock issuer)

**Files:**
- Create: `crates/kyma-server/tests/oidc_backend_it.rs`

- [x] **Step 1:** Build a mock issuer: an axum test server (bound to `127.0.0.1:0`) serving `/.well-known/openid-configuration` (pointing at itself for `jwks_uri`) and `/jwks` with a generated RSA public key. Mint tokens signed by the private key.
- [x] **Step 2:** Tests: (a) full flow — env-config the backend with the mock issuer, authenticate a valid token, assert Principal claims incl. `allowed_databases`; (b) key rotation — swap the issuer's key, assert refresh-on-unknown-kid validates the new token; (c) chaining — opaque token falls through to an `EnvAuthBackend` configured alongside.
- [x] **Step 3:** `cargo test -p kyma-server --test oidc_backend_it` — PASS.
- [x] **Step 4: Commit** `git commit -am "test(server): OIDC integration tests with mock issuer (rotation, chaining)"`

---

## Phase 3 — @kyma-ai/react core (provider, theme, errors)

### Task 3.1: Theme tokens + presets

**Files:**
- Create: `packages/react/src/theme/tokens.ts`
- Create: `packages/react/src/theme/presets.ts`
- Test: `packages/react/src/theme/tokens.test.ts`

- [x] **Step 1:** Define `KymaTheme` as a flat record of every token used by the tailwind config (Task 0.3 Step 4): `background, surface, foreground, border, borderStrong, input, ring, primary, primaryForeground, secondary, secondaryForeground, destructive, destructiveForeground, muted, mutedForeground, accent, accentForeground, card, cardForeground, popover, popoverForeground, brandFrom, brandTo` (HSL triplet strings like `"213 26% 7%"`), plus `radius` (e.g. `"0.5rem"`), `fontSans`, `fontMono`. Write `themeToCssVars(theme: Partial<KymaTheme>): Record<string, string>` mapping camelCase → `--kyma-kebab-case`.

Failing test:

```ts
import { describe, expect, it } from "vitest";
import { themeToCssVars } from "./tokens";
import { kymaDark, kymaLight } from "./presets";

describe("themeToCssVars", () => {
  it("maps camelCase tokens to --kyma-kebab-case vars", () => {
    const vars = themeToCssVars({ background: "213 26% 7%", brandFrom: "180 72% 45%" });
    expect(vars["--kyma-background"]).toBe("213 26% 7%");
    expect(vars["--kyma-brand-from"]).toBe("180 72% 45%");
  });
  it("presets cover every token", () => {
    for (const preset of [kymaDark, kymaLight]) {
      const vars = themeToCssVars(preset);
      expect(Object.keys(vars).length).toBeGreaterThanOrEqual(26);
      expect(vars["--kyma-font-sans"]).toBeTruthy();
    }
  });
});
```

- [x] **Step 2:** Implement; populate `kymaDark`/`kymaLight` by **copying the literal HSL values from `web/src/styles/globals.css`** `:root` (light) and `.dark` blocks. `fontSans` default: `"ui-sans-serif, system-ui, sans-serif"` (inherit-friendly — never force a webfont on hosts); `fontMono`: `"ui-monospace, SFMono-Regular, Menlo, monospace"`.
- [x] **Step 3:** Tests pass. Commit: `git commit -am "feat(react): KymaTheme tokens and dark/light presets"`

### Task 3.2: KymaProvider

**Files:**
- Create: `packages/react/src/provider/KymaProvider.tsx`
- Create: `packages/react/src/provider/context.ts`
- Create: `packages/react/src/provider/portal.tsx`
- Test: `packages/react/src/provider/KymaProvider.test.tsx`

- [x] **Step 1: Failing test**

```tsx
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { KymaProvider } from "./KymaProvider";
import { useKymaClient } from "./context";

function Probe() {
  const client = useKymaClient();
  return <div data-testid="ep">{client.transport.endpoint}</div>;
}

describe("KymaProvider", () => {
  it("provides a client and renders a kyma-root with theme vars", () => {
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }}>
        <Probe />
      </KymaProvider>,
    );
    expect(screen.getByTestId("ep").textContent).toBe("https://kyma.test");
    const root = document.querySelector(".kyma-root") as HTMLElement;
    expect(root).toBeTruthy();
    expect(root.style.getPropertyValue("--kyma-background")).not.toBe("");
  });

  it("throws a descriptive error when hooks are used outside the provider", () => {
    expect(() => render(<Probe />)).toThrow(/KymaProvider/);
  });
});
```

- [x] **Step 2: Implement.**

`context.ts`:

```ts
import { createContext, useContext } from "react";
import type { KymaClient } from "@kyma-ai/client";
import type { KymaTheme } from "../theme/tokens";

export interface KymaContextValue {
  client: KymaClient;
  theme: Required<KymaTheme>;
  /** Element Radix portals must target so they stay inside .kyma-root vars. */
  portalContainer: HTMLElement | null;
  onError?: (err: unknown) => void;
}

export const KymaContext = createContext<KymaContextValue | null>(null);

export function useKymaContext(): KymaContextValue {
  const ctx = useContext(KymaContext);
  if (!ctx) throw new Error("Kyma components must be rendered inside <KymaProvider>");
  return ctx;
}

export function useKymaClient() {
  return useKymaContext().client;
}
```

`KymaProvider.tsx`:

```tsx
import { useMemo, useRef, useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createKymaClient, type KymaAuth } from "@kyma-ai/client";
import { KymaContext } from "./context";
import { themeToCssVars, type KymaTheme } from "../theme/tokens";
import { kymaDark } from "../theme/presets";

export interface KymaProviderProps {
  endpoint: string;
  auth: KymaAuth;
  database?: string;
  /** Preset or partial override; merged over kymaDark. "inherit" maps host vars. */
  theme?: Partial<KymaTheme> | "inherit";
  /** Reuse the host's React Query client; otherwise an isolated one is created. */
  queryClient?: QueryClient;
  onError?: (err: unknown) => void;
  children: ReactNode;
}

export function KymaProvider(props: KymaProviderProps) {
  const { endpoint, auth, database, theme, queryClient, onError, children } = props;

  const client = useMemo(
    () => createKymaClient({ endpoint, auth, database }),
    // auth is intentionally captured once per endpoint/database — token
    // rotation happens via getToken, not by re-creating the client.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [endpoint, database],
  );

  const ownQueryClient = useRef<QueryClient>();
  if (!queryClient && !ownQueryClient.current) {
    ownQueryClient.current = new QueryClient({
      defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 } },
    });
  }
  const qc = queryClient ?? ownQueryClient.current!;

  const resolvedTheme = useMemo(() => {
    if (theme === "inherit") return { ...kymaDark }; // vars come from host CSS; presets are fallbacks
    return { ...kymaDark, ...theme };
  }, [theme]);
  const cssVars = useMemo(() => themeToCssVars(resolvedTheme), [resolvedTheme]);

  const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(null);

  const value = useMemo(
    () => ({ client, theme: resolvedTheme as Required<KymaTheme>, portalContainer, onError }),
    [client, resolvedTheme, portalContainer, onError],
  );

  return (
    <KymaContext.Provider value={value}>
      <QueryClientProvider client={qc}>
        <div className="kyma-root" style={cssVars as React.CSSProperties}>
          {children}
          {/* Portal target carrying the same class + vars so Radix popovers/
              dialogs keep their theming when they escape the DOM subtree. */}
          <div ref={setPortalContainer} className="kyma-root" style={cssVars as React.CSSProperties} />
        </div>
      </QueryClientProvider>
    </KymaContext.Provider>
  );
}
```

- [x] **Step 3:** Tests pass. Commit: `git commit -am "feat(react): KymaProvider with isolated query client, theme vars, portal container"`

### Task 3.3: Error boundary + capability gating

**Files:**
- Create: `packages/react/src/internal/KymaErrorBoundary.tsx` (class component; default fallback = inline card with message + retry button that resets the boundary; reports to `onError` from context)
- Create: `packages/react/src/hooks/useKymaCapabilities.ts` (React Query over `client.capabilities`, `staleTime: 5min`)
- Create: `packages/react/src/internal/CapabilityGate.tsx` (renders children when capability present; styled "not available on this server" card otherwise)
- Tests: co-located `.test.tsx` for each — boundary catches a throwing child and renders fallback + calls onError; gate renders fallback when capability absent.

- [x] **Step 1:** Failing tests → **Step 2:** implement → **Step 3:** pass → **Step 4:** Commit `git commit -am "feat(react): error boundary and capability gating"`

---

## Phase 4 — Headless hooks

### Task 4.1: useKymaGraph

**Files:**
- Create: `packages/react/src/hooks/useKymaGraph.ts`
- Test: `packages/react/src/hooks/useKymaGraph.test.tsx`
- Reference: `web/src/features/graph/useGraph.ts` (`useUnifiedGraph` — port its merge logic)

API:

```ts
export interface UseKymaGraphArgs {
  /** Graphs to merge; omit to load all graphs of the default database. */
  graphs?: Array<{ database?: string; graph: string }>;
  realm?: string;
  limit?: number;
}
export interface UseKymaGraphResult {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  stats: GraphStats | null;
  isLoading: boolean;
  error: unknown;
  expandNode: (nodeId: string) => Promise<void>;
  searchNodes: (text: string) => Promise<SearchHits>;
  refetch: () => void;
}
export function useKymaGraph(args?: UseKymaGraphArgs): UseKymaGraphResult;
```

- [x] **Step 1:** Failing test with a provider wrapper + mocked client (renderHook from Testing Library; mock `client.graph.listGraphs/getOverview` via a fake transport fetch returning fixture payloads). Assert merged nodes from two graphs carry `namespace`/`database` and `expandNode` appends edges.
- [x] **Step 2:** Implement by porting `useUnifiedGraph` merge logic from `web/src/features/graph/useGraph.ts`, swapping session access for `useKymaClient()` and `useQueries` keyed `["kyma", endpoint, database, "graph", ...]`.
- [x] **Step 3:** Pass. Commit `git commit -am "feat(react): useKymaGraph headless hook"`

### Task 4.2: useKymaQuery, useKymaDiscover, useKymaDashboards, useKymaAgent

Same recipe per hook — failing test with mocked client → port logic → pass → commit each:

- `useKymaQuery`: wraps `client.query.executeQuery` (Arrow decode already in client); returns `{ execute(query, opts), columns, rows, arrow, isRunning, error, cancel }`. Reference `web/src/routes/_app.query.tsx` for how results/columns are derived today.
- `useKymaDiscover`: wraps the NDJSON async generator `client.discover.searchDiscover`; accumulates frames into state; `abort()` cancels via AbortController; unmount aborts. Reference `web/src/features/discover/useDiscoverSearch.ts`.
- `useKymaDashboards`: React Query CRUD over `client.dashboards`.
- `useKymaAgent`: port the transport glue from `web/src/features/agent/transport.ts` against `client.agent`; returns `{ messages, send, status, stop }`.
- [x] All four implemented, tested, committed (`feat(react): useKymaQuery/useKymaDiscover/useKymaDashboards/useKymaAgent`).

### Task 4.3: Root index exports

- [x] `packages/react/src/index.ts` exports: `KymaProvider`, all hooks, theme (`kymaDark`, `kymaLight`, `KymaTheme`, `themeToCssVars`), error boundary, and re-exports types from `@kyma-ai/client`. Build + typecheck + commit `git commit -am "feat(react): public root exports"`.

---

## Phase 5 — The five components

**Shared extraction rules (apply in every task of this phase):**

1. `git mv` the source files from `web/src/features/<x>/` into `packages/react/src/<view>/` (keep history). Shared UI primitives they import from `web/src/components/ui/*` move (once, first time needed) to `packages/react/src/internal/ui/` — **copy** (not move) any primitive web's shell still uses; web keeps its own copy until Phase 6 decides per-file.
2. Replace `useSession()`/endpoint/token access with `useKymaClient()`; replace module-level Zustand stores with **per-instance stores**: `const store = useRef(createGraphStore()).current` + React context within the component subtree (pattern: `createStore` from `zustand/vanilla` + `useStore`).
3. Prefix every Tailwind class: run the codemod `node scripts/ky-prefix.mjs packages/react/src/<view>` (created in Task 5.1) then **review the diff by hand** — dynamic `cn(cond && "...")` strings are included; template-literal class fragments must be checked manually.
4. Radix portal components must pass `container={useKymaContext().portalContainer}` to their `*.Portal` part.
5. Wrap the exported component in `KymaErrorBoundary` + `CapabilityGate` where the view needs a capability.
6. Storybook story added in Phase 8 (not here).
7. Per-view smoke test: renders inside `KymaProvider` with mocked fetch; asserts the view's main landmark appears and **no global store leaks** (two instances mounted side-by-side hold independent state).

### Task 5.1: ky-prefix codemod

**Files:**
- Create: `scripts/ky-prefix.mjs`

- [x] **Step 1:** Write the codemod: walks `.tsx/.ts` files, rewrites Tailwind tokens inside string literals that appear in `className=`, `cn(...)`, `cva(...)` call expressions. Token rewrite rule: each whitespace-separated class `c` becomes `ky-` + c, preserving variant prefixes (`hover:flex` → `hover:ky-flex`, `md:dark:px-2` → `md:dark:ky-px-2`) and arbitrary values; already-prefixed tokens and non-tailwind tokens pass through (maintain an allowlist by loading the package tailwind config class candidates via `tailwindcss` API — pragmatic fallback: rewrite all tokens, then `pnpm --filter @kyma-ai/react build:css` + grep dist/style.css to verify generated classes; unknown tokens are inert).
- [x] **Step 2:** Unit-test the rewrite function with cases: plain, variant-chained, arbitrary value `w-[13px]`, negative `-mt-2` → `-ky-mt-2`, template literal skipped + warning emitted.
- [x] **Step 3:** Commit `git commit -am "chore: ky- tailwind prefix codemod"`

### Task 5.2: KymaGraph

**Files:**
- Move: `web/src/features/graph/{GraphView,GraphCanvas,GraphSidebar}.tsx`, `graph-store.ts`, `graph-style.ts`, `graph-icons.tsx`, `graph-community.ts`, (+ any siblings found in the dir) → `packages/react/src/graph/`
- Create: `packages/react/src/graph/KymaGraph.tsx` (public wrapper)
- Create: `packages/react/src/graph/index.ts`
- Test: `packages/react/src/graph/KymaGraph.test.tsx`

Public API (from the spec):

```tsx
export interface KymaGraphProps {
  graphs?: Array<{ database?: string; graph: string }>;
  height?: number | string;            // default "100%"
  layout?: "force" | "hierarchical" | "radial";
  toolbar?: boolean;                   // default true
  sidebar?: boolean;                   // default true
  minimap?: boolean;                   // default true
  focusQuery?: string;
  database?: string;
  className?: string;
  style?: React.CSSProperties;
  fallback?: ReactNode;
  onNodeClick?: (node: GraphNode) => void;
  onSelectionChange?: (nodes: GraphNode[]) => void;
  renderNodeTooltip?: (node: GraphNode) => ReactNode;
}
export function KymaGraph(props: KymaGraphProps): JSX.Element;
```

- [x] **Steps:** failing smoke test (two instances = independent selection state; `onNodeClick` fires from canvas click handler — simulate by calling the handler prop on the mocked ForceGraph2D, mock `react-force-graph-2d` in vitest as a div) → move files → apply shared rules 1–5 → `graph-store.ts` becomes `createGraphStore()` (zustand vanilla) provided via `GraphStoreContext` → theme colors in `graph-style.ts` read from theme context tokens instead of checking `.dark` on `<html>` → codemod prefixes → tests pass → web/graph route temporarily imports from `@kyma-ai/react/graph` (quick edit of `web/src/routes/_app.graph.tsx` to keep the app compiling; full dogfood pass in Phase 6).
- [x] **Verify:** `pnpm --filter @kyma-ai/react test && pnpm --filter @kyma-ai/react build && pnpm --filter kyma-web build`
- [x] **Commit** `git commit -am "feat(react): KymaGraph embeddable component"`

### Task 5.3: KymaQueryEditor

- Move: `web/src/features/editor/`, `web/src/features/schema-browser/`, `web/src/features/results-grid/`, `web/src/features/time-range/`, `web/src/features/chart/ChartPanel.tsx` → `packages/react/src/query/` (subfolders `editor/`, `schema/`, `results/`, `time-range/`, `chart/`).
- Create: `packages/react/src/query/KymaQueryEditor.tsx` with props from the spec (`language, defaultQuery, showSchemaBrowser, showResults, timeRange, readOnly, onResults, renderResultCell, database, className, style, fallback`).
- Workspace-tabs coupling: the editor components must NOT import `features/tabs` — tab orchestration stays app-side; `KymaQueryEditor` is single-query. Where the web editor read tab state, lift to internal `useState` seeded from `defaultQuery`.
- Monaco: keep `@monaco-editor/react` lazy loading; expose `monacoTheme` mapping from theme tokens (port from wherever web configures the Monaco theme — find via `rg -l monaco web/src`).
- [x] Same step structure as 5.2 (failing smoke test → move → decouple → prefix → pass → commit `feat(react): KymaQueryEditor embeddable component`).

### Task 5.4: KymaDiscover

- Move: `web/src/features/discover/*` (DiscoverPage, FieldsRail, Histogram, QueryBar, RowDetailDrawer, ScopePicker, SourceTableView, SourcesRail, StreamView, SummaryLine + their tests; SavedViewsMenu stays app-side — saved views are an app concern), `web/src/features/field-stats/`, `web/src/features/histogram/` → `packages/react/src/discover/`.
- Create: `KymaDiscover.tsx` (`sources, defaultQuery, onRowClick, database, className, style, fallback`); discover-store becomes per-instance.
- Existing `StreamView.test.ts`/`StreamView.integration.test.tsx`/`SummaryLine.test.ts` move along and must stay green.
- [x] Same step structure; commit `feat(react): KymaDiscover embeddable component`.

### Task 5.5: KymaDashboard

- Move: `web/src/features/dashboards/{PanelCard.tsx,panels/,useDashboardQuery.ts}` → `packages/react/src/dashboards/`; `AddPanelModal.tsx` moves too (needed for `editable`).
- Create: `KymaDashboard.tsx` (`dashboardId, editable=false, timeRange, database, className, style, fallback`) — loads via `useKymaDashboards`, renders react-grid-layout, panels execute through `useKymaQuery`; ECharts theme derived from theme tokens.
- [x] Same step structure; commit `feat(react): KymaDashboard embeddable component`.

### Task 5.6: KymaAgentChat

- Add deps to `packages/react/package.json`: `"ai": "^6.0.193"`, `"@ai-sdk/react": "^3.0.195"`, `"streamdown": "^2.5.0"`.
- Move: `web/src/features/agent/{AgentConsole.tsx,transport.ts}` + `web/src/components/ai-elements/` → `packages/react/src/agent/` (MemoryPanel/MemorySearch/MemorySettingsPanel stay app-side — settings surface, not embed surface; AskAIDialog stays app-side).
- Create: `KymaAgentChat.tsx` (`placeholder, systemContext, onMessage, renderMessage, database, className, style, fallback`), gated by `CapabilityGate` on the agent capability.
- [x] Same step structure; commit `feat(react): KymaAgentChat embeddable component`.

---

## Phase 6 — web/ dogfoods the packages

### Task 6.1: Replace shimmed call sites with a session-level client

**Files:**
- Create: `web/src/sdk/client.ts`
- Delete: `web/src/sdk/compat.ts`, `web/src/sdk/auth-fetch.ts`
- Modify: `web/src/main.tsx` (drop `installAuthFetch()`), all `transportFor(...)` call sites

- [x] **Step 1:** `web/src/sdk/client.ts`:

```ts
// App-level KymaClient bound to the live session store. Token rotation and
// dead-session redirects reuse the app's existing refresh flow.
import { createKymaClient, KymaAuthError } from "@kyma-ai/client";
import { useSession } from "./session";
import { refresh } from "@kyma-ai/client";

let cached: { key: string; client: ReturnType<typeof createKymaClient> } | null = null;

export function sessionClient() {
  const { endpoint, database } = useSession.getState();
  const key = `${endpoint}|${database ?? ""}`;
  if (cached?.key === key) return cached.client;
  const client = createKymaClient({
    endpoint,
    database,
    auth: {
      getToken: async ({ reason } = { reason: "initial" }) => {
        const s = useSession.getState();
        if (reason === "expired") {
          try {
            const pair = await refresh({ endpoint: s.endpoint, refreshToken: s.refreshToken });
            s.set({ token: pair.access_token, refreshToken: pair.refresh_token, accessExpiresAt: pair.access_expires_at, user: pair.user });
            return pair.access_token;
          } catch {
            s.reset();
            if (!window.location.pathname.startsWith("/login")) {
              const next = encodeURIComponent(window.location.pathname + window.location.search);
              window.location.assign(`/login?next=${next}`);
            }
            throw new KymaAuthError(401, "session expired");
          }
        }
        return s.token;
      },
    },
  });
  cached = { key, client };
  return client;
}
```

- [x] **Step 2:** Sweep `rg -l transportFor web/src` and replace with `sessionClient().<ns>.<fn>(args)`; delete compat.ts + auth-fetch.ts; remove the `installAuthFetch()` call in `main.tsx`.
- [x] **Step 3:** `pnpm --filter kyma-web typecheck && pnpm --filter kyma-web test` — PASS.
- [x] **Step 4: Commit** `git commit -am "refactor(web): session-level KymaClient replaces global fetch patching"`

### Task 6.2: Routes render package components inside KymaProvider

**Files:**
- Modify: `web/src/app/providers.tsx` — wrap the router in `KymaProvider` fed from the session store (`endpoint`, `auth: {getToken}` same as 6.1, `theme` mapped from `useTheme().resolved` → `kymaDark`/`kymaLight`, `queryClient` = the app's existing one).
- Modify: `web/src/routes/_app.graph.tsx` → renders `<KymaGraph>` (props from URL search params; selection events drive app-side deep links).
- Modify: `web/src/routes/_app.discover.tsx` → `<KymaDiscover>` + app-side SavedViewsMenu/tab chrome around it.
- Modify: `web/src/routes/_app.query.tsx` → `<KymaQueryEditor>` per workspace tab (tab state stays app-side; editor state per tab via `key={tabId}` + `defaultQuery`).
- Modify: `web/src/routes/_app.dashboards.$id.tsx` → `<KymaDashboard dashboardId editable>`; dashboards list route keeps app-side list UI.
- Modify: `web/src/routes/_app.agent.tsx` → `<KymaAgentChat>` + app-side MemoryPanel.
- Delete: now-unused remnants under `web/src/features/{graph,discover,editor,schema-browser,results-grid,time-range,chart,field-stats,histogram}` and `web/src/components/ai-elements` (anything not moved that became dead — verify with `pnpm --filter kyma-web typecheck` + `rg` for imports before deleting each file).

- [x] **Step 1–N:** One route at a time: rewire → `pnpm --filter kyma-web typecheck && pnpm --filter kyma-web test` → visual check via `pnpm --filter kyma-web dev` against a local server (`cargo run -p kyma-bin` if feasible; otherwise existing dev server) → commit per route (`refactor(web): graph route renders @kyma-ai/react KymaGraph`, etc.).
- [x] **Final step:** `pnpm --filter kyma-web build` (production build) + `pnpm --filter kyma-web e2e` if the Playwright suite covers these routes — PASS. Commit.

---

## Phase 7 — Example app

### Task 7.1: examples/embed-demo

**Files:**
- Create: `examples/embed-demo/package.json` (name `kyma-embed-demo`, private, deps: `@kyma-ai/react workspace:*`, `@kyma-ai/client workspace:*`, react 18, vite, typescript; scripts dev/build/typecheck)
- Create: `examples/embed-demo/vite.config.ts`, `tsconfig.json`, `index.html`
- Create: `examples/embed-demo/src/main.tsx`, `src/App.tsx`
- Create: `examples/embed-demo/src/mint-token-server.mjs`
- Create: `examples/embed-demo/README.md`

- [x] **Step 1:** `App.tsx`: top-level config form (endpoint URL, token — persisted to localStorage), theme toggle (kymaDark/kymaLight/custom-accent demo), then a tabbed page rendering all five components inside one `KymaProvider`. Import `@kyma-ai/react/styles.css`.
- [x] **Step 2:** `mint-token-server.mjs`: ~40-line Node http server demonstrating the multi-tenant pattern — holds an admin/native token server-side, exposes `POST /api/kyma-token` returning it (with a comment block explaining the OIDC variant: mint a JWT with `kyma_role`/`kyma_databases` claims via the host's IdP instead).
- [x] **Step 3:** README: install/run instructions, the `getToken` wiring snippet, OIDC server config env vars (`KYMA_OIDC_ISSUERS`, `KYMA_OIDC_AUDIENCE`, claims reference, `KYMA_CORS_ALLOWED_ORIGINS`).
- [x] **Step 4:** `pnpm --filter kyma-embed-demo build && pnpm --filter kyma-embed-demo typecheck` — PASS. Manual verification against a running server (see Phase 10).
- [x] **Step 5: Commit** `git commit -am "feat(examples): embed-demo host app exercising all five components"`

---

## Phase 8 — Storybook

### Task 8.1: Storybook for @kyma-ai/react

**Files:**
- Create: `packages/react/.storybook/main.ts` (framework `@storybook/react-vite`), `.storybook/preview.tsx`
- Create: `packages/react/src/<view>/<Component>.stories.tsx` × 5 + `KymaProvider.stories.tsx` (theme playground)
- Modify: `packages/react/package.json` devDeps: `storybook`, `@storybook/react-vite`, `@storybook/react` (v8 line)

- [x] **Step 1:** `preview.tsx` decorator wraps every story in `KymaProvider` pointing at `import.meta.env.VITE_KYMA_ENDPOINT ?? "http://localhost:8080"` with token from `VITE_KYMA_TOKEN`, and injects a mock-fetch mode (default): a decorator-level fetch stub serving fixture JSON for graph/discover/query/dashboards/capabilities endpoints so stories render offline. Fixtures: create `packages/react/src/__fixtures__/{graph,discover,query,dashboards,capabilities}.ts` with small realistic payloads (copy shapes from client types).
- [x] **Step 2:** One story file per component: default story + knobs (argTypes) for the headline props (layout/toolbar/sidebar for graph; language/readOnly for query; theme tokens on the provider story).
- [x] **Step 3:** `pnpm --filter @kyma-ai/react build-storybook` — builds clean.
- [x] **Step 4: Commit** `git commit -am "feat(react): storybook with offline fixtures for all five components"`

---

## Phase 9 — Docs + publish pipeline

### Task 9.1: Embedding docs

**Files:**
- Create: `docs/embedding/README.md` (or `docs/site/` page if the docs site has a content dir — check `docs/site` structure first and place a page in its content tree, linked from its nav)

- [x] Content sections (write fully, no stubs): Install & quickstart (provider + one component in <20 lines) · Auth patterns (static token; getToken + mint endpoint; full OIDC server config incl. every `KYMA_OIDC_*` env var and the `kyma_role`/`kyma_databases` claim contract; CORS) · Theming (token table, presets, "inherit") · Per-component prop reference (all five, every prop from the Phase 5 APIs) · Headless hooks reference · Bundle-size note (subpath imports) · Server compatibility (capabilities-based degradation).
- [x] Commit `git commit -am "docs: embedding guide for @kyma-ai/react"`

### Task 9.2: Publish pipeline

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: existing CI workflow (find under `.github/workflows/`) — add a job: `pnpm install && pnpm build:packages && pnpm test:packages && pnpm --filter kyma-embed-demo build`

- [x] **Step 1:** `release.yml`: on push to main — `changesets/action@v1` with `publish: pnpm changeset publish`, `NPM_TOKEN` secret, `pnpm build:packages` as prepublish. Standard changesets release flow (version PR → merge → publish).
- [x] **Step 2:** Add `"publishConfig": { "access": "public" }` to both package.jsons. Verify `pnpm changeset version` dry-run works (`pnpm changeset add` a patch note first).
- [x] **Step 3:** Commit `git commit -am "ci: changesets release workflow for @kyma-ai packages"`

---

## Phase 10 — End-to-end verification & merge

### Task 10.1: Full verification sweep

- [x] `pnpm install` clean from lockfile
- [x] `pnpm typecheck:all` — PASS
- [x] `pnpm --filter @kyma-ai/client test` / `--filter @kyma-ai/react test` / `--filter kyma-web test` — PASS
- [x] `pnpm build:packages && pnpm --filter kyma-web build && pnpm --filter kyma-embed-demo build && pnpm --filter @kyma-ai/react build-storybook` — PASS
- [x] `cargo build --workspace && cargo test -p kyma-server` — PASS
- [x] Live smoke: start the server (`cargo run -p kyma-bin` with dev env), `pnpm --filter kyma-web dev` — click through all five views; `pnpm --filter kyma-embed-demo dev` — all five embedded components render data against the live server; verify theme toggle and a `KYMA_CORS_ALLOWED_ORIGINS` round-trip (set it to the demo's origin, restart server, confirm requests pass; set it to a wrong origin, confirm browser blocks).
- [x] OIDC smoke: run the mock-issuer integration test once more; optionally configure the live server with a real issuer if available — otherwise the IT suffices.

### Task 10.2: Finish the branch

- [x] Use superpowers:finishing-a-development-branch — user preference on this repo is local `git merge --no-ff` to main + push (see memory: local-merge sync preferred over PRs).

---

## Self-review (per writing-plans skill)

- **Spec coverage:** packages+factory (Ph 0/1) ✓ · OIDC/scoping/CORS (Ph 2) ✓ · provider/theme/portals (Ph 3) ✓ · headless hooks (Ph 4) ✓ · five components incl. per-instance stores + prefix isolation (Ph 5) ✓ · dogfooding (Ph 6) ✓ · example app + mint pattern (Ph 7) ✓ · Storybook (Ph 8) ✓ · docs + publish (Ph 9) ✓ · capabilities degradation (Task 3.3, used in 5.6) ✓ · npm-scope verification → Task 9.2 note: verify `@kyma-ai` availability before enabling publish; fallback `@agentcylabs/*` (rename is a find-replace + package.json edit).
- **Placeholder scan:** extraction tasks intentionally reference existing source files as the content source (move-refactors); all novel logic has full code. No TBDs.
- **Type consistency:** `KymaTransport.request(path, opts)` used by all modules; `KymaAuth` = `{token} | {getToken}` consistent across client/provider/docs; `Principal.allowed_databases: Option<Vec<String>>` consistent across 2.1/2.2/2.4.
