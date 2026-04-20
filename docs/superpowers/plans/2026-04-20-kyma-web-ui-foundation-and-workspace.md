# kyma Web UI — Foundation + Query Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an ADX-class web explorer for kyma that runs as a single binary (UI embedded in `kyma-server` under `--features web-ui`) and as a Tauri 2 desktop app.

**Architecture:** Vite + React + TS frontend, embedded into the Rust server via `include_dir!`. The browser talks to the existing Arrow Flight service via `tonic-web` (gRPC-web) on `:8080/flight/*`, and to a new `/v1/catalog/schema` endpoint for schema discovery. Tauri packages the same `dist/` bundle.

**Tech Stack:**
- **Server:** Rust (axum 0.7, tonic 0.12, tonic-web 0.12, `include_dir` 0.7, arrow-flight 53)
- **Frontend:** Vite 5, React 18.3, TypeScript 5.5, Tailwind 3.4, shadcn/ui, TanStack Router / Query / Table / Virtual, Monaco Editor 0.52, ECharts 5.5, apache-arrow 17, zustand 5
- **gRPC-web client:** `@bufbuild/buf` + `@connectrpc/connect-web` (codegen from a vendored `Flight.proto`)
- **Desktop:** Tauri 2, `@tauri-apps/plugin-store` for OS keyring
- **Packaging:** multi-stage Dockerfile; `docker-compose` gains a `kyma` service
- **CI:** two GitHub Actions workflows (`web.yml`, `release.yml`)

**Spec:** [`docs/superpowers/specs/2026-04-20-kyma-web-ui-foundation-and-workspace-design.md`](../specs/2026-04-20-kyma-web-ui-foundation-and-workspace-design.md)

---

## Pre-flight

Before starting, from the repo root:

```bash
# isolated worktree so this work doesn't collide with main
git worktree add .worktrees/web-ui -b feat/web-ui
cd .worktrees/web-ui

# node + pnpm — the plan assumes these versions are present
node --version   # must be >= 20
pnpm --version   # must be >= 9

# buf CLI for gRPC-web codegen
brew install bufbuild/buf/buf    # or: curl from https://github.com/bufbuild/buf/releases
buf --version    # must be >= 1.40
```

---

## Phase 1 — Web scaffold (minimal)

Goal: a Vite + React + TS app that builds an empty `dist/`, so downstream server tasks can `include_dir!` it.

### Task 1.1 — Initialize pnpm workspace root

**Files:**
- Create: `pnpm-workspace.yaml`
- Create: `.npmrc`
- Create: `package.json`

- [ ] **Step 1: Write `pnpm-workspace.yaml`**

```yaml
packages:
  - "web"
  - "web/src-tauri"
```

- [ ] **Step 2: Write `.npmrc`**

```
engine-strict=true
auto-install-peers=true
```

- [ ] **Step 3: Write root `package.json`**

```json
{
  "name": "kyma-workspace",
  "private": true,
  "scripts": {
    "dev": "pnpm -C web dev",
    "build": "pnpm -C web build",
    "test": "pnpm -C web test",
    "typecheck": "pnpm -C web typecheck",
    "lint": "pnpm -C web lint"
  },
  "engines": { "node": ">=20", "pnpm": ">=9" }
}
```

- [ ] **Step 4: Commit**

```bash
git add pnpm-workspace.yaml .npmrc package.json
git commit -m "feat(web): add pnpm workspace root"
```

### Task 1.2 — Scaffold Vite + React + TS in `web/`

**Files:**
- Create: `web/package.json`
- Create: `web/tsconfig.json`
- Create: `web/tsconfig.node.json`
- Create: `web/vite.config.ts`
- Create: `web/index.html`
- Create: `web/src/main.tsx`
- Create: `web/src/App.tsx`
- Create: `web/.gitignore`

- [ ] **Step 1: Write `web/package.json`**

```json
{
  "name": "kyma-web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "typecheck": "tsc -b --noEmit",
    "lint": "eslint . --ext .ts,.tsx",
    "test": "vitest run",
    "test:watch": "vitest",
    "generate": "buf generate"
  },
  "dependencies": {
    "react": "18.3.1",
    "react-dom": "18.3.1"
  },
  "devDependencies": {
    "@types/react": "18.3.3",
    "@types/react-dom": "18.3.0",
    "@typescript-eslint/eslint-plugin": "7.18.0",
    "@typescript-eslint/parser": "7.18.0",
    "@vitejs/plugin-react": "4.3.1",
    "eslint": "8.57.0",
    "eslint-plugin-react-hooks": "4.6.2",
    "eslint-plugin-react-refresh": "0.4.7",
    "typescript": "5.5.4",
    "vite": "5.4.0",
    "vitest": "2.0.5"
  }
}
```

- [ ] **Step 2: Write `web/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "baseUrl": ".",
    "paths": { "@/*": ["./src/*"] }
  },
  "include": ["src"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

- [ ] **Step 3: Write `web/tsconfig.node.json`**

```json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "strict": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 4: Write `web/vite.config.ts`**

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(__dirname, "src") } },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/v1":     { target: "http://localhost:8080", changeOrigin: true },
      "/flight": { target: "http://localhost:8080", changeOrigin: true },
      "/health": { target: "http://localhost:8080", changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
    target: "es2022",
  },
});
```

- [ ] **Step 5: Write `web/index.html`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="color-scheme" content="light dark" />
    <title>kyma</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 6: Write `web/src/main.tsx`**

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 7: Write `web/src/App.tsx`**

```tsx
export default function App() {
  return (
    <div style={{ padding: 24, fontFamily: "system-ui" }}>
      <h1>kyma</h1>
      <p>Web UI scaffolding. Nothing wired yet.</p>
    </div>
  );
}
```

- [ ] **Step 8: Write `web/.gitignore`**

```
node_modules
dist
.vite
*.log
```

- [ ] **Step 9: Install + build**

```bash
pnpm install
pnpm -C web build
```
Expected: `web/dist/index.html`, `web/dist/assets/*.js`, `web/dist/assets/*.css` all exist.

- [ ] **Step 10: Commit**

```bash
git add web/ package.json pnpm-lock.yaml
git commit -m "feat(web): scaffold Vite + React + TS"
```

### Task 1.3 — Tailwind CSS + shadcn/ui primitives

**Files:**
- Create: `web/tailwind.config.ts`
- Create: `web/postcss.config.js`
- Create: `web/src/styles/globals.css`
- Create: `web/components.json`
- Create: `web/src/lib/utils.ts`
- Modify: `web/src/main.tsx`
- Modify: `web/package.json`

- [ ] **Step 1: Install deps**

```bash
pnpm -C web add -D tailwindcss@3.4.10 postcss@8.4.41 autoprefixer@10.4.20 \
  tailwindcss-animate class-variance-authority clsx tailwind-merge
pnpm -C web add lucide-react @radix-ui/react-slot
```

- [ ] **Step 2: Write `web/tailwind.config.ts`**

```ts
import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

export default {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    container: { center: true, padding: "1.5rem" },
    extend: {
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary:   { DEFAULT: "hsl(var(--primary))",   foreground: "hsl(var(--primary-foreground))" },
        secondary: { DEFAULT: "hsl(var(--secondary))", foreground: "hsl(var(--secondary-foreground))" },
        destructive: { DEFAULT: "hsl(var(--destructive))", foreground: "hsl(var(--destructive-foreground))" },
        muted:     { DEFAULT: "hsl(var(--muted))",     foreground: "hsl(var(--muted-foreground))" },
        accent:    { DEFAULT: "hsl(var(--accent))",    foreground: "hsl(var(--accent-foreground))" },
        card:      { DEFAULT: "hsl(var(--card))",      foreground: "hsl(var(--card-foreground))" },
      },
      borderRadius: { lg: "var(--radius)", md: "calc(var(--radius) - 2px)", sm: "calc(var(--radius) - 4px)" },
      fontFamily: {
        sans: ["ui-sans-serif","system-ui","-apple-system","Segoe UI","Roboto","sans-serif"],
        mono: ["ui-monospace","SFMono-Regular","Menlo","monospace"],
      },
    },
  },
  plugins: [animate],
} satisfies Config;
```

- [ ] **Step 3: Write `web/postcss.config.js`**

```js
export default { plugins: { tailwindcss: {}, autoprefixer: {} } };
```

- [ ] **Step 4: Write `web/src/styles/globals.css`** (shadcn/ui "slate" tokens)

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root {
    --background: 0 0% 100%;
    --foreground: 222.2 84% 4.9%;
    --card: 0 0% 100%;
    --card-foreground: 222.2 84% 4.9%;
    --primary: 221.2 83.2% 53.3%;
    --primary-foreground: 210 40% 98%;
    --secondary: 210 40% 96.1%;
    --secondary-foreground: 222.2 47.4% 11.2%;
    --muted: 210 40% 96.1%;
    --muted-foreground: 215.4 16.3% 46.9%;
    --accent: 210 40% 96.1%;
    --accent-foreground: 222.2 47.4% 11.2%;
    --destructive: 0 84.2% 60.2%;
    --destructive-foreground: 210 40% 98%;
    --border: 214.3 31.8% 91.4%;
    --input: 214.3 31.8% 91.4%;
    --ring: 221.2 83.2% 53.3%;
    --radius: 0.5rem;
  }
  .dark {
    --background: 222.2 84% 4.9%;
    --foreground: 210 40% 98%;
    --card: 222.2 84% 4.9%;
    --card-foreground: 210 40% 98%;
    --primary: 217.2 91.2% 59.8%;
    --primary-foreground: 222.2 47.4% 11.2%;
    --secondary: 217.2 32.6% 17.5%;
    --secondary-foreground: 210 40% 98%;
    --muted: 217.2 32.6% 17.5%;
    --muted-foreground: 215 20.2% 65.1%;
    --accent: 217.2 32.6% 17.5%;
    --accent-foreground: 210 40% 98%;
    --destructive: 0 62.8% 30.6%;
    --destructive-foreground: 210 40% 98%;
    --border: 217.2 32.6% 17.5%;
    --input: 217.2 32.6% 17.5%;
    --ring: 224.3 76.3% 48%;
  }
  * { @apply border-border; }
  body { @apply bg-background text-foreground antialiased; }
}
```

- [ ] **Step 5: Write `web/components.json`** (shadcn config for later CLI adds)

```json
{
  "$schema": "https://ui.shadcn.com/schema.json",
  "style": "default",
  "tailwind": {
    "config": "tailwind.config.ts",
    "css": "src/styles/globals.css",
    "baseColor": "slate",
    "cssVariables": true
  },
  "aliases": {
    "components": "@/components",
    "ui": "@/components/ui",
    "utils": "@/lib/utils"
  }
}
```

- [ ] **Step 6: Write `web/src/lib/utils.ts`**

```ts
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
```

- [ ] **Step 7: Update `web/src/main.tsx`**

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

- [ ] **Step 8: Update `web/src/App.tsx`**

```tsx
export default function App() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold">kyma</h1>
      <p className="text-muted-foreground">Web UI scaffolding.</p>
    </div>
  );
}
```

- [ ] **Step 9: Verify build**

```bash
pnpm -C web build
```
Expected: `dist/` re-emitted; no errors; `dist/assets/*.css` contains `--background:` tokens.

- [ ] **Step 10: Commit**

```bash
git add web/ pnpm-lock.yaml
git commit -m "feat(web): add Tailwind + shadcn/ui primitives"
```

### Task 1.4 — TanStack Router, Query, zustand, toast

**Files:**
- Modify: `web/src/main.tsx`
- Create: `web/src/app/providers.tsx`
- Create: `web/src/app/router.tsx`
- Create: `web/src/routes/__root.tsx`
- Create: `web/src/routes/index.tsx`
- Modify: `web/vite.config.ts`

- [ ] **Step 1: Install deps**

```bash
pnpm -C web add @tanstack/react-router @tanstack/react-query \
  @tanstack/react-table @tanstack/react-virtual zustand sonner
pnpm -C web add -D @tanstack/router-plugin @tanstack/router-devtools \
  @tanstack/react-query-devtools
```

- [ ] **Step 2: Update `web/vite.config.ts` to include router plugin**

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { TanStackRouterVite } from "@tanstack/router-plugin/vite";
import path from "node:path";

export default defineConfig({
  plugins: [
    TanStackRouterVite({ routesDirectory: "src/routes", generatedRouteTree: "src/app/routeTree.gen.ts" }),
    react(),
  ],
  resolve: { alias: { "@": path.resolve(__dirname, "src") } },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/v1":     { target: "http://localhost:8080", changeOrigin: true },
      "/flight": { target: "http://localhost:8080", changeOrigin: true },
      "/health": { target: "http://localhost:8080", changeOrigin: true },
    },
  },
  build: { outDir: "dist", sourcemap: true, target: "es2022" },
});
```

- [ ] **Step 3: Write `web/src/routes/__root.tsx`**

```tsx
import { createRootRoute, Outlet } from "@tanstack/react-router";
import { Toaster } from "sonner";

export const Route = createRootRoute({
  component: () => (
    <>
      <Outlet />
      <Toaster richColors position="top-right" />
    </>
  ),
});
```

- [ ] **Step 4: Write `web/src/routes/index.tsx`**

```tsx
import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  beforeLoad: () => { throw redirect({ to: "/explore" }); },
});
```

- [ ] **Step 5: Write `web/src/app/router.tsx`**

```tsx
import { createRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";

export const router = createRouter({ routeTree, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register { router: typeof router; }
}
```

- [ ] **Step 6: Write `web/src/app/providers.tsx`**

```tsx
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { router } from "./router";

const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false } },
});

export function Providers() {
  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <ReactQueryDevtools />
    </QueryClientProvider>
  );
}
```

- [ ] **Step 7: Update `web/src/main.tsx`**

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import { Providers } from "./app/providers";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers />
  </React.StrictMode>,
);
```

- [ ] **Step 8: Temporarily delete `web/src/App.tsx`** — the explore route will replace it in Phase 5.

```bash
rm web/src/App.tsx
```

- [ ] **Step 9: Add placeholder `web/src/routes/explore.tsx`**

```tsx
import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/explore")({
  component: () => <div className="p-6">Explore (placeholder)</div>,
});
```

- [ ] **Step 10: Verify build + dev**

```bash
pnpm -C web build
pnpm -C web dev  # check http://localhost:5173/ redirects to /explore — Ctrl-C when verified
```

- [ ] **Step 11: Commit**

```bash
git add web/ pnpm-lock.yaml
git commit -m "feat(web): add router, query client, toast, zustand deps"
```

---

## Phase 2 — `kyma-web-assets` crate

### Task 2.1 — Create the embedded-assets crate

**Files:**
- Create: `crates/kyma-web-assets/Cargo.toml`
- Create: `crates/kyma-web-assets/src/lib.rs`
- Create: `crates/kyma-web-assets/build.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Write `crates/kyma-web-assets/Cargo.toml`**

```toml
[package]
name    = "kyma-web-assets"
version = "0.1.0"
edition = "2021"

[dependencies]
include_dir = "0.7"

[build-dependencies]
# empty — build.rs runs unconditionally, no deps
```

- [ ] **Step 2: Write `crates/kyma-web-assets/build.rs`** — cargo must rebuild when `web/dist/` changes.

```rust
fn main() {
    println!("cargo:rerun-if-changed=../../web/dist");
}
```

- [ ] **Step 3: Write `crates/kyma-web-assets/src/lib.rs`**

```rust
//! Embedded Web UI assets — compiled into the binary at build time.
//!
//! The caller (kyma-server) iterates these behind the `web-ui` feature.
#![forbid(unsafe_code)]

use include_dir::{include_dir, Dir};

/// The Web UI's `dist/` directory, baked in at compile time.
pub static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../web/dist");

/// MIME type for a filename, by extension. Falls back to `application/octet-stream`.
pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg"  => "image/svg+xml",
        "png"  => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ico"  => "image/x-icon",
        "map"  => "application/json",
        _ => "application/octet-stream",
    }
}
```

- [ ] **Step 4: Register crate in workspace `Cargo.toml`**

Modify the `members = [...]` list in the workspace root `Cargo.toml` to add `"crates/kyma-web-assets"`.

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p kyma-web-assets
```
Expected: clean compile. If `web/dist/` is empty, `include_dir!` still succeeds with zero entries — that's fine.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-web-assets Cargo.toml Cargo.lock
git commit -m "feat(server): add kyma-web-assets crate"
```

---

## Phase 3 — Static UI serving in `kyma-server`

### Task 3.1 — Add `web-ui` feature flag

**Files:**
- Modify: `crates/kyma-server/Cargo.toml`
- Modify: `crates/kyma-bin/Cargo.toml`

- [ ] **Step 1: Add feature to `crates/kyma-server/Cargo.toml`**

Add to `[dependencies]`:
```toml
kyma-web-assets = { path = "../kyma-web-assets", optional = true }
```

Add section:
```toml
[features]
default = []
web-ui  = ["dep:kyma-web-assets"]
```

- [ ] **Step 2: Add feature to `crates/kyma-bin/Cargo.toml`**

Update the `kyma-server` dep entry:
```toml
kyma-server = { path = "../kyma-server" }
```
Add:
```toml
[features]
default = []
web-ui  = ["kyma-server/web-ui"]
```

- [ ] **Step 3: Verify both builds**

```bash
cargo check -p kyma-bin
cargo check -p kyma-bin --features web-ui
```
Expected: both succeed (web-ui compiles in `kyma-web-assets`; default does not).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-server/Cargo.toml crates/kyma-bin/Cargo.toml Cargo.lock
git commit -m "feat(server): add web-ui feature flag"
```

### Task 3.2 — Static handler + SPA fallback

**Files:**
- Create: `crates/kyma-server/src/web_ui.rs`
- Modify: `crates/kyma-server/src/lib.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kyma-server/tests/web_assets.rs`:
```rust
//! Only compiles/runs when the `web-ui` feature is on.
#![cfg(feature = "web-ui")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for .oneshot

#[tokio::test]
async fn serves_index_at_root() {
    let app = kyma_server::web_ui::router();
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().starts_with("text/html"));
}

#[tokio::test]
async fn spa_fallback_to_index_for_unknown_path() {
    let app = kyma_server::web_ui::router();
    let req = Request::builder().uri("/some/client-route").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().starts_with("text/html"));
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test -p kyma-server --features web-ui --test web_assets
```
Expected: FAIL — `kyma_server::web_ui` doesn't exist yet.

- [ ] **Step 3: Write `crates/kyma-server/src/web_ui.rs`**

```rust
//! UI static-serving routes. Compiled in only when the `web-ui` feature is on.

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use kyma_web_assets::{mime_for, DIST};

/// Router exposing: `GET /`, `GET /assets/*`, and a SPA fallback.
pub fn router() -> Router {
    Router::new()
        .route("/",             get(serve_index))
        .route("/assets/*path", get(serve_asset))
        .fallback(serve_spa_fallback)
}

async fn serve_index() -> Response {
    match DIST.get_file("index.html") {
        Some(f) => html_response(f.contents()),
        None    => (StatusCode::NOT_FOUND, "index.html missing from embedded assets").into_response(),
    }
}

async fn serve_asset(uri: Uri) -> Response {
    // path strips the leading "/"
    let raw = uri.path().trim_start_matches('/');
    match DIST.get_file(raw) {
        Some(f) => asset_response(raw, f.contents()),
        None    => (StatusCode::NOT_FOUND, "asset not found").into_response(),
    }
}

/// Unknown paths (not `/v1/*`, `/flight/*`, `/health`, `/metrics`) fall back
/// to `index.html` so client-side routing works on reload / deep-link.
async fn serve_spa_fallback() -> Response {
    match DIST.get_file("index.html") {
        Some(f) => html_response(f.contents()),
        None    => (StatusCode::NOT_FOUND, "index.html missing").into_response(),
    }
}

fn html_response(body: &'static [u8]) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE,  HeaderValue::from_static("text/html; charset=utf-8"))
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .body(Body::from(body))
        .unwrap()
}

fn asset_response(path: &str, body: &'static [u8]) -> Response {
    let ct = mime_for(path);
    // Vite-style hashed filenames (contain a dash or dot followed by 8+ hex chars)
    // get immutable cache. `index.html` and bare names get no-cache.
    let is_hashed = path.contains('-')
        && path.rsplit(['-', '.']).next().map_or(false, |tail| {
            tail.len() >= 8 && tail.chars().all(|c| c.is_ascii_hexdigit() || c == 'j' || c == 's' || c == 'c' || c == '.')
        });
    let cache = if is_hashed { "public, max-age=31536000, immutable" } else { "no-cache" };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE,  HeaderValue::from_static(ct))
        .header(header::CACHE_CONTROL, HeaderValue::from_str(cache).unwrap())
        .body(Body::from(body))
        .unwrap()
}
```

- [ ] **Step 4: Expose module from `crates/kyma-server/src/lib.rs`**

Add near the other `mod` declarations:
```rust
#[cfg(feature = "web-ui")]
pub mod web_ui;
```

- [ ] **Step 5: Ensure the web build has happened before the test runs**

```bash
pnpm -C web build
cargo test -p kyma-server --features web-ui --test web_assets
```
Expected: PASS (both tests).

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/src/web_ui.rs crates/kyma-server/src/lib.rs \
  crates/kyma-server/tests/web_assets.rs
git commit -m "feat(server): static UI serving behind web-ui feature"
```

### Task 3.3 — Wire `web_ui::router()` into the top-level server

**Files:**
- Modify: `crates/kyma-server/src/lib.rs`

- [ ] **Step 1: Locate the top-level app-construction function**

Search:
```bash
grep -n "fn run\|fn serve\|fn build_app\|fn make_app" crates/kyma-server/src/lib.rs
```
Note the function name; it owns the final `Router` composition.

- [ ] **Step 2: Merge `web_ui::router()` into it**

Inside that function, after the existing auth-gated and health routers are composed, add:
```rust
#[cfg(feature = "web-ui")]
let app = app.merge(crate::web_ui::router());
```
The merged `web_ui::router()` has no auth layer — it's public by design (the app itself gates access via the Settings page).

- [ ] **Step 3: Confirm auth layer order does not gate `/`**

Inspect `crates/kyma-server/src/auth.rs` — the auth middleware must be applied only to the `/v1/*` and `/flight/*` sub-routers, never to the merged `web_ui::router()`. If the current code applies auth *globally*, refactor so auth is `.layer`ed onto only the `router(state)` sub-router that owns `/v1/query` et al.

Expected shape:
```rust
let api_router = kyma_server::router(state).layer(auth_layer(...));
let app = Router::new()
    .merge(api_router)
    .merge(kyma_server::health_router())
    .merge(kyma_server::metrics_router());
#[cfg(feature = "web-ui")]
let app = app.merge(kyma_server::web_ui::router());
```

- [ ] **Step 4: Smoke test**

```bash
pnpm -C web build
cargo run -p kyma-bin --features web-ui &
SERVER=$!
sleep 2
curl -sS -o /dev/null -w "%{http_code} %{content_type}\n" http://localhost:8080/
kill $SERVER
```
Expected: `200 text/html; charset=utf-8`.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/src/lib.rs
git commit -m "feat(server): merge web UI router under web-ui feature"
```

---

## Phase 4 — `/v1/catalog/schema` endpoint

### Task 4.1 — Extend the `Catalog` trait with schema-listing methods

**Files:**
- Modify: `crates/kyma-core/src/catalog.rs`
- Modify: `crates/kyma-catalog/src/lib.rs`

- [ ] **Step 1: Add trait methods in `crates/kyma-core/src/catalog.rs`**

Add to the `Catalog` trait (alongside existing methods):
```rust
/// Lightweight schema tree for the UI.
async fn list_databases(&self)
    -> Result<Vec<String>, CatalogError>;

async fn list_tables(&self, database: &str)
    -> Result<Vec<String>, CatalogError>;

async fn get_table_columns(&self, database: &str, table: &str)
    -> Result<Vec<ColumnInfo>, CatalogError>;
```

In the same file, define `ColumnInfo`:
```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnInfo {
    pub name: String,
    /// `string`, `int`, `long`, `real`, `datetime`, `bool`, `dynamic`.
    pub r#type: String,
    pub nullable: bool,
}
```

- [ ] **Step 2: Add placeholder impls in test doubles**

Any in-memory `MockCatalog` in test-only code should implement the new methods returning empty vecs. Search:
```bash
grep -rn "impl Catalog for" crates --include="*.rs"
```
Add three methods to each impl; use `todo!()` *only* if the file is covered by unrelated tests — prefer returning `Ok(vec![])`.

- [ ] **Step 3: Verify trait compiles**

```bash
cargo check -p kyma-core
```
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-core/src/catalog.rs crates/**/src/*.rs
git commit -m "feat(catalog): schema-listing trait methods"
```

### Task 4.2 — Implement schema-listing in `PostgresCatalog`

**Files:**
- Modify: `crates/kyma-catalog/src/lib.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kyma-catalog/tests/catalog_schema_it.rs`:
```rust
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

#[tokio::test]
async fn lists_databases_tables_columns() {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@localhost:{port}/postgres");
    let catalog = PostgresCatalog::connect(&url).await.unwrap();
    catalog.run_migrations().await.unwrap();

    catalog.create_database("obs").await.unwrap();
    catalog.create_table("obs", "otel_logs", serde_json::json!({
        "columns": [
            { "name": "timestamp",     "type": "datetime", "nullable": false },
            { "name": "service.name",  "type": "string",   "nullable": false },
            { "name": "severity_text", "type": "string",   "nullable": true  }
        ]
    })).await.unwrap();

    assert_eq!(catalog.list_databases().await.unwrap(), vec!["obs"]);
    assert_eq!(catalog.list_tables("obs").await.unwrap(), vec!["otel_logs"]);
    let cols = catalog.get_table_columns("obs", "otel_logs").await.unwrap();
    assert_eq!(cols.len(), 3);
    assert_eq!(cols[0].name, "timestamp");
    assert_eq!(cols[0].r#type, "datetime");
    assert!(!cols[0].nullable);
}
```

Note: adapt the `create_database` / `create_table` calls to whatever signatures currently exist in `PostgresCatalog`. If they don't exist, the plan's assumption is that the migration-002 era tests in `crates/kyma-catalog/tests/catalog_it.rs` already create fixture data; reuse the same fixture helpers in this new test.

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p kyma-catalog --test catalog_schema_it
```
Expected: compile error — new trait methods unimplemented on `PostgresCatalog`.

- [ ] **Step 3: Implement on `PostgresCatalog`**

In `crates/kyma-catalog/src/lib.rs`, add an `impl Catalog` block (or extend the existing one):
```rust
async fn list_databases(&self) -> Result<Vec<String>, CatalogError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM databases ORDER BY name ASC",
    )
    .fetch_all(&self.pool)
    .await
    .map_err(CatalogError::from)?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

async fn list_tables(&self, database: &str) -> Result<Vec<String>, CatalogError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT t.name FROM tables t
         JOIN databases d ON d.id = t.database_id
         WHERE d.name = $1
         ORDER BY t.name ASC",
    )
    .bind(database)
    .fetch_all(&self.pool)
    .await
    .map_err(CatalogError::from)?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

async fn get_table_columns(
    &self,
    database: &str,
    table: &str,
) -> Result<Vec<kyma_core::catalog::ColumnInfo>, CatalogError> {
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT s.schema_json
         FROM tables t
         JOIN databases d ON d.id = t.database_id
         JOIN schema_snapshots s ON s.id = t.schema_snapshot_id
         WHERE d.name = $1 AND t.name = $2",
    )
    .bind(database)
    .bind(table)
    .fetch_one(&self.pool)
    .await
    .map_err(CatalogError::from)?;

    #[derive(serde::Deserialize)]
    struct SchemaJson { columns: Vec<kyma_core::catalog::ColumnInfo> }
    let parsed: SchemaJson = serde_json::from_value(row.0)
        .map_err(|e| CatalogError::Internal(e.to_string()))?;
    Ok(parsed.columns)
}
```

Adapt table/column names to the real DDL (check `crates/kyma-catalog/migrations/*.sql`).

- [ ] **Step 4: Run test to verify pass**

```bash
cargo test -p kyma-catalog --test catalog_schema_it
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-catalog/src/lib.rs crates/kyma-catalog/tests/catalog_schema_it.rs
git commit -m "feat(catalog): list databases/tables/columns in PostgresCatalog"
```

### Task 4.3 — `GET /v1/catalog/schema` handler with 5 s cache

**Files:**
- Create: `crates/kyma-server/src/catalog_handler.rs`
- Modify: `crates/kyma-server/src/lib.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kyma-server/tests/catalog_http.rs`:
```rust
use axum::http::{Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt;
use serde_json::Value;

#[tokio::test]
async fn returns_schema_tree() {
    // fixture: reuse the same catalog + state builder the query tests use
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let app = kyma_server::router(state); // (this is the existing query router; we will merge the catalog route into it)

    let req = Request::builder()
        .uri("/v1/catalog/schema")
        .header("authorization", "Bearer test-read-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["databases"][0]["name"], "obs");
    assert_eq!(v["databases"][0]["tables"][0]["name"], "otel_logs");
    assert!(v["databases"][0]["tables"][0]["columns"].is_array());
}

#[tokio::test]
async fn requires_read_role() {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let app = kyma_server::router(state);
    let req = Request::builder()
        .uri("/v1/catalog/schema")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

Add `pub mod test_support` in `kyma-server/src/lib.rs` (behind `#[cfg(any(test, feature = "test-support"))]`) exposing `seeded_state_with_obs_otel_logs()` — a helper that builds a `QueryState` against a testcontainers Postgres + MinIO fixture and calls the new catalog methods from Task 4.1 to seed data.

- [ ] **Step 2: Run to confirm failure**

```bash
cargo test -p kyma-server --test catalog_http
```
Expected: FAIL — route not registered.

- [ ] **Step 3: Write `crates/kyma-server/src/catalog_handler.rs`**

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::QueryState;
use kyma_core::catalog::{Catalog, ColumnInfo};

#[derive(Serialize)]
pub struct SchemaDoc { pub databases: Vec<DatabaseDoc> }

#[derive(Serialize)]
pub struct DatabaseDoc { pub name: String, pub tables: Vec<TableDoc> }

#[derive(Serialize)]
pub struct TableDoc { pub name: String, pub columns: Vec<ColumnInfo> }

const TTL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct SchemaCache {
    inner: Mutex<Option<(Instant, Arc<SchemaDoc>)>>,
}

impl SchemaCache {
    pub fn new() -> Self { Self::default() }
}

pub async fn schema_handler(State(state): State<QueryState>) -> impl IntoResponse {
    let cache = state.schema_cache.clone();
    let mut guard = cache.inner.lock().await;
    if let Some((t, doc)) = guard.as_ref() {
        if t.elapsed() < TTL {
            return (StatusCode::OK, Json((**doc).clone())).into_response();
        }
    }

    let doc = match build(&*state.catalog).await {
        Ok(d) => Arc::new(d),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error":{"code":"catalog","message": e.to_string()}})),
            )
                .into_response()
        }
    };
    *guard = Some((Instant::now(), doc.clone()));
    (StatusCode::OK, Json((*doc).clone())).into_response()
}

async fn build(catalog: &dyn Catalog) -> Result<SchemaDoc, kyma_core::errors::CatalogError> {
    let mut out = SchemaDoc { databases: Vec::new() };
    for db in catalog.list_databases().await? {
        let mut tables = Vec::new();
        for tbl in catalog.list_tables(&db).await? {
            let cols = catalog.get_table_columns(&db, &tbl).await?;
            tables.push(TableDoc { name: tbl, columns: cols });
        }
        out.databases.push(DatabaseDoc { name: db, tables });
    }
    Ok(out)
}
```

Note `SchemaDoc` needs `Clone` via `#[derive(Clone)]` on all three structs. Add that.

- [ ] **Step 4: Add `SchemaCache` to `QueryState`**

Modify the existing `QueryState` struct (likely in `crates/kyma-server/src/lib.rs`) to add:
```rust
pub schema_cache: std::sync::Arc<crate::catalog_handler::SchemaCache>,
```
Initialize it where `QueryState` is constructed (`Arc::new(SchemaCache::new())`).

- [ ] **Step 5: Register route**

In `crates/kyma-server/src/lib.rs`, extend the `router(state: QueryState)` function to add:
```rust
.route("/v1/catalog/schema", axum::routing::get(catalog_handler::schema_handler))
```
Add `pub mod catalog_handler;` at the top.

- [ ] **Step 6: Run tests to verify pass**

```bash
cargo test -p kyma-server --test catalog_http
```
Expected: both tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-server/src/catalog_handler.rs crates/kyma-server/src/lib.rs \
  crates/kyma-server/tests/catalog_http.rs
git commit -m "feat(server): /v1/catalog/schema with 5s cache"
```

---

## Phase 5 — `tonic-web` middleware on Flight

### Task 5.1 — Expose Flight over `/flight/*` on the HTTP listener

**Files:**
- Modify: `crates/kyma-server/Cargo.toml`
- Modify: `crates/kyma-server/src/flight.rs` or equivalent
- Modify: `crates/kyma-server/src/lib.rs`

- [ ] **Step 1: Add dep**

In `crates/kyma-server/Cargo.toml`, add:
```toml
tonic-web = "0.12"
```

- [ ] **Step 2: Write a gRPC-web smoke test**

Create `crates/kyma-server/tests/flight_web_smoke.rs`:
```rust
//! Minimal gRPC-web smoke test — sends a Flight DoGet over HTTP/1.1 and
//! asserts we get Arrow-framed body back.
#![cfg(feature = "web-ui")]

use bytes::{BufMut, BytesMut};

#[tokio::test]
async fn grpc_web_do_get_returns_arrow_stream() {
    let server = kyma_server::test_support::start_test_server_with_seeded_data().await;
    let base = server.http_base_url(); // e.g. http://127.0.0.1:PORT
    let ticket_json = serde_json::json!({
        "database": "obs",
        "query": "otel_logs | take 1",
        "language": "kql"
    }).to_string();

    // Flight.Ticket protobuf: { bytes ticket = 1 }. Field 1, wire type 2 (length-delimited).
    let mut proto = BytesMut::new();
    proto.put_u8(0x0a); // tag: field=1, wire=2
    prost::encoding::encode_varint(ticket_json.len() as u64, &mut proto);
    proto.extend_from_slice(ticket_json.as_bytes());

    // gRPC-web frame: [0x00][len u32 BE][proto]
    let mut frame = BytesMut::with_capacity(5 + proto.len());
    frame.put_u8(0x00);
    frame.put_u32(proto.len() as u32);
    frame.extend_from_slice(&proto);

    let client = reqwest::Client::builder().http1_only().build().unwrap();
    let resp = client.post(format!("{base}/flight/arrow.flight.protocol.FlightService/DoGet"))
        .header("authorization", "Bearer test-read-token")
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(frame.freeze())
        .send().await.unwrap();

    assert!(resp.status().is_success(), "status: {}", resp.status());
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/grpc-web+proto",
    );
    let body = resp.bytes().await.unwrap();
    assert!(body.len() > 5, "expected at least one gRPC-web frame; got {} bytes", body.len());
    server.shutdown().await;
}
```

Add `prost = "0.13"` and `bytes = "1"` + `reqwest = {...}` to `[dev-dependencies]` of `kyma-server` if not present.

- [ ] **Step 3: Run to confirm failure**

```bash
pnpm -C web build  # dist/ must exist for web-ui feature
cargo test -p kyma-server --features web-ui --test flight_web_smoke
```
Expected: FAIL — `/flight/*` not mounted on the HTTP listener.

- [ ] **Step 4: Wrap `FlightServiceServer` with `tonic_web::enable` and merge into axum**

In the HTTP app-construction function (same place Task 3.3 touched), add:
```rust
use tonic_web::GrpcWebLayer;
use tower::ServiceBuilder;
use arrow_flight::flight_service_server::FlightServiceServer;

let flight_svc = FlightServiceServer::new(state.flight_service.clone());
// IMPORTANT: auth must apply to /flight/* the same way it applies to /v1/*.
// Layer order: auth_layer (outer) → GrpcWebLayer → FlightServiceServer.
let flight_web = ServiceBuilder::new()
    .layer(auth_layer(state.auth.clone()))   // same middleware used for /v1/*
    .layer(GrpcWebLayer::new())
    .service(flight_svc);

let app = app.nest_service("/flight", flight_web);
```

`state.auth` is the existing auth config (`KYMA_AUTH_TOKENS` + role table). If `auth_layer` is currently a function that accepts the whole `QueryState`, use that same signature. If auth is today only applied via `.layer(...)` on the API `Router`, factor it out into a standalone `tower::Layer` constructor so it can be composed onto both the axum router *and* this nested tonic service — the two routes must enforce auth identically.

The native `tonic::transport::Server` on `:9090` remains unchanged; we're merely *also* exposing Flight over `:8080/flight/*`.

- [ ] **Step 5: Run test to verify pass**

```bash
cargo test -p kyma-server --features web-ui --test flight_web_smoke
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-server/Cargo.toml crates/kyma-server/src/lib.rs \
  crates/kyma-server/tests/flight_web_smoke.rs Cargo.lock
git commit -m "feat(server): expose Flight over gRPC-web at /flight/*"
```

---

## Phase 6 — Frontend SDK

### Task 6.1 — Vendor Flight proto and set up buf codegen

**Files:**
- Create: `web/proto/Flight.proto`
- Create: `web/buf.yaml`
- Create: `web/buf.gen.yaml`
- Modify: `web/package.json`
- Modify: `web/.gitignore`

- [ ] **Step 1: Install codegen deps**

```bash
pnpm -C web add @bufbuild/protobuf@2.2.0 @connectrpc/connect@2.0.1 @connectrpc/connect-web@2.0.1
pnpm -C web add -D @bufbuild/protoc-gen-es@2.2.0 @connectrpc/protoc-gen-connect-es@2.0.1
```

- [ ] **Step 2: Vendor `web/proto/Flight.proto`**

Copy the canonical Arrow Flight proto (Apache Arrow 17 release) into `web/proto/Flight.proto`:
```bash
curl -fsSL https://raw.githubusercontent.com/apache/arrow/release-17.0.0/format/Flight.proto \
  -o web/proto/Flight.proto
```
Verify the file's package line is `package arrow.flight.protocol;`.

- [ ] **Step 3: Write `web/buf.yaml`**

```yaml
version: v2
modules:
  - path: proto
lint:
  use: [DEFAULT]
  except: [PACKAGE_VERSION_SUFFIX, RPC_REQUEST_STANDARD_NAME, RPC_RESPONSE_STANDARD_NAME]
```

- [ ] **Step 4: Write `web/buf.gen.yaml`**

```yaml
version: v2
plugins:
  - local: ["pnpm", "exec", "protoc-gen-es"]
    out: src/sdk/gen
    opt: ["target=ts"]
  - local: ["pnpm", "exec", "protoc-gen-connect-es"]
    out: src/sdk/gen
    opt: ["target=ts"]
inputs:
  - directory: proto
```

- [ ] **Step 5: Add generated-files ignore**

Append to `web/.gitignore`:
```
src/sdk/gen
```

- [ ] **Step 6: Run codegen**

```bash
pnpm -C web exec buf lint
pnpm -C web exec buf generate
ls web/src/sdk/gen
```
Expected: `Flight_pb.ts`, `Flight_connect.ts` (or similar) exist.

- [ ] **Step 7: Commit**

```bash
git add web/proto web/buf.yaml web/buf.gen.yaml web/.gitignore web/package.json pnpm-lock.yaml
git commit -m "feat(web): vendor Flight.proto + buf codegen"
```

### Task 6.2 — Session store (zustand) with localStorage

**Files:**
- Create: `web/src/sdk/session.ts`
- Create: `web/src/sdk/session.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/sdk/session.test.ts
import { beforeEach, expect, test } from "vitest";
import { useSession } from "./session";

beforeEach(() => { localStorage.clear(); useSession.getState().reset(); });

test("stores and retrieves endpoint + token", () => {
  useSession.getState().set({ endpoint: "http://localhost:8080", token: "abc" });
  const s = useSession.getState();
  expect(s.endpoint).toBe("http://localhost:8080");
  expect(s.token).toBe("abc");
});

test("is configured only when both endpoint and token are present", () => {
  expect(useSession.getState().isConfigured()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "" });
  expect(useSession.getState().isConfigured()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "t" });
  expect(useSession.getState().isConfigured()).toBe(true);
});
```

- [ ] **Step 2: Run to confirm failure**

```bash
pnpm -C web test src/sdk/session.test.ts
```
Expected: FAIL (module missing).

- [ ] **Step 3: Write `web/src/sdk/session.ts`**

```ts
import { create } from "zustand";
import { persist } from "zustand/middleware";

export type SessionState = {
  endpoint: string;
  token: string;
  database: string;
  set: (p: Partial<Pick<SessionState, "endpoint" | "token" | "database">>) => void;
  reset: () => void;
  isConfigured: () => boolean;
};

export const useSession = create<SessionState>()(
  persist(
    (set, get) => ({
      endpoint: "",
      token: "",
      database: "obs",
      set: (p) => set(p),
      reset: () => set({ endpoint: "", token: "", database: "obs" }),
      isConfigured: () => {
        const { endpoint, token } = get();
        return endpoint.length > 0 && token.length > 0;
      },
    }),
    { name: "kyma.session" },
  ),
);
```

- [ ] **Step 4: Run tests**

```bash
pnpm -C web test src/sdk/session.test.ts
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/sdk/session.ts web/src/sdk/session.test.ts
git commit -m "feat(web): session store with localStorage persistence"
```

### Task 6.3 — Catalog client

**Files:**
- Create: `web/src/sdk/catalog.ts`
- Create: `web/src/sdk/catalog.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/sdk/catalog.test.ts
import { beforeEach, describe, expect, test, vi } from "vitest";
import { fetchSchema, type SchemaDoc } from "./catalog";

const mockFetch = vi.fn();
beforeEach(() => { vi.stubGlobal("fetch", mockFetch); mockFetch.mockReset(); });

test("fetches and returns the schema tree", async () => {
  const doc: SchemaDoc = {
    databases: [{ name: "obs", tables: [{ name: "otel_logs",
      columns: [{ name: "timestamp", type: "datetime", nullable: false }] }] }],
  };
  mockFetch.mockResolvedValue(new Response(JSON.stringify(doc), {
    status: 200, headers: { "content-type": "application/json" },
  }));
  const got = await fetchSchema({ endpoint: "http://localhost:8080", token: "t" });
  expect(got).toEqual(doc);
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:8080/v1/catalog/schema",
    expect.objectContaining({ headers: expect.objectContaining({ authorization: "Bearer t" }) }),
  );
});

test("throws Unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(fetchSchema({ endpoint: "http://x", token: "t" })).rejects.toThrow(/unauthorized/i);
});
```

- [ ] **Step 2: Confirm failure**

```bash
pnpm -C web test src/sdk/catalog.test.ts
```
Expected: FAIL.

- [ ] **Step 3: Write `web/src/sdk/catalog.ts`**

```ts
export interface ColumnInfo { name: string; type: string; nullable: boolean; }
export interface TableDoc  { name: string; columns: ColumnInfo[]; }
export interface DatabaseDoc { name: string; tables: TableDoc[]; }
export interface SchemaDoc { databases: DatabaseDoc[]; }

export async function fetchSchema(args: { endpoint: string; token: string }): Promise<SchemaDoc> {
  const res = await fetch(`${args.endpoint.replace(/\/$/, "")}/v1/catalog/schema`, {
    headers: { authorization: `Bearer ${args.token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (!res.ok) throw new Error(`catalog fetch failed: ${res.status}`);
  return (await res.json()) as SchemaDoc;
}
```

- [ ] **Step 4: Verify pass**

```bash
pnpm -C web test src/sdk/catalog.test.ts
```

- [ ] **Step 5: Commit**

```bash
git add web/src/sdk/catalog.ts web/src/sdk/catalog.test.ts
git commit -m "feat(web): catalog client"
```

### Task 6.4 — Flight gRPC-web query client

**Files:**
- Create: `web/src/sdk/query.ts`
- Create: `web/src/sdk/query.test.ts`

- [ ] **Step 1: Install Arrow runtime**

```bash
pnpm -C web add apache-arrow@17.0.0
```

- [ ] **Step 2: Write the failing test**

```ts
// web/src/sdk/query.test.ts — smoke-level; integration test covered in E2E.
import { expect, test } from "vitest";
import { encodeTicket } from "./query";

test("encodeTicket serializes database+query+language as JSON", () => {
  const bytes = encodeTicket({ database: "obs", query: "otel_logs | take 1", language: "kql" });
  const text = new TextDecoder().decode(bytes);
  const json = JSON.parse(text);
  expect(json).toEqual({ database: "obs", query: "otel_logs | take 1", language: "kql" });
});
```

- [ ] **Step 3: Confirm failure**

```bash
pnpm -C web test src/sdk/query.test.ts
```

- [ ] **Step 4: Write `web/src/sdk/query.ts`**

```ts
import { createGrpcWebTransport } from "@connectrpc/connect-web";
import { createClient } from "@connectrpc/connect";
import { FlightService } from "./gen/Flight_connect";
import { Ticket, FlightData } from "./gen/Flight_pb";
import { tableFromIPC, RecordBatch } from "apache-arrow";

export type QueryArgs = {
  endpoint: string;
  token: string;
  database: string;
  query: string;
  language: "kql" | "sql";
  walMs?: number;
  memBytes?: number;
  signal?: AbortSignal;
};

export function encodeTicket(t: { database: string; query: string; language: string }): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(t));
}

export async function* runQuery(args: QueryArgs): AsyncGenerator<RecordBatch, void, void> {
  const transport = createGrpcWebTransport({ baseUrl: args.endpoint.replace(/\/$/, "") });
  const client = createClient(FlightService, transport);

  const ticket = new Ticket({ ticket: encodeTicket({
    database: args.database, query: args.query, language: args.language,
  }) });

  const headers: Record<string, string> = { authorization: `Bearer ${args.token}` };
  if (args.walMs)    headers["x-kyma-max-wall-clock-ms"] = String(args.walMs);
  if (args.memBytes) headers["x-kyma-max-memory-bytes"]  = String(args.memBytes);

  const stream = client.doGet(ticket, { headers, signal: args.signal });

  // Each FlightData message carries a chunk of the Arrow IPC stream in .dataBody.
  // Concatenate and decode progressively — but because Arrow's StreamReader needs
  // the cumulative buffer, we reassemble then decode once completion signal or
  // a RecordBatch boundary is likely. For the MVP we buffer all messages and
  // decode the full stream in one shot; incremental decode is a perf follow-up.
  const chunks: Uint8Array[] = [];
  for await (const msg of stream as AsyncIterable<FlightData>) {
    if (msg.dataHeader.length) chunks.push(msg.dataHeader);
    if (msg.dataBody.length)   chunks.push(msg.dataBody);
  }
  const full = concat(chunks);
  const table = tableFromIPC(full);
  for (const batch of table.batches) yield batch;
}

function concat(xs: Uint8Array[]): Uint8Array {
  const total = xs.reduce((n, x) => n + x.length, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const x of xs) { out.set(x, o); o += x.length; }
  return out;
}
```

Note: Flight's `DoGet` response is a *stream* of `FlightData` — the first message carries the schema (`data_header`), subsequent ones carry `RecordBatch`es. The MVP buffers then decodes; incremental streaming is a Phase-F perf follow-up noted in the spec's Open Questions.

- [ ] **Step 5: Verify unit test passes**

```bash
pnpm -C web test src/sdk/query.test.ts
```

- [ ] **Step 6: Typecheck the whole web project**

```bash
pnpm -C web typecheck
```
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add web/src/sdk/query.ts web/src/sdk/query.test.ts web/package.json pnpm-lock.yaml
git commit -m "feat(web): Flight gRPC-web query client"
```

### Task 6.5 — Arrow → grid / chart adapter

**Files:**
- Create: `web/src/sdk/arrow.ts`
- Create: `web/src/sdk/arrow.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/sdk/arrow.test.ts
import { expect, test } from "vitest";
import { tableFromArrays, Utf8, Int32, Timestamp, TimeUnit } from "apache-arrow";
import { batchToRows, inferColumnTypes, autoChartAxes } from "./arrow";

test("batchToRows extracts columns + rows", () => {
  const table = tableFromArrays({
    service: ["a", "b"],
    n: [1, 2],
  });
  const { columns, rows } = batchToRows(table.batches[0]);
  expect(columns.map((c) => c.name)).toEqual(["service", "n"]);
  expect(rows).toEqual([{ service: "a", n: 1 }, { service: "b", n: 2 }]);
});

test("autoChartAxes picks line for time + numeric", () => {
  const cols = [
    { name: "ts", kind: "time"    as const },
    { name: "n",  kind: "numeric" as const },
  ];
  expect(autoChartAxes(cols)).toEqual({ type: "line", x: "ts", y: "n" });
});

test("autoChartAxes picks bar for string + numeric", () => {
  const cols = [
    { name: "svc", kind: "string"  as const },
    { name: "n",   kind: "numeric" as const },
  ];
  expect(autoChartAxes(cols)).toEqual({ type: "bar", x: "svc", y: "n" });
});
```

- [ ] **Step 2: Confirm failure**

```bash
pnpm -C web test src/sdk/arrow.test.ts
```

- [ ] **Step 3: Write `web/src/sdk/arrow.ts`**

```ts
import type { RecordBatch, Field } from "apache-arrow";
import { DataType } from "apache-arrow";

export type ColKind = "time" | "numeric" | "string" | "bool" | "other";
export type Column = { name: string; kind: ColKind };

export type ChartSpec =
  | { type: "line"    ; x: string; y: string }
  | { type: "bar"     ; x: string; y: string }
  | { type: "scatter" ; x: string; y: string }
  | { type: "stat"    ; y: string }
  | { type: "none"    };

export function columnKind(f: Field): ColKind {
  const t = f.type;
  if (DataType.isTimestamp(t) || DataType.isDate(t))      return "time";
  if (DataType.isInt(t) || DataType.isFloat(t) || DataType.isDecimal(t)) return "numeric";
  if (DataType.isUtf8(t) || DataType.isLargeUtf8(t))      return "string";
  if (DataType.isBool(t))                                  return "bool";
  return "other";
}

export function inferColumnTypes(batch: RecordBatch): Column[] {
  return batch.schema.fields.map((f) => ({ name: f.name, kind: columnKind(f) }));
}

export function batchToRows(batch: RecordBatch): { columns: Column[]; rows: Record<string, unknown>[] } {
  const columns = inferColumnTypes(batch);
  const rows: Record<string, unknown>[] = [];
  for (let i = 0; i < batch.numRows; i++) {
    const row: Record<string, unknown> = {};
    for (const c of columns) {
      const v = batch.getChild(c.name)?.get(i);
      row[c.name] = v instanceof Date ? v.toISOString() : v;
    }
    rows.push(row);
  }
  return { columns, rows };
}

export function autoChartAxes(cols: Column[]): ChartSpec {
  const time    = cols.find((c) => c.kind === "time");
  const numerics = cols.filter((c) => c.kind === "numeric");
  const strings  = cols.filter((c) => c.kind === "string");
  if (time && numerics[0])          return { type: "line",    x: time.name,       y: numerics[0].name };
  if (strings[0] && numerics[0])    return { type: "bar",     x: strings[0].name, y: numerics[0].name };
  if (numerics.length >= 2)         return { type: "scatter", x: numerics[0].name, y: numerics[1].name };
  if (numerics.length === 1 && cols.length === 1) return { type: "stat", y: numerics[0].name };
  return { type: "none" };
}
```

- [ ] **Step 4: Verify pass**

```bash
pnpm -C web test src/sdk
```
Expected: all SDK tests PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/sdk/arrow.ts web/src/sdk/arrow.test.ts
git commit -m "feat(web): Arrow-to-grid/chart adapter"
```

---

## Phase 7 — Shell, router, settings page

### Task 7.1 — Shell component (top bar + outlet)

**Files:**
- Create: `web/src/app/shell.tsx`
- Create: `web/src/routes/_app.tsx`
- Modify: `web/src/routes/explore.tsx`

- [ ] **Step 1: Write `web/src/app/shell.tsx`**

```tsx
import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { Database, Settings as SettingsIcon, Compass } from "lucide-react";
import { useSession } from "@/sdk/session";

export function Shell() {
  const { endpoint, database } = useSession();
  const active = useRouterState().location.pathname;
  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex h-12 items-center gap-4 border-b px-4 text-sm">
        <div className="font-semibold tracking-tight">kyma</div>
        <div className="flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs">
          <Database className="h-3.5 w-3.5" /> {database || "—"}
        </div>
        <nav className="ml-auto flex items-center gap-1">
          <Link to="/explore" className={btn(active.startsWith("/explore"))}>
            <Compass className="h-4 w-4" /> Explore
          </Link>
          <Link to="/settings" className={btn(active === "/settings")}>
            <SettingsIcon className="h-4 w-4" /> Settings
          </Link>
        </nav>
        <div className="ml-2 max-w-[22ch] truncate text-xs text-muted-foreground">{endpoint || "no server"}</div>
      </header>
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
    </div>
  );
}

const btn = (on: boolean) =>
  `inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs transition ${
    on ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/60"
  }`;
```

- [ ] **Step 2: Write `web/src/routes/_app.tsx`** (pathless layout wrapping all authenticated routes)

```tsx
import { createFileRoute, redirect } from "@tanstack/react-router";
import { Shell } from "@/app/shell";
import { useSession } from "@/sdk/session";

export const Route = createFileRoute("/_app")({
  beforeLoad: ({ location }) => {
    const s = useSession.getState();
    if (!s.isConfigured() && location.pathname !== "/settings") {
      throw redirect({ to: "/settings", search: { next: location.pathname } });
    }
  },
  component: Shell,
});
```

- [ ] **Step 3: Move `explore.tsx` under `_app`**

```bash
mv web/src/routes/explore.tsx web/src/routes/_app.explore.tsx
```

Then update its file contents:
```tsx
import { createFileRoute } from "@tanstack/react-router";
export const Route = createFileRoute("/_app/explore")({
  component: () => <div className="p-6 text-sm text-muted-foreground">Explore route — workspace lands here in Phase 8.</div>,
});
```

- [ ] **Step 4: Regenerate route tree + build**

```bash
pnpm -C web dev  # TanStack plugin regenerates routeTree.gen.ts on file change — Ctrl-C when it reports ready
pnpm -C web build
```

- [ ] **Step 5: Commit**

```bash
git add web/src/app/shell.tsx web/src/routes
git commit -m "feat(web): shell component + _app layout with auth redirect"
```

### Task 7.2 — Settings page (token paste)

**Files:**
- Create: `web/src/routes/settings.tsx`
- Create: `web/src/components/ui/button.tsx`
- Create: `web/src/components/ui/input.tsx`
- Create: `web/src/components/ui/label.tsx`

- [ ] **Step 1: Install shadcn primitives via CLI**

```bash
pnpm -C web dlx shadcn@2.1.0 add button input label card
```
Expected: files created under `web/src/components/ui/`.

- [ ] **Step 2: Write `web/src/routes/settings.tsx`**

```tsx
import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useSession } from "@/sdk/session";

export const Route = createFileRoute("/settings")({
  validateSearch: (s: Record<string, unknown>) => ({ next: typeof s.next === "string" ? s.next : "/explore" }),
  component: Settings,
});

function Settings() {
  const session = useSession();
  const { next } = useSearch({ from: "/settings" });
  const navigate = useNavigate();
  const [endpoint, setEndpoint] = useState(session.endpoint || "http://localhost:8080");
  const [token, setToken]       = useState(session.token);
  const [database, setDatabase] = useState(session.database || "obs");

  const save = async () => {
    try {
      const ping = await fetch(`${endpoint.replace(/\/$/, "")}/health`);
      if (!ping.ok) throw new Error(`health ${ping.status}`);
    } catch (e) {
      toast.error(`Can't reach ${endpoint}: ${(e as Error).message}`);
      return;
    }
    session.set({ endpoint: endpoint.replace(/\/$/, ""), token, database });
    toast.success("Saved. Connected to kyma.");
    navigate({ to: next });
  };

  return (
    <div className="mx-auto max-w-xl p-6">
      <Card>
        <CardHeader><CardTitle>Connect to kyma</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="endpoint">Server URL</Label>
            <Input id="endpoint" value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="http://localhost:8080" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="token">Bearer token</Label>
            <Input id="token" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="paste token from KYMA_AUTH_TOKENS" />
            <p className="text-xs text-muted-foreground">
              Configure with <code className="font-mono">KYMA_AUTH_TOKENS=token:read</code> on the server; leave blank
              and set <code>KYMA_AUTH_DISABLED=1</code> for unauthenticated dev.
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="database">Default database</Label>
            <Input id="database" value={database} onChange={(e) => setDatabase(e.target.value)} placeholder="obs" />
          </div>
          <div className="flex gap-2 pt-2">
            <Button onClick={save}>Save + connect</Button>
            <Button variant="ghost" onClick={() => session.reset()}>Reset</Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
```

- [ ] **Step 3: Build + smoke test**

```bash
pnpm -C web build
pnpm -C web dev  # open http://localhost:5173/settings, fill, save
```

- [ ] **Step 4: Commit**

```bash
git add web/src/routes/settings.tsx web/src/components/ui
git commit -m "feat(web): settings page with server URL + token + DB"
```

---

## Phase 8 — Query workspace core

### Task 8.1 — Workspace zustand store

**Files:**
- Create: `web/src/features/tabs/workspace-store.ts`
- Create: `web/src/features/tabs/workspace-store.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/features/tabs/workspace-store.test.ts
import { beforeEach, expect, test } from "vitest";
import { useWorkspace } from "./workspace-store";

beforeEach(() => { localStorage.clear(); useWorkspace.getState().resetAll(); });

test("newTab creates a fresh tab with empty query", () => {
  const id = useWorkspace.getState().newTab();
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(t.query).toBe("");
  expect(t.timeRange.preset).toBe("1h");
});

test("setQuery updates only the target tab", () => {
  const a = useWorkspace.getState().newTab();
  const b = useWorkspace.getState().newTab();
  useWorkspace.getState().setQuery(a, "foo | take 1");
  const { tabs } = useWorkspace.getState();
  expect(tabs.find((t) => t.id === a)!.query).toBe("foo | take 1");
  expect(tabs.find((t) => t.id === b)!.query).toBe("");
});

test("closeTab removes the tab and picks a new active when needed", () => {
  const a = useWorkspace.getState().newTab();
  const b = useWorkspace.getState().newTab();
  useWorkspace.getState().setActive(b);
  useWorkspace.getState().closeTab(b);
  expect(useWorkspace.getState().activeId).toBe(a);
});
```

- [ ] **Step 2: Confirm failure**

```bash
pnpm -C web test src/features/tabs/workspace-store.test.ts
```

- [ ] **Step 3: Write `web/src/features/tabs/workspace-store.ts`**

```ts
import { create } from "zustand";
import { persist } from "zustand/middleware";

export type TimeRangePreset = "5m" | "15m" | "1h" | "6h" | "24h" | "7d" | "30d" | "custom";
export type TimeRange = { preset: TimeRangePreset; from?: string; to?: string };

export type ResultsState =
  | { kind: "idle" }
  | { kind: "running"; startedAt: number }
  | { kind: "ok"; rowCount: number; bytes: number; durationMs: number; finishedAt: number }
  | { kind: "error"; message: string; requestId?: string; code?: string };

export type ChartConfig = {
  override?: { type: "line" | "bar" | "scatter" | "stat"; x?: string; y?: string };
};

export type Tab = {
  id: string;
  title: string;
  query: string;
  timeRange: TimeRange;
  results: ResultsState;
  chart: ChartConfig;
  dirty: boolean;
};

type Store = {
  tabs: Tab[];
  activeId: string | null;
  newTab: (seed?: Partial<Tab>) => string;
  closeTab: (id: string) => void;
  setActive: (id: string) => void;
  setQuery: (id: string, q: string) => void;
  setTimeRange: (id: string, r: TimeRange) => void;
  setResults: (id: string, r: ResultsState) => void;
  setChart: (id: string, c: ChartConfig) => void;
  resetAll: () => void;
};

const DEFAULT_RANGE: TimeRange = { preset: "1h" };

export const useWorkspace = create<Store>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeId: null,
      newTab: (seed) => {
        const id = crypto.randomUUID();
        const tab: Tab = {
          id, title: `query ${get().tabs.length + 1}`, query: "", timeRange: DEFAULT_RANGE,
          results: { kind: "idle" }, chart: {}, dirty: false, ...seed,
        };
        set({ tabs: [...get().tabs, tab], activeId: id });
        return id;
      },
      closeTab: (id) => {
        const tabs = get().tabs.filter((t) => t.id !== id);
        const activeId = get().activeId === id ? tabs.at(-1)?.id ?? null : get().activeId;
        set({ tabs, activeId });
      },
      setActive: (id) => set({ activeId: id }),
      setQuery: (id, query) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, query, dirty: true } : t)),
      }),
      setTimeRange: (id, timeRange) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, timeRange, dirty: true } : t)),
      }),
      setResults: (id, results) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, results, dirty: false } : t)),
      }),
      setChart: (id, chart) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, chart } : t)),
      }),
      resetAll: () => set({ tabs: [], activeId: null }),
    }),
    // Per-server scoped key is applied in Task 8.2 once the session store is plumbed here.
    { name: "kyma.workspace" },
  ),
);
```

- [ ] **Step 4: Verify pass**

```bash
pnpm -C web test src/features/tabs
```

- [ ] **Step 5: Commit**

```bash
git add web/src/features/tabs
git commit -m "feat(web): workspace store (tabs, queries, time-range, results)"
```

### Task 8.2 — Monaco editor + KQL language registration

**Files:**
- Create: `web/src/features/editor/kql-language.ts`
- Create: `web/src/features/editor/Editor.tsx`
- Modify: `web/package.json`

- [ ] **Step 1: Install Monaco**

```bash
pnpm -C web add monaco-editor@0.52.0 @monaco-editor/react@4.6.0
```

- [ ] **Step 2: Write `web/src/features/editor/kql-language.ts`**

```ts
import type * as MonacoNs from "monaco-editor";

const KEYWORDS = [
  "where","project","project-away","extend","summarize","by","take","limit",
  "sort","order","asc","desc","top","count","distinct","join","kind","on",
  "let","union","range","evaluate","parse","mv-expand","make-series","as","between",
];
const FUNCTIONS = [
  "now","ago","bin","startofhour","startofday","strcat","tolower","toupper","tostring",
  "toint","tolong","todouble","todatetime","isnull","isnotnull","isempty","iff",
  "count","sum","avg","min","max","dcount","percentile","stdev","variance",
];
const TYPES = ["int","long","real","double","bool","string","datetime","timespan","dynamic","guid"];
const STRING_OPS = ["contains","!contains","startswith","!startswith","endswith","!endswith","has","!has","matches","regex"];

export const KQL_LANG_ID = "kql";

export function registerKql(monaco: typeof MonacoNs) {
  if (monaco.languages.getLanguages().some((l) => l.id === KQL_LANG_ID)) return;

  monaco.languages.register({ id: KQL_LANG_ID, extensions: [".kql"], aliases: ["KQL","Kusto"] });

  monaco.languages.setMonarchTokensProvider(KQL_LANG_ID, {
    defaultToken: "",
    tokenPostfix: ".kql",
    keywords: KEYWORDS,
    functions: FUNCTIONS,
    typeKeywords: TYPES,
    stringOps: STRING_OPS,
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/\|/, "operator"],
        [/"([^"\\]|\\.)*"/, "string"],
        [/'([^'\\]|\\.)*'/, "string"],
        [/\b\d+(\.\d+)?\b/, "number"],
        [/\b(true|false|null)\b/, "keyword"],
        [/\b([A-Za-z_][A-Za-z0-9_]*)\b/, {
          cases: {
            "@keywords":     "keyword",
            "@functions":    "type.identifier",
            "@typeKeywords": "type",
            "@stringOps":    "keyword.operator",
            "@default":      "identifier",
          },
        }],
        [/[=!<>]=?|[+\-*/]|\band\b|\bor\b|\bnot\b/, "operator"],
      ],
    },
  });

  monaco.languages.setLanguageConfiguration(KQL_LANG_ID, {
    comments: { lineComment: "//" },
    brackets: [["(", ")"], ["[", "]"], ["{", "}"]],
    autoClosingPairs: [
      { open: "(", close: ")" },
      { open: "[", close: "]" },
      { open: "{", close: "}" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
  });
}

export function registerKqlCompletions(
  monaco: typeof MonacoNs,
  getSchema: () => { tables: string[]; columnsByTable: Record<string, string[]> },
) {
  monaco.languages.registerCompletionItemProvider(KQL_LANG_ID, {
    triggerCharacters: [" ", "|", "."],
    provideCompletionItems(model, position) {
      const schema = getSchema();
      const lineText = model.getLineContent(position.lineNumber).slice(0, position.column - 1);
      const word = model.getWordUntilPosition(position);
      const range = new monaco.Range(position.lineNumber, word.startColumn, position.lineNumber, word.endColumn);

      // After `| ` → suggest operators
      if (/\|\s*$/.test(lineText)) {
        return { suggestions: KEYWORDS.map((k) => ({
          label: k, kind: monaco.languages.CompletionItemKind.Keyword,
          insertText: k, range,
        })) };
      }
      // After `project `, `where `, `by `, `extend ` → suggest columns of the leading source
      const tableMatch = model.getValue().match(/^\s*([A-Za-z_][A-Za-z0-9_]*)/);
      const leadingTable = tableMatch?.[1];
      if (/(where|project|project-away|by|extend|summarize)\s+(\w*[,\s]*)*$/.test(lineText) && leadingTable) {
        const cols = schema.columnsByTable[leadingTable] ?? [];
        return { suggestions: cols.map((c) => ({
          label: c, kind: monaco.languages.CompletionItemKind.Field, insertText: c, range,
        })) };
      }
      // Default: offer tables + keywords + functions
      return {
        suggestions: [
          ...schema.tables.map((t) => ({
            label: t, kind: monaco.languages.CompletionItemKind.Class, insertText: t, range,
          })),
          ...KEYWORDS.map((k)  => ({ label: k, kind: monaco.languages.CompletionItemKind.Keyword,  insertText: k, range })),
          ...FUNCTIONS.map((f) => ({ label: f, kind: monaco.languages.CompletionItemKind.Function, insertText: `${f}()`, range })),
        ],
      };
    },
  });
}
```

- [ ] **Step 3: Write `web/src/features/editor/Editor.tsx`**

```tsx
import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef } from "react";
import { registerKql, registerKqlCompletions, KQL_LANG_ID } from "./kql-language";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  schema?: SchemaDoc;
  database: string;
};

export function Editor({ value, onChange, onRun, schema, database }: Props) {
  const onRunRef = useRef(onRun);
  useEffect(() => { onRunRef.current = onRun; }, [onRun]);

  const handleMount: OnMount = useCallback((editor, monaco) => {
    registerKql(monaco);
    registerKqlCompletions(monaco, () => {
      const db = schema?.databases.find((d) => d.name === database);
      const tables = db?.tables.map((t) => t.name) ?? [];
      const columnsByTable: Record<string, string[]> = {};
      db?.tables.forEach((t) => { columnsByTable[t.name] = t.columns.map((c) => c.name); });
      return { tables, columnsByTable };
    });
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => onRunRef.current());
  }, [database, schema]);

  return (
    <MonacoEditor
      height="100%"
      language={KQL_LANG_ID}
      theme="vs"
      value={value}
      onChange={(v) => onChange(v ?? "")}
      onMount={handleMount}
      options={{
        minimap: { enabled: false },
        lineNumbers: "on",
        scrollBeyondLastLine: false,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        tabSize: 2,
        automaticLayout: true,
        wordWrap: "on",
      }}
    />
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add web/src/features/editor web/package.json pnpm-lock.yaml
git commit -m "feat(web): Monaco editor with KQL language + completion"
```

### Task 8.3 — Time-range picker

**Files:**
- Create: `web/src/features/time-range/TimeRangePicker.tsx`
- Create: `web/src/features/time-range/time-range.ts`
- Create: `web/src/features/time-range/time-range.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/features/time-range/time-range.test.ts
import { expect, test } from "vitest";
import { prependTimeFilter, presetToKqlAgo } from "./time-range";

test("presetToKqlAgo maps presets", () => {
  expect(presetToKqlAgo("5m")).toBe("ago(5m)");
  expect(presetToKqlAgo("1h")).toBe("ago(1h)");
  expect(presetToKqlAgo("24h")).toBe("ago(24h)");
  expect(presetToKqlAgo("7d")).toBe("ago(7d)");
});

test("prependTimeFilter injects preamble only when user hasn't already filtered by timestamp", () => {
  const out = prependTimeFilter("otel_logs", { preset: "1h" });
  expect(out).toContain("| where timestamp between (ago(1h) .. now())");
});

test("prependTimeFilter leaves user-supplied timestamp filter alone", () => {
  const q = "otel_logs | where timestamp > ago(30m)";
  expect(prependTimeFilter(q, { preset: "1h" })).toBe(q);
});

test("prependTimeFilter supports custom from/to", () => {
  const out = prependTimeFilter("otel_logs", {
    preset: "custom", from: "2026-04-20T14:00:00Z", to: "2026-04-20T15:00:00Z",
  });
  expect(out).toContain("datetime(2026-04-20T14:00:00Z)");
  expect(out).toContain("datetime(2026-04-20T15:00:00Z)");
});
```

- [ ] **Step 2: Confirm failure**

```bash
pnpm -C web test src/features/time-range
```

- [ ] **Step 3: Write `web/src/features/time-range/time-range.ts`**

```ts
import type { TimeRange, TimeRangePreset } from "@/features/tabs/workspace-store";

export function presetToKqlAgo(p: TimeRangePreset): string {
  switch (p) {
    case "5m":  return "ago(5m)";
    case "15m": return "ago(15m)";
    case "1h":  return "ago(1h)";
    case "6h":  return "ago(6h)";
    case "24h": return "ago(24h)";
    case "7d":  return "ago(7d)";
    case "30d": return "ago(30d)";
    case "custom": return "";
  }
}

export function prependTimeFilter(query: string, range: TimeRange): string {
  // If the user already filtered by timestamp, respect it.
  if (/\btimestamp\s*(>|<|between)/i.test(query)) return query;

  let filter: string;
  if (range.preset === "custom" && range.from && range.to) {
    filter = `| where timestamp between (datetime(${range.from}) .. datetime(${range.to}))`;
  } else {
    const ago = presetToKqlAgo(range.preset);
    if (!ago) return query;
    filter = `| where timestamp between (${ago} .. now())`;
  }

  // Insert immediately after the leading table name (first line).
  const lines = query.split("\n");
  const firstNonEmpty = lines.findIndex((l) => l.trim().length > 0);
  if (firstNonEmpty < 0) return `${query}\n${filter}`;
  lines.splice(firstNonEmpty + 1, 0, filter);
  return lines.join("\n");
}
```

- [ ] **Step 4: Write `web/src/features/time-range/TimeRangePicker.tsx`**

```tsx
import { useState } from "react";
import type { TimeRange, TimeRangePreset } from "@/features/tabs/workspace-store";
import { Button } from "@/components/ui/button";

const PRESETS: { value: TimeRangePreset; label: string }[] = [
  { value: "5m",  label: "Last 5 min" },
  { value: "15m", label: "Last 15 min" },
  { value: "1h",  label: "Last 1 hour" },
  { value: "6h",  label: "Last 6 hours" },
  { value: "24h", label: "Last 24 hours" },
  { value: "7d",  label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
  { value: "custom", label: "Custom…" },
];

export function TimeRangePicker({ value, onChange }: { value: TimeRange; onChange: (r: TimeRange) => void }) {
  const [open, setOpen] = useState(false);
  const label = PRESETS.find((p) => p.value === value.preset)?.label ?? "Last 1 hour";
  return (
    <div className="relative">
      <Button variant="outline" size="sm" onClick={() => setOpen((v) => !v)}>
        {label}
      </Button>
      {open && (
        <div className="absolute z-20 mt-1 w-48 rounded-md border bg-popover p-1 shadow-md">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              className="block w-full rounded px-2 py-1 text-left text-xs hover:bg-accent"
              onClick={() => {
                onChange({ preset: p.value });
                setOpen(false);
              }}>
              {p.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Verify tests pass**

```bash
pnpm -C web test
```

- [ ] **Step 6: Commit**

```bash
git add web/src/features/time-range
git commit -m "feat(web): time-range picker + KQL preamble injection"
```

### Task 8.4 — Explore page: editor + run + status bar

**Files:**
- Modify: `web/src/routes/_app.explore.tsx`
- Create: `web/src/features/run/useRunQuery.ts`

- [ ] **Step 1: Write `web/src/features/run/useRunQuery.ts`**

```ts
import { useCallback, useRef } from "react";
import { toast } from "sonner";
import { runQuery } from "@/sdk/query";
import { useSession } from "@/sdk/session";
import { useWorkspace, type Tab } from "@/features/tabs/workspace-store";
import { prependTimeFilter } from "@/features/time-range/time-range";
import { batchToRows, autoChartAxes, type Column } from "@/sdk/arrow";
import type { RecordBatch } from "apache-arrow";

export type TabResult = {
  columns: Column[];
  rows: Record<string, unknown>[];
  chartPoints: Record<string, unknown>[];
  totalBytes: number;
};

export function useRunQuery() {
  const aborters = useRef(new Map<string, AbortController>());

  const run = useCallback(async (tab: Tab, onBatch: (r: TabResult) => void) => {
    const { endpoint, token, database } = useSession.getState();
    const workspace = useWorkspace.getState();
    if (!endpoint || !token) { toast.error("Configure server + token in Settings"); return; }

    // Cancel any in-flight run for this tab.
    aborters.current.get(tab.id)?.abort();
    const ctl = new AbortController();
    aborters.current.set(tab.id, ctl);

    const finalQuery = prependTimeFilter(tab.query, tab.timeRange);
    const startedAt = performance.now();
    workspace.setResults(tab.id, { kind: "running", startedAt: Date.now() });

    const acc: TabResult = { columns: [], rows: [], chartPoints: [], totalBytes: 0 };
    try {
      for await (const batch of runQuery({
        endpoint, token, database, query: finalQuery, language: "kql", signal: ctl.signal,
      })) {
        const { columns, rows } = batchToRows(batch as RecordBatch);
        if (acc.columns.length === 0) acc.columns = columns;
        acc.rows.push(...rows);
        acc.chartPoints.push(...rows);
        onBatch(acc);
      }
      const durationMs = performance.now() - startedAt;
      workspace.setResults(tab.id, {
        kind: "ok", rowCount: acc.rows.length, bytes: acc.totalBytes,
        durationMs, finishedAt: Date.now(),
      });
      const axes = autoChartAxes(acc.columns);
      if (axes.type !== "none" && !workspace.tabs.find((t) => t.id === tab.id)?.chart.override) {
        // pass-through — chart component reads from auto result via columns
      }
    } catch (e) {
      if ((e as Error).name === "AbortError") {
        workspace.setResults(tab.id, { kind: "idle" });
        return;
      }
      const msg = (e as Error).message || "query failed";
      toast.error(msg);
      workspace.setResults(tab.id, { kind: "error", message: msg });
    }
  }, []);

  const cancel = useCallback((tabId: string) => {
    aborters.current.get(tabId)?.abort();
    aborters.current.delete(tabId);
  }, []);

  return { run, cancel };
}
```

- [ ] **Step 2: Update `web/src/routes/_app.explore.tsx`**

```tsx
import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Editor } from "@/features/editor/Editor";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { useRunQuery, type TabResult } from "@/features/run/useRunQuery";

export const Route = createFileRoute("/_app/explore")({ component: ExplorePage });

function ExplorePage() {
  const { endpoint, token, database } = useSession();
  const workspace = useWorkspace();
  const { run, cancel } = useRunQuery();
  const [liveResult, setLiveResult] = useState<TabResult | null>(null);

  const { data: schema } = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    staleTime: 5 * 60_000,
    enabled: Boolean(endpoint && token),
  });

  // Ensure at least one tab exists.
  useEffect(() => {
    if (workspace.tabs.length === 0) workspace.newTab({ query: "otel_logs | take 50" });
  }, [workspace]);

  const active = workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0];
  const status = active?.results.kind ?? "idle";

  const runActive = async () => {
    if (!active) return;
    setLiveResult(null);
    await run(active, setLiveResult);
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b px-4 py-2 text-xs">
        <TimeRangePicker
          value={active?.timeRange ?? { preset: "1h" }}
          onChange={(r) => active && workspace.setTimeRange(active.id, r)}
        />
        {status === "running" ? (
          <Button size="sm" variant="destructive" onClick={() => active && cancel(active.id)}>
            <Square className="mr-1 h-3.5 w-3.5" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={runActive} disabled={!schema}>
            <Play className="mr-1 h-3.5 w-3.5" /> Run <kbd className="ml-2 text-muted-foreground">⌘↵</kbd>
          </Button>
        )}
        <span className="ml-auto text-muted-foreground">
          {active?.results.kind === "ok"
            ? `${active.results.rowCount.toLocaleString()} rows · ${active.results.durationMs.toFixed(0)} ms`
            : active?.results.kind === "error" ? `error: ${active.results.message}` : ""}
        </span>
      </div>

      <div className="flex-1 overflow-hidden">
        <div className="h-1/2 border-b">
          {active && schema && (
            <Editor
              value={active.query}
              onChange={(v) => workspace.setQuery(active.id, v)}
              onRun={runActive}
              schema={schema}
              database={database}
            />
          )}
        </div>
        <div className="h-1/2 overflow-auto p-4 text-xs">
          {!liveResult && <p className="text-muted-foreground">Run a query to see results.</p>}
          {liveResult && <PreviewTable r={liveResult} />}
        </div>
      </div>
    </div>
  );
}

function PreviewTable({ r }: { r: TabResult }) {
  const cols = useMemo(() => r.columns.slice(0, 20), [r.columns]);
  return (
    <table className="min-w-full font-mono">
      <thead>
        <tr className="border-b">
          {cols.map((c) => (<th key={c.name} className="px-2 py-1 text-left">{c.name}</th>))}
        </tr>
      </thead>
      <tbody>
        {r.rows.slice(0, 200).map((row, i) => (
          <tr key={i} className="border-b">
            {cols.map((c) => (<td key={c.name} className="px-2 py-1">{String(row[c.name] ?? "")}</td>))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
```

- [ ] **Step 3: Verify build**

```bash
pnpm -C web build
```

- [ ] **Step 4: Smoke test end-to-end**

```bash
cargo run -p kyma-bin --features web-ui &
SERVER=$!
pnpm -C web dev
# browse http://localhost:5173, settings → paste token → /explore → write `otel_logs | take 5` → ⌘↵ → rows should appear
kill $SERVER
```

- [ ] **Step 5: Commit**

```bash
git add web/src/features/run web/src/routes/_app.explore.tsx
git commit -m "feat(web): explore page with editor + run + inline results"
```

### Task 8.5 — Results grid (TanStack Table + virtualization)

**Files:**
- Create: `web/src/features/results-grid/ResultsGrid.tsx`
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Write `web/src/features/results-grid/ResultsGrid.tsx`**

```tsx
import { useMemo, useRef } from "react";
import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type SortingState } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useState } from "react";
import type { Column } from "@/sdk/arrow";

export function ResultsGrid({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const defs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () => columns.map((c) => ({
      accessorKey: c.name,
      header: c.name,
      cell: (info) => String(info.getValue() ?? ""),
    })),
    [columns],
  );
  const table = useReactTable({
    data: rows,
    columns: defs,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: table.getRowModel().rows.length,
    estimateSize: () => 28,
    getScrollElement: () => scrollRef.current,
    overscan: 12,
  });

  return (
    <div ref={scrollRef} className="h-full overflow-auto text-xs font-mono">
      <table className="min-w-full">
        <thead className="sticky top-0 z-10 bg-background">
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="border-b">
              {hg.headers.map((h) => (
                <th
                  key={h.id}
                  className="cursor-pointer px-2 py-1 text-left hover:bg-accent/50"
                  onClick={h.column.getToggleSortingHandler()}>
                  {flexRender(h.column.columnDef.header, h.getContext())}
                  {({ asc: " ▲", desc: " ▼" } as Record<string, string>)[h.column.getIsSorted() as string] ?? ""}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const row = table.getRowModel().rows[vi.index];
            return (
              <tr key={row.id} style={{
                position: "absolute", top: 0, left: 0, width: "100%",
                transform: `translateY(${vi.start}px)`,
              }} className="border-b hover:bg-accent/30">
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="px-2 py-1">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function exportCsv(columns: Column[], rows: Record<string, unknown>[]): Blob {
  const header = columns.map((c) => csvCell(c.name)).join(",");
  const body = rows.map((r) => columns.map((c) => csvCell(String(r[c.name] ?? ""))).join(",")).join("\n");
  return new Blob([header, "\n", body], { type: "text/csv" });
}

function csvCell(s: string): string {
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}
```

- [ ] **Step 2: Replace `PreviewTable` in `_app.explore.tsx`**

In `web/src/routes/_app.explore.tsx`, import `ResultsGrid` and replace the `<PreviewTable r={liveResult}/>` line with:
```tsx
<ResultsGrid columns={liveResult.columns} rows={liveResult.rows} />
```
Delete the inline `PreviewTable` function.

- [ ] **Step 3: Build + smoke test**

```bash
pnpm -C web build
```

- [ ] **Step 4: Wire CSV + JSON export buttons**

Add to the toolbar in `_app.explore.tsx` (next to the Run button):
```tsx
import { exportCsv } from "@/features/results-grid/ResultsGrid";

// ...
<Button size="sm" variant="ghost" disabled={!liveResult?.rows.length} onClick={() => {
  if (!liveResult) return;
  const blob = exportCsv(liveResult.columns, liveResult.rows);
  downloadBlob(blob, `kyma-${Date.now()}.csv`);
}}>Export CSV</Button>

<Button size="sm" variant="ghost" disabled={!liveResult?.rows.length} onClick={() => {
  if (!liveResult) return;
  const blob = new Blob([JSON.stringify(liveResult.rows, null, 2)], { type: "application/json" });
  downloadBlob(blob, `kyma-${Date.now()}.json`);
}}>Export JSON</Button>
```

And define the helper once in `web/src/lib/download.ts`:
```ts
export function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url; a.download = filename;
  document.body.appendChild(a); a.click();
  a.remove(); URL.revokeObjectURL(url);
}
```

Import it in the explore page.

- [ ] **Step 5: Commit**

```bash
git add web/src/features/results-grid web/src/routes/_app.explore.tsx web/src/lib/download.ts
git commit -m "feat(web): virtualized sortable results grid + CSV/JSON export"
```

### Task 8.6 — Chart panel (ECharts + auto-axis)

**Files:**
- Create: `web/src/features/chart/ChartPanel.tsx`
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Install ECharts**

```bash
pnpm -C web add echarts@5.5.0 echarts-for-react@3.0.2
```

- [ ] **Step 2: Write `web/src/features/chart/ChartPanel.tsx`**

```tsx
import ReactECharts from "echarts-for-react";
import { useMemo } from "react";
import { autoChartAxes, type Column } from "@/sdk/arrow";

export function ChartPanel({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  const option = useMemo(() => {
    const spec = autoChartAxes(columns);
    if (spec.type === "none")
      return null;
    if (spec.type === "stat") {
      const v = Number(rows.at(-1)?.[spec.y] ?? NaN);
      return null; // stat is rendered as text below instead of ECharts
    }
    const x = rows.map((r) => r[spec.x!]);
    const y = rows.map((r) => Number(r[spec.y]));

    if (spec.type === "line" || spec.type === "scatter") {
      return {
        animation: false,
        tooltip: { trigger: "axis" },
        xAxis: { type: spec.type === "line" ? "time" : "value", name: spec.x },
        yAxis: { type: "value", name: spec.y },
        series: [{ name: spec.y, type: spec.type === "line" ? "line" : "scatter", data: rows.map((r) => [r[spec.x!], r[spec.y]]), showSymbol: false }],
      };
    }
    // bar
    return {
      animation: false,
      tooltip: { trigger: "axis" },
      xAxis: { type: "category", data: x, name: spec.x },
      yAxis: { type: "value", name: spec.y },
      series: [{ name: spec.y, type: "bar", data: y }],
    };
  }, [columns, rows]);

  if (!option) {
    const spec = autoChartAxes(columns);
    if (spec.type === "stat") {
      const v = rows.at(-1)?.[spec.y];
      return (
        <div className="flex h-full flex-col items-center justify-center">
          <div className="text-sm text-muted-foreground">{spec.y}</div>
          <div className="text-5xl font-bold tabular-nums">{String(v ?? "—")}</div>
        </div>
      );
    }
    return <div className="p-6 text-sm text-muted-foreground">No auto-chart for this shape.</div>;
  }
  return <ReactECharts option={option} style={{ height: "100%", width: "100%" }} theme="default" notMerge={true} />;
}
```

- [ ] **Step 3: Add Results/Chart tab switcher in `_app.explore.tsx`**

In `web/src/routes/_app.explore.tsx`, replace the single-panel results area with:
```tsx
import { useState } from "react";
import { ChartPanel } from "@/features/chart/ChartPanel";

// inside component:
const [view, setView] = useState<"grid" | "chart">("grid");
// ...
<div className="h-1/2 flex flex-col">
  <div className="flex items-center gap-1 border-b px-3 py-1 text-xs">
    <button className={tabBtn(view === "grid")}  onClick={() => setView("grid")}>Results</button>
    <button className={tabBtn(view === "chart")} onClick={() => setView("chart")}>Chart</button>
  </div>
  <div className="flex-1 overflow-hidden">
    {liveResult && view === "grid"  && <ResultsGrid columns={liveResult.columns} rows={liveResult.rows} />}
    {liveResult && view === "chart" && <ChartPanel  columns={liveResult.columns} rows={liveResult.rows} />}
    {!liveResult && <p className="p-6 text-xs text-muted-foreground">Run a query to see results.</p>}
  </div>
</div>
```

Define `tabBtn`:
```tsx
const tabBtn = (on: boolean) =>
  `rounded-md px-2 py-0.5 transition ${on ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/50"}`;
```

- [ ] **Step 4: Commit**

```bash
git add web/src/features/chart web/src/routes/_app.explore.tsx web/package.json pnpm-lock.yaml
git commit -m "feat(web): chart panel with auto-axis detection"
```

---

## Phase 9 — Workspace polish

### Task 9.1 — Schema browser (left rail)

**Files:**
- Create: `web/src/features/schema-browser/SchemaBrowser.tsx`
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Install `@radix-ui/react-collapsible`**

```bash
pnpm -C web dlx shadcn@2.1.0 add collapsible scroll-area
```

- [ ] **Step 2: Write `web/src/features/schema-browser/SchemaBrowser.tsx`**

```tsx
import { useState } from "react";
import { ChevronRight, Database, Table2, Columns3 } from "lucide-react";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = { schema?: SchemaDoc; onInsert: (text: string) => void };

export function SchemaBrowser({ schema, onInsert }: Props) {
  if (!schema) return <div className="p-3 text-xs text-muted-foreground">Loading schema…</div>;
  return (
    <div className="h-full overflow-auto p-2 text-xs">
      {schema.databases.map((db) => <DatabaseNode key={db.name} db={db} onInsert={onInsert} />)}
    </div>
  );
}

function DatabaseNode({ db, onInsert }: { db: SchemaDoc["databases"][number]; onInsert: (t: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div>
      <button className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50" onClick={() => setOpen(!open)}>
        <ChevronRight className={`h-3 w-3 transition ${open ? "rotate-90" : ""}`} />
        <Database className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="font-medium">{db.name}</span>
      </button>
      {open && <div className="ml-3 border-l pl-1">{db.tables.map((t) => <TableNode key={t.name} table={t} onInsert={onInsert} />)}</div>}
    </div>
  );
}

function TableNode({ table, onInsert }: { table: SchemaDoc["databases"][number]["tables"][number]; onInsert: (t: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50"
        onClick={() => setOpen(!open)}
        onDoubleClick={() => onInsert(table.name)}>
        <ChevronRight className={`h-3 w-3 transition ${open ? "rotate-90" : ""}`} />
        <Table2 className="h-3.5 w-3.5 text-muted-foreground" />
        <span>{table.name}</span>
      </button>
      {open && (
        <div className="ml-3 border-l pl-1 text-[11px]">
          {table.columns.map((c) => (
            <button key={c.name}
              className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-left hover:bg-accent/50"
              onClick={() => onInsert(c.name)}>
              <Columns3 className="h-3 w-3 text-muted-foreground" />
              <span>{c.name}</span>
              <span className="ml-auto text-muted-foreground">{c.type}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 3: Add left rail to explore page**

In `web/src/routes/_app.explore.tsx`, wrap the main editor/results area in a flex row and add the left rail:
```tsx
import { SchemaBrowser } from "@/features/schema-browser/SchemaBrowser";
// ...
<div className="flex h-full">
  <aside className="w-64 shrink-0 border-r">
    <SchemaBrowser schema={schema} onInsert={(t) => active && workspace.setQuery(active.id, `${active.query}${active.query.endsWith("\n") || !active.query ? "" : " "}${t}`)} />
  </aside>
  <section className="flex flex-1 flex-col">
    {/* existing toolbar + editor/results */}
  </section>
</div>
```

- [ ] **Step 4: Commit**

```bash
git add web/src/features/schema-browser web/src/routes/_app.explore.tsx
git commit -m "feat(web): schema browser left rail with click-to-insert"
```

### Task 9.2 — Tab bar

**Files:**
- Create: `web/src/features/tabs/TabBar.tsx`
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Write `web/src/features/tabs/TabBar.tsx`**

```tsx
import { Plus, X } from "lucide-react";
import { useWorkspace } from "./workspace-store";

export function TabBar() {
  const { tabs, activeId, setActive, closeTab, newTab } = useWorkspace();
  return (
    <div className="flex items-center gap-1 border-b bg-background px-2 py-1 text-xs">
      {tabs.map((t) => (
        <button key={t.id}
          onClick={() => setActive(t.id)}
          className={`flex items-center gap-1 rounded px-2 py-1 ${t.id === activeId ? "bg-accent text-accent-foreground" : "hover:bg-accent/40 text-muted-foreground"}`}>
          {t.dirty && <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />}
          <span className="max-w-[20ch] truncate">{t.title}</span>
          <X className="h-3 w-3 opacity-60 hover:opacity-100"
             onClick={(e) => { e.stopPropagation(); closeTab(t.id); }} />
        </button>
      ))}
      <button className="rounded p-1 hover:bg-accent/40" onClick={() => newTab({ query: "" })} title="New tab (⌘T)">
        <Plus className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
```

- [ ] **Step 2: Mount in explore page**

Insert `<TabBar />` just above the toolbar in `_app.explore.tsx`.

- [ ] **Step 3: Add keyboard shortcuts**

Create `web/src/lib/shortcuts.ts`:
```ts
import { useEffect } from "react";
import { useWorkspace } from "@/features/tabs/workspace-store";

export function useWorkspaceShortcuts(runActive: () => void) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      if (e.key === "Enter") { e.preventDefault(); runActive(); }
      else if (e.key.toLowerCase() === "t") { e.preventDefault(); useWorkspace.getState().newTab({ query: "" }); }
      else if (e.key.toLowerCase() === "w") {
        e.preventDefault();
        const s = useWorkspace.getState();
        if (s.activeId) s.closeTab(s.activeId);
      } else if (/^[1-9]$/.test(e.key)) {
        const idx = Number(e.key) - 1;
        const t = useWorkspace.getState().tabs[idx];
        if (t) { e.preventDefault(); useWorkspace.getState().setActive(t.id); }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [runActive]);
}
```
Call `useWorkspaceShortcuts(runActive)` inside `ExplorePage`.

- [ ] **Step 4: Commit**

```bash
git add web/src/features/tabs/TabBar.tsx web/src/lib/shortcuts.ts web/src/routes/_app.explore.tsx
git commit -m "feat(web): tab bar + keyboard shortcuts"
```

### Task 9.3 — URL state codec

**Files:**
- Create: `web/src/lib/url-state.ts`
- Create: `web/src/lib/url-state.test.ts`
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Write the failing test**

```ts
// web/src/lib/url-state.test.ts
import { expect, test } from "vitest";
import { encodeQueryState, decodeQueryState } from "./url-state";

test("round-trips a query + time range", () => {
  const s = { query: "otel_logs | take 5", preset: "1h", from: undefined, to: undefined } as const;
  const encoded = encodeQueryState(s);
  expect(encoded.length).toBeGreaterThan(0);
  const got = decodeQueryState(encoded);
  expect(got).toEqual(s);
});

test("decode returns null for garbage", () => {
  expect(decodeQueryState("not-a-valid-base64!!")).toBeNull();
});
```

- [ ] **Step 2: Write `web/src/lib/url-state.ts`**

```ts
import type { TimeRangePreset } from "@/features/tabs/workspace-store";

export type QueryState = { query: string; preset: TimeRangePreset; from?: string; to?: string };

export function encodeQueryState(s: QueryState): string {
  const json = JSON.stringify(s);
  const utf8 = new TextEncoder().encode(json);
  // base64url, no padding
  return btoa(String.fromCharCode(...utf8))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

export function decodeQueryState(encoded: string): QueryState | null {
  try {
    const b64 = encoded.replace(/-/g, "+").replace(/_/g, "/") + "===".slice(0, (4 - (encoded.length % 4)) % 4);
    const json = atob(b64);
    return JSON.parse(json) as QueryState;
  } catch {
    return null;
  }
}
```

- [ ] **Step 3: Verify pass**

```bash
pnpm -C web test src/lib/url-state.test.ts
```

- [ ] **Step 4: Integrate into explore page**

In `_app.explore.tsx`:
```tsx
import { useSearch, useNavigate } from "@tanstack/react-router";
import { encodeQueryState, decodeQueryState } from "@/lib/url-state";

// in createFileRoute:
validateSearch: (s) => ({ q: typeof s.q === "string" ? s.q : undefined }),

// inside ExplorePage:
const { q } = useSearch({ from: "/_app/explore" });
const navigate = useNavigate();
useEffect(() => {
  if (q) {
    const decoded = decodeQueryState(q);
    if (decoded) workspace.newTab({ query: decoded.query, timeRange: { preset: decoded.preset, from: decoded.from, to: decoded.to } });
  }
  // eslint-disable-next-line react-hooks/exhaustive-deps
}, []);

// after successful run, sync URL:
useEffect(() => {
  if (!active) return;
  const encoded = encodeQueryState({
    query: active.query, preset: active.timeRange.preset, from: active.timeRange.from, to: active.timeRange.to,
  });
  navigate({ to: "/explore", search: { q: encoded }, replace: true });
}, [active?.query, active?.timeRange.preset]);
```

- [ ] **Step 5: Commit**

```bash
git add web/src/lib/url-state.ts web/src/lib/url-state.test.ts web/src/routes/_app.explore.tsx
git commit -m "feat(web): URL-state codec for shareable queries"
```

### Task 9.4 — Command palette (⌘K)

**Files:**
- Create: `web/src/features/palette/CommandPalette.tsx`
- Modify: `web/src/app/shell.tsx`

- [ ] **Step 1: Install shadcn command component**

```bash
pnpm -C web dlx shadcn@2.1.0 add command dialog
```

- [ ] **Step 2: Write `web/src/features/palette/CommandPalette.tsx`**

```tsx
import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Compass, Settings as Gear, FilePlus, Play } from "lucide-react";
import { Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { useWorkspace } from "@/features/tabs/workspace-store";

export function CommandPalette({ onRun }: { onRun: () => void }) {
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); setOpen((v) => !v); }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const close = () => setOpen(false);

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Type a command or search…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem onSelect={() => { onRun(); close(); }}>
            <Play className="mr-2 h-4 w-4" /> Run current query <kbd className="ml-auto text-xs">⌘↵</kbd>
          </CommandItem>
          <CommandItem onSelect={() => { useWorkspace.getState().newTab({ query: "" }); close(); }}>
            <FilePlus className="mr-2 h-4 w-4" /> New tab <kbd className="ml-auto text-xs">⌘T</kbd>
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Navigate">
          <CommandItem onSelect={() => { navigate({ to: "/explore" }); close(); }}>
            <Compass className="mr-2 h-4 w-4" /> Explore
          </CommandItem>
          <CommandItem onSelect={() => { navigate({ to: "/settings" }); close(); }}>
            <Gear className="mr-2 h-4 w-4" /> Settings
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
```

- [ ] **Step 3: Mount in explore page**

Pass `onRun={runActive}` from `ExplorePage` into a mounted `<CommandPalette />`.

- [ ] **Step 4: Commit**

```bash
git add web/src/features/palette web/src/routes/_app.explore.tsx
git commit -m "feat(web): ⌘K command palette"
```

---

## Phase 10 — Error + loading polish

### Task 10.1 — Reconnect watcher

**Files:**
- Create: `web/src/sdk/reconnect.ts`
- Create: `web/src/features/reconnect/ReconnectBanner.tsx`
- Modify: `web/src/app/shell.tsx`

- [ ] **Step 1: Write `web/src/sdk/reconnect.ts`**

```ts
import { create } from "zustand";

type State = { online: boolean; setOnline: (v: boolean) => void; start: (endpoint: string) => () => void };

export const useHealth = create<State>((set) => ({
  online: true,
  setOnline: (online) => set({ online }),
  start: (endpoint) => {
    let stop = false;
    const tick = async () => {
      try {
        const res = await fetch(`${endpoint.replace(/\/$/, "")}/health`, { cache: "no-store" });
        set({ online: res.ok });
      } catch {
        set({ online: false });
      }
    };
    tick();
    const id = window.setInterval(tick, 30_000);
    return () => { stop = true; clearInterval(id); };
  },
}));
```

- [ ] **Step 2: Write `ReconnectBanner.tsx`**

```tsx
import { useHealth } from "@/sdk/reconnect";

export function ReconnectBanner() {
  const online = useHealth((s) => s.online);
  if (online) return null;
  return (
    <div className="border-b border-destructive/40 bg-destructive/10 px-4 py-1 text-xs text-destructive">
      Reconnecting to kyma…
    </div>
  );
}
```

- [ ] **Step 3: Start watcher in shell**

In `web/src/app/shell.tsx`, add:
```tsx
import { useEffect } from "react";
import { useHealth } from "@/sdk/reconnect";
// ...
const { endpoint } = useSession();
const startHealth = useHealth((s) => s.start);
useEffect(() => endpoint ? startHealth(endpoint) : undefined, [endpoint, startHealth]);
// render `<ReconnectBanner />` just under `<header>`
```

- [ ] **Step 4: Commit**

```bash
git add web/src/sdk/reconnect.ts web/src/features/reconnect web/src/app/shell.tsx
git commit -m "feat(web): reconnect watcher + banner"
```

### Task 10.2 — Empty + error states in results area

**Files:**
- Modify: `web/src/routes/_app.explore.tsx`

- [ ] **Step 1: Render explicit states**

Update the results area to show:
```tsx
{active?.results.kind === "idle"    && <EmptyState text="Run a query to see results." />}
{active?.results.kind === "running" && <EmptyState text="Streaming results…" />}
{active?.results.kind === "error"   && <ErrorCard msg={active.results.message} rid={active.results.requestId} />}
{liveResult && active?.results.kind === "ok" && liveResult.rows.length === 0 &&
  <EmptyState text={`0 rows in ${active.results.durationMs.toFixed(0)} ms — try widening the time range.`} />}
```

with:
```tsx
function EmptyState({ text }: { text: string }) {
  return <div className="flex h-full items-center justify-center p-6 text-xs text-muted-foreground">{text}</div>;
}

function ErrorCard({ msg, rid }: { msg: string; rid?: string }) {
  return (
    <div className="mx-auto mt-6 max-w-2xl rounded border border-destructive/40 bg-destructive/5 p-4 text-xs">
      <div className="font-semibold text-destructive">Query failed</div>
      <pre className="mt-1 whitespace-pre-wrap font-mono text-destructive/90">{msg}</pre>
      {rid && <div className="mt-2 text-muted-foreground">request id: <code>{rid}</code></div>}
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/routes/_app.explore.tsx
git commit -m "feat(web): explicit empty/running/error states"
```

---

## Phase 11 — Tauri desktop

### Task 11.1 — Scaffold Tauri 2 wrapper

**Files:**
- Create: `web/src-tauri/` (via CLI)
- Modify: `web/package.json`

- [ ] **Step 1: Scaffold**

```bash
pnpm -C web add -D @tauri-apps/cli@2.1.0
pnpm -C web tauri init \
  --app-name kyma \
  --window-title "kyma" \
  --dist-dir ../dist \
  --dev-path http://localhost:5173 \
  --ci
```

- [ ] **Step 2: Add plugin-store**

```bash
pnpm -C web add @tauri-apps/plugin-store@2.1.0
(cd web/src-tauri && cargo add tauri-plugin-store@2.1.0)
```

Edit `web/src-tauri/src/lib.rs` to register the plugin:
```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 3: Verify desktop build**

```bash
pnpm -C web build
pnpm -C web tauri build --no-bundle  # compiles the binary without creating installers
```
Expected: `web/src-tauri/target/release/kyma` or `.exe` exists.

- [ ] **Step 4: Commit**

```bash
git add web/src-tauri web/package.json pnpm-lock.yaml
git commit -m "feat(desktop): scaffold Tauri 2 wrapper"
```

### Task 11.2 — Keyring-backed token storage on desktop

**Files:**
- Create: `web/src/sdk/session-desktop.ts`
- Modify: `web/src/sdk/session.ts`

- [ ] **Step 1: Detect Tauri + replace persistence**

Update `web/src/sdk/session.ts` so when `window.__TAURI__` exists, the persist middleware routes to the plugin-store:
```ts
import { create } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";

const isTauri = typeof window !== "undefined" && "__TAURI__" in window;

const tauriStorage: StateStorage = {
  async getItem(name)      { const { Store } = await import("@tauri-apps/plugin-store"); const s = await Store.load("kyma.store"); return (await s.get<string>(name)) ?? null; },
  async setItem(name, val) { const { Store } = await import("@tauri-apps/plugin-store"); const s = await Store.load("kyma.store"); await s.set(name, val); await s.save(); },
  async removeItem(name)   { const { Store } = await import("@tauri-apps/plugin-store"); const s = await Store.load("kyma.store"); await s.delete(name); await s.save(); },
};

export const useSession = create<SessionState>()(
  persist(
    /* ...existing state factory... */,
    {
      name: "kyma.session",
      storage: isTauri ? createJSONStorage(() => tauriStorage) : createJSONStorage(() => localStorage),
    },
  ),
);
```

(Reuse the existing `SessionState` shape from Task 6.2.)

- [ ] **Step 2: Commit**

```bash
git add web/src/sdk/session.ts
git commit -m "feat(desktop): store session in OS keyring via plugin-store"
```

---

## Phase 12 — Packaging

### Task 12.1 — Dockerfile (multi-stage)

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`

- [ ] **Step 1: Write `.dockerignore`**

```
.git
.worktrees
target
web/node_modules
web/dist
node_modules
.claude
docs/superpowers
```

- [ ] **Step 2: Write `Dockerfile`**

```dockerfile
# syntax=docker/dockerfile:1.7

# ---------- stage 1: web ----------
FROM node:20-alpine AS web
RUN corepack enable && corepack prepare pnpm@9 --activate
WORKDIR /src
COPY pnpm-workspace.yaml package.json .npmrc ./
COPY web/package.json web/pnpm-lock.yaml* web/
RUN pnpm -C web install --frozen-lockfile || pnpm -C web install
COPY web/ web/
RUN pnpm -C web build

# ---------- stage 2: server ----------
FROM rust:1.84-bookworm AS server
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
COPY --from=web /src/web/dist ./web/dist
RUN cargo build -p kyma-bin --release --features web-ui

# ---------- stage 3: runtime ----------
FROM gcr.io/distroless/cc-debian12
COPY --from=server /src/target/release/kyma-bin /usr/local/bin/kyma
EXPOSE 8080 9090
USER nonroot
ENTRYPOINT ["/usr/local/bin/kyma"]
```

- [ ] **Step 3: Verify build**

```bash
docker build -t kyma:dev .
docker run --rm -p 8080:8080 -p 9090:9090 kyma:dev &
sleep 3
curl -sS http://localhost:8080/health
docker kill $(docker ps -q --filter ancestor=kyma:dev)
```

- [ ] **Step 4: Commit**

```bash
git add Dockerfile .dockerignore
git commit -m "feat(deploy): multi-stage Dockerfile with web-ui feature"
```

### Task 12.2 — `kyma` service in docker-compose

**Files:**
- Modify: `docker-compose.yml`

- [ ] **Step 1: Add service block**

Append to `docker-compose.yml` under `services:`:
```yaml
  kyma:
    build: .
    container_name: kyma
    depends_on:
      postgres:
        condition: service_healthy
      minio:
        condition: service_healthy
    environment:
      KYMA_CATALOG_URL: postgres://kyma:kyma_dev@postgres:5432/kyma
      KYMA_OBJECT_STORE_URL: s3://kyma?endpoint=http://minio:9000&access_key_id=kyma_admin&secret_access_key=kyma_admin_dev
      KYMA_HTTP_ADDR: 0.0.0.0:8080
      KYMA_GRPC_ADDR: 0.0.0.0:9090
      KYMA_AUTH_DISABLED: "1"    # dev-mode unauthenticated; set KYMA_AUTH_TOKENS for real auth
    ports:
      - "8080:8080"
      - "9090:9090"
    healthcheck:
      test: ["CMD", "wget", "--spider", "-q", "http://localhost:8080/health"]
      interval: 5s
      timeout: 3s
      retries: 20
```

Confirm the env var names match whatever `kyma-bin` currently reads (check `crates/kyma-bin/src/main.rs`). Replace placeholders with the real names.

- [ ] **Step 2: Smoke test**

```bash
docker-compose up -d --build
sleep 8
curl -sS http://localhost:8080/health && echo
docker-compose down
```

- [ ] **Step 3: Commit**

```bash
git add docker-compose.yml
git commit -m "feat(deploy): kyma service in docker-compose"
```

### Task 12.3 — README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the Quick Start section**

Append the following block after the existing "Quick start (dev stack)" snippet:
```markdown
### Web UI

`docker-compose up -d --build` brings up Postgres, MinIO, Redpanda, and kyma
(with the Web UI baked in). Browse **http://localhost:8080**, paste a
bearer token in Settings, and start writing KQL.

For local web development with hot reload:

```bash
cargo run -p kyma-bin --features web-ui       # server on :8080 + :9090
pnpm -C web dev                                # Vite dev server on :5173 (proxies /v1 + /flight)
```

Desktop:

```bash
pnpm -C web tauri dev     # desktop app against your local cargo-run server
pnpm -C web tauri build   # produces .dmg / .msi / .AppImage under web/src-tauri/target/release/bundle/
```
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: web UI quick start"
```

---

## Phase 13 — CI

### Task 13.1 — `web.yml` workflow

**Files:**
- Create: `.github/workflows/web.yml`

- [ ] **Step 1: Write workflow**

```yaml
name: web
on:
  pull_request:
    paths: ["web/**", ".github/workflows/web.yml", "pnpm-workspace.yaml"]
  push:
    branches: [main]
    paths: ["web/**", ".github/workflows/web.yml"]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: "pnpm" }
      - run: pnpm install --frozen-lockfile
      - run: pnpm -C web typecheck
      - run: pnpm -C web lint
      - run: pnpm -C web test
      - run: pnpm -C web build
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/web.yml
git commit -m "ci: web typecheck + lint + tests + build"
```

### Task 13.2 — `release.yml` Tauri build matrix

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Write workflow**

```yaml
name: release-desktop
on:
  push:
    tags: ["v*"]
jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-22.04, macos-14, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: "pnpm" }
      - uses: dtolnay/rust-toolchain@stable
      - name: Linux deps
        if: matrix.os == 'ubuntu-22.04'
        run: sudo apt-get update && sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev librsvg2-dev
      - run: pnpm install --frozen-lockfile
      - run: pnpm -C web build
      - run: pnpm -C web tauri build
      - uses: actions/upload-artifact@v4
        with:
          name: kyma-${{ matrix.os }}
          path: |
            web/src-tauri/target/release/bundle/**
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: release-desktop Tauri build matrix"
```

---

## Phase 14 — Playwright E2E

### Task 14.1 — Scaffold Playwright

**Files:**
- Create: `web/playwright.config.ts`
- Create: `web/e2e/golden-path.spec.ts`
- Modify: `web/package.json`

- [ ] **Step 1: Install**

```bash
pnpm -C web add -D @playwright/test@1.48.0
pnpm -C web exec playwright install chromium
```

- [ ] **Step 2: Write `web/playwright.config.ts`**

```ts
import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  fullyParallel: false,
  use: {
    baseURL: process.env.KYMA_WEB_URL ?? "http://localhost:8080",
    trace: "retain-on-failure",
  },
});
```

- [ ] **Step 3: Write `web/e2e/golden-path.spec.ts`**

```ts
import { test, expect } from "@playwright/test";

const TOKEN    = process.env.KYMA_E2E_TOKEN ?? "";
const ENDPOINT = process.env.KYMA_WEB_URL  ?? "http://localhost:8080";

test("settings → run KQL → see results → share URL → reload restores", async ({ page, context }) => {
  await page.goto("/");
  await expect(page).toHaveURL(/\/settings/);

  await page.fill("input#endpoint", ENDPOINT);
  await page.fill("input#token",    TOKEN);
  await page.fill("input#database", "obs");
  await page.getByRole("button", { name: /save \+ connect/i }).click();

  await expect(page).toHaveURL(/\/explore/);

  // write a query, run it
  await page.locator(".monaco-editor").click();
  await page.keyboard.type("otel_logs | take 5");
  await page.keyboard.press("Meta+Enter");

  await expect(page.getByText(/rows ·/)).toBeVisible({ timeout: 15_000 });

  // URL encodes the state
  const url = page.url();
  expect(url).toMatch(/q=[A-Za-z0-9_\-]+/);

  // reload → same state restored
  await page.goto(url);
  await expect(page.locator(".monaco-editor")).toContainText("take 5");
});
```

- [ ] **Step 4: Add `package.json` scripts**

Add to `web/package.json`:
```json
"e2e": "playwright test",
"e2e:ui": "playwright test --ui"
```

- [ ] **Step 5: Smoke**

```bash
docker-compose up -d --build
KYMA_E2E_TOKEN="" KYMA_AUTH_DISABLED=1 pnpm -C web e2e
docker-compose down
```

- [ ] **Step 6: Commit**

```bash
git add web/playwright.config.ts web/e2e web/package.json pnpm-lock.yaml
git commit -m "test(web): Playwright golden-path E2E"
```

### Task 14.2 — E2E in CI

**Files:**
- Modify: `.github/workflows/web.yml`

- [ ] **Step 1: Append an `e2e` job**

```yaml
  e2e:
    runs-on: ubuntu-latest
    needs: [check]
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: "pnpm" }
      - run: pnpm install --frozen-lockfile
      - run: pnpm -C web exec playwright install --with-deps chromium
      - name: Start stack
        run: docker compose up -d --build
      - name: Wait for kyma
        run: until curl -sS http://localhost:8080/health; do sleep 2; done
      - env: { KYMA_E2E_TOKEN: "" }
        run: pnpm -C web e2e
      - if: always()
        run: docker compose logs kyma > kyma.log
      - if: failure()
        uses: actions/upload-artifact@v4
        with: { name: e2e-traces, path: web/test-results }
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/web.yml
git commit -m "ci: run Playwright E2E against compose stack"
```

---

## Final validation

### Task 15 — End-to-end sanity pass

- [ ] **Step 1: Clean build**

```bash
pnpm install --frozen-lockfile
pnpm -C web typecheck
pnpm -C web build
cargo build -p kyma-bin --release --features web-ui
```

- [ ] **Step 2: Boot the stack**

```bash
docker-compose up -d --build
curl -sS http://localhost:8080/health
open http://localhost:8080    # macOS — or xdg-open on Linux
```
Walk the golden path manually: settings → token (or auth-disabled) → /explore → run a query → grid + chart render → share URL → reload → state restores.

- [ ] **Step 3: Shut down**

```bash
docker-compose down
```

- [ ] **Step 4: Merge / push**

Rebase onto `main` and push the branch. Open a PR against `main`.

---

## Self-review notes (for the author)

The following mappings show every spec section is covered by a task:

| Spec section | Task(s) |
|---|---|
| §1 System architecture (single binary, Tauri, docker) | 2.1, 3.1–3.3, 11.1, 12.1 |
| §2.1 `/v1/catalog/schema` | 4.1–4.3 |
| §2.2 tonic-web | 5.1 |
| §2.3 static UI serving | 3.2–3.3 |
| §3 Frontend structure, state, SDK | 1.1–1.4, 6.1–6.5 |
| §4 Query Workspace UX | 7.1–7.2, 8.1–8.6, 9.1–9.4 |
| §5 Data flow end-to-end | 8.4 (runner), 10.1 (reconnect) |
| §6 Error/loading states | 10.1–10.2 |
| §7 Testing strategy | 3.2 (web_assets), 4.2 (catalog_it), 4.3 (catalog_http), 5.1 (flight_web_smoke), 6.2–6.5 (unit), 14.1–14.2 (E2E) |
| §8 Packaging | 11.1–11.2 (Tauri), 12.1–12.3 (Docker/compose/README), 13.1–13.2 (CI) |

No TBDs remain in the plan. Task 9.3 integrates the URL codec into the explore page; Task 9.2 adds keyboard shortcuts referenced in Task 4's UX; Task 6.4's Flight client buffers-then-decodes (incremental decode noted as a follow-up).
