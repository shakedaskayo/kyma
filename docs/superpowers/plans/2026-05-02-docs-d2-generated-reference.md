# Docs D2 — Generated Reference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Prerequisite:** [Docs D1 Core Content](2026-05-02-docs-d1-core-content.md) is complete and committed. Strongly recommended that DB M0 has shipped before D2 (so `Capabilities` and the schema-mapping `TYPE_MAP` arrays exist for the generators to read).

**Goal:** Eliminate documentation drift on mechanical reference content. Add a `docs-export` subcommand to `kyma-cli` that emits six JSON files (HTTP routes, CLI commands, env vars, KQL functions, pushdown capabilities, schema mappings) into `docs/site/.vitepress/data/generated/`. Build six Vue components that render those JSONs as structured doc blocks. Wire a CI gate that fails any PR that changes a public surface without regenerating. The Reference section becomes a thin shell of hand-written intros embedding generated tables; every connector page can use `<PushdownMatrix>` and `<SchemaMappingTable>`.

**Architecture:** A new binary subcommand `kyma-cli docs-export` introspects compiled-in metadata (axum routes, clap derive, KQL function registry, `Capabilities` per engine, `TYPE_MAP` const arrays) and writes deterministic JSON. A small consolidation in `kyma-bin` puts every env var on a single `KymaEnv` struct with `#[doc = "..."]` attributes so the generator picks them up. Six Vue components consume the JSON. CI runs the exporter, then a `git diff --exit-code` gate; if the diff isn't empty the PR fails with "run docs-export and commit."

**Tech Stack:** Rust 1.95 + clap (existing), `serde_json`, `proc-macro2`/`syn` (only if needed; the existing `kyma-cli` uses derive `clap`, and we can introspect via `clap::Command`). VitePress data files, Vue 3.5. Spec: [`docs/superpowers/specs/2026-05-02-kyma-docs-site-design.md`](../specs/2026-05-02-kyma-docs-site-design.md).

---

## File Structure

**New files:**

- `crates/kyma-cli/src/docs_export/mod.rs` — `docs-export` subcommand entry; routes to per-target sub-subcommands.
- `crates/kyma-cli/src/docs_export/api.rs` — extracts axum routes from `kyma-server`.
- `crates/kyma-cli/src/docs_export/cli.rs` — extracts clap commands.
- `crates/kyma-cli/src/docs_export/env.rs` — extracts env vars from a consolidated `KymaEnv` struct.
- `crates/kyma-cli/src/docs_export/kql.rs` — walks the kyma-kql parser registry.
- `crates/kyma-cli/src/docs_export/capabilities.rs` — emits `Capabilities` per engine.
- `crates/kyma-cli/src/docs_export/schema_mappings.rs` — emits `TYPE_MAP` per engine.
- `crates/kyma-bin/src/env.rs` — consolidated `KymaEnv` struct with `#[doc]` attributes on every field.
- `docs/site/.vitepress/theme/components/ApiEndpoint.vue`
- `docs/site/.vitepress/theme/components/CliCommand.vue`
- `docs/site/.vitepress/theme/components/EnvVarTable.vue`
- `docs/site/.vitepress/theme/components/PushdownMatrix.vue`
- `docs/site/.vitepress/theme/components/SchemaMappingTable.vue`
- `docs/site/.vitepress/theme/components/KqlFunctionTable.vue`
- `docs/site/scripts/verify-generated.mjs` — CI gate (diff check).
- `docs/site/.vitepress/data/generated/.gitkeep`
- `.github/workflows/docs-generated.yml` (or extend existing CI workflow) — runs the gate.
- `docs/site/reference/api/index.md` — section landing.
- `docs/site/reference/cli/index.md`
- `docs/site/reference/env/index.md`
- `docs/site/reference/kql-functions/index.md`

**Modified files:**

- `crates/kyma-cli/src/main.rs` — register the new subcommand.
- `crates/kyma-cli/Cargo.toml` — depend on `kyma-server`, `kyma-kql`, `kyma-connectors`, `kyma-federation` so the exporter can introspect them. (Avoid circular deps; if a circular dep arises, route through a small `kyma-meta` crate that all three populate.)
- `crates/kyma-bin/src/main.rs` — read env via the new `KymaEnv` struct (refactor existing `std::env::var` calls).
- `docs/site/.vitepress/theme/index.ts` — register the new components.
- `docs/site/reference/index.md` — populate with cards linking to the four reference subsections.

---

## Task 1: Consolidated `KymaEnv` struct

**Files:**
- Create: `crates/kyma-bin/src/env.rs`
- Modify: `crates/kyma-bin/src/main.rs`

- [ ] **Step 1:** Read existing env-var usage in `crates/kyma-bin/src/main.rs` (per inventory: `KYMA_CATALOG_URL`, `KYMA_HTTP_ADDR`, `KYMA_GRPC_ADDR`, `KYMA_OTLP_ADDR`, `KYMA_OTLP_DATABASE`, `KYMA_PATH_PREFIX`, `KYMA_AUTH_TOKENS`, `KYMA_STAGING_DISABLED`, `KYMA_COMPACTION_IDLE_SLEEP_MS`, plus the `KYMA_S3_*` set).

- [ ] **Step 2: Define the struct** with `#[doc]` per field:

```rust
//! Consolidated kyma environment configuration.
//! Single source of truth for `kyma-cli docs-export env`.

#[derive(Debug, Clone, kyma_meta_derive::DocEnv)]   // see Task 2 for the macro; if you don't want a custom derive, use a manual list
pub struct KymaEnv {
    /// Postgres connection string for the catalog. Required.
    /// Default: `postgres://kyma:kyma_dev@localhost:5433/kyma`
    pub catalog_url: String,

    /// HTTP server listen address. Default: `0.0.0.0:8080`.
    pub http_addr: String,

    /// Arrow Flight / gRPC server listen address. Set to `off` to disable.
    /// Default: `0.0.0.0:9090`.
    pub grpc_addr: String,

    /// OTLP gRPC listen address (e.g., `0.0.0.0:4317`). Set to `off` to disable.
    /// Default: `off`.
    pub otlp_addr: String,

    /// Target database for auto-created OTLP tables. Default: `default`.
    pub otlp_database: String,

    // ... etc.
}

impl KymaEnv {
    pub fn from_env() -> Self {
        Self {
            catalog_url: std::env::var("KYMA_CATALOG_URL").unwrap_or_else(|_| "postgres://kyma:kyma_dev@localhost:5433/kyma".into()),
            http_addr: std::env::var("KYMA_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            grpc_addr: std::env::var("KYMA_GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:9090".into()),
            otlp_addr: std::env::var("KYMA_OTLP_ADDR").unwrap_or_else(|_| "off".into()),
            otlp_database: std::env::var("KYMA_OTLP_DATABASE").unwrap_or_else(|_| "default".into()),
            // ...
        }
    }

    /// Returns a const list of (name, doc, default) entries for docs-export.
    /// Hand-maintained alongside the struct; Task 2 adds an architectural test
    /// that fails when struct fields and the metadata list drift.
    pub const METADATA: &'static [(&'static str, &'static str, &'static str)] = &[
        ("KYMA_CATALOG_URL", "Postgres connection string for the catalog. Required.", "postgres://kyma:kyma_dev@localhost:5433/kyma"),
        ("KYMA_HTTP_ADDR", "HTTP server listen address.", "0.0.0.0:8080"),
        ("KYMA_GRPC_ADDR", "Arrow Flight / gRPC server listen address. `off` to disable.", "0.0.0.0:9090"),
        ("KYMA_OTLP_ADDR", "OTLP gRPC listen address. `off` to disable.", "off"),
        ("KYMA_OTLP_DATABASE", "Target database for auto-created OTLP tables.", "default"),
        ("KYMA_PATH_PREFIX", "Object-store path prefix.", "kyma"),
        ("KYMA_AUTH_TOKENS", "Bearer tokens (space-separated). Auth disabled when empty.", ""),
        ("KYMA_STAGING_DISABLED", "Set to `1` to bypass group-commit staging buffer (testing only).", ""),
        ("KYMA_COMPACTION_IDLE_SLEEP_MS", "Override compaction worker sleep (testing).", ""),
        ("KYMA_S3_ENDPOINT", "Object store S3 endpoint URL.", ""),
        ("KYMA_S3_BUCKET", "Object store bucket name.", ""),
        ("KYMA_S3_REGION", "Object store region.", ""),
        ("KYMA_S3_ACCESS_KEY_ID", "Object store access key.", ""),
        ("KYMA_S3_SECRET_ACCESS_KEY", "Object store secret key.", ""),
        ("KYMA_S3_PATH_STYLE", "Use path-style URLs (`true`/`false`).", "false"),
        ("KYMA_S3_ALLOW_HTTP", "Allow plaintext object-store URLs (`true`/`false`).", "false"),
    ];
}
```

(Choose either: a custom derive macro, or the const-array approach + an architectural test. Const array is simpler — go with it.)

- [ ] **Step 3: Refactor `main.rs`** to call `KymaEnv::from_env()` once and read fields off it.

- [ ] **Step 4: Architectural test**

`crates/kyma-bin/tests/env_metadata_in_sync.rs`:

```rust
#[test]
fn metadata_lists_every_kyma_env_var_used() {
    // Grep src/main.rs for `KYMA_*` literals; assert each appears in METADATA.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs")).unwrap();
    let re = regex::Regex::new(r"KYMA_[A-Z0-9_]+").unwrap();
    let mut used: std::collections::BTreeSet<String> = re.find_iter(&src).map(|m| m.as_str().to_string()).collect();

    let metadata: std::collections::BTreeSet<String> = kyma_bin::env::KymaEnv::METADATA.iter()
        .map(|(name, _, _)| name.to_string()).collect();

    used.retain(|n| !metadata.contains(n));
    assert!(used.is_empty(), "env vars used in code but not in METADATA: {used:?}");
}
```

(Add `regex = "1"` as a dev-dependency.)

- [ ] **Step 5: Run + commit**

```bash
cargo test -p kyma-bin --test env_metadata_in_sync
git add crates/kyma-bin/src/env.rs crates/kyma-bin/src/main.rs crates/kyma-bin/tests/env_metadata_in_sync.rs crates/kyma-bin/Cargo.toml
git commit -m "feat(docs): consolidated KymaEnv with metadata-in-sync test"
```

---

## Task 2: `docs-export` subcommand entry

**Files:**
- Create: `crates/kyma-cli/src/docs_export/mod.rs`
- Modify: `crates/kyma-cli/src/main.rs`
- Modify: `crates/kyma-cli/Cargo.toml`

- [ ] **Step 1: Wire subcommand into clap.** Read `crates/kyma-cli/src/main.rs` to identify the existing `Subcommand` enum; add:

```rust
DocsExport(docs_export::Args),
```

- [ ] **Step 2: Write `docs_export/mod.rs`**

```rust
use clap::Args as ClapArgs;
use std::path::PathBuf;

pub mod api;
pub mod cli;
pub mod env;
pub mod kql;
pub mod capabilities;
pub mod schema_mappings;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Output directory for generated JSON.
    #[arg(long, default_value = "docs/site/.vitepress/data/generated")]
    pub out: PathBuf,
    /// Which target to export. Default: `all`.
    #[arg(long, default_value = "all")]
    pub target: String,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.out)?;
    let targets: &[&str] = if args.target == "all" {
        &["api", "cli", "env", "kql", "capabilities", "schema-mappings"]
    } else {
        std::slice::from_ref(&args.target.as_str())
    };
    for t in targets {
        match *t {
            "api" => api::write(&args.out)?,
            "cli" => cli::write(&args.out)?,
            "env" => env::write(&args.out)?,
            "kql" => kql::write(&args.out)?,
            "capabilities" => capabilities::write(&args.out)?,
            "schema-mappings" => schema_mappings::write(&args.out)?,
            other => anyhow::bail!("unknown target: {other}"),
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Add deps to `kyma-cli/Cargo.toml`**

```toml
kyma-server = { workspace = true }
kyma-kql = { workspace = true }
kyma-connectors = { workspace = true, features = ["postgres", "mysql", "mongo"] }
kyma-federation = { workspace = true, optional = false }
kyma-bin = { workspace = true }
```

(If circular deps appear, restructure `kyma-cli` to be a separate root or move env-metadata into a leaf crate.)

- [ ] **Step 4: Build**

```bash
cargo build -p kyma-cli
```

Expected: PASS.

- [ ] **Step 5: Commit**

---

## Task 3: API export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/api.rs`
- Modify: `crates/kyma-server/src/lib.rs` (expose route metadata)

- [ ] **Step 1: Expose route metadata from `kyma-server`.** Axum routes don't introspect cleanly — easiest is a `pub const ROUTES: &[ApiRoute] = &[...]` list in `kyma-server/src/lib.rs` that mirrors the actual `Router::new().route(...)` calls. Architectural test ensures parity.

```rust
// crates/kyma-server/src/api_metadata.rs
pub struct ApiRoute {
    pub method: &'static str,
    pub path: &'static str,
    pub auth: AuthRequirement,
    pub doc: &'static str,
    pub content_types: &'static [&'static str],
}

pub enum AuthRequirement { None, Bearer, BearerOptional }

pub const ROUTES: &[ApiRoute] = &[
    ApiRoute { method: "GET",    path: "/health",                auth: AuthRequirement::None,           doc: "Liveness check; always public.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/metrics",               auth: AuthRequirement::None,           doc: "Prometheus exposition; always public.", content_types: &["text/plain"] },
    ApiRoute { method: "POST",   path: "/v1/query",              auth: AuthRequirement::BearerOptional, doc: "Run a KQL or SQL query. Read-only. Content-Type selects language.", content_types: &["application/sql", "application/x-kql"] },
    ApiRoute { method: "POST",   path: "/v1/ingest",             auth: AuthRequirement::Bearer,         doc: "Ingest NDJSON rows. Headers: X-Database, X-Table, X-Idempotency-Key.", content_types: &["application/x-ndjson"] },
    ApiRoute { method: "GET",    path: "/v1/catalog/schema",     auth: AuthRequirement::BearerOptional, doc: "Returns the catalog schema tree (databases → tables → columns).", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/agent/ask",          auth: AuthRequirement::Bearer,         doc: "Natural-language query; SSE response.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/agent/runs/:run_id", auth: AuthRequirement::Bearer,         doc: "Fetch persisted agent run trace.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/dashboards",         auth: AuthRequirement::Bearer,         doc: "List dashboards.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/dashboards",         auth: AuthRequirement::Bearer,         doc: "Create dashboard.", content_types: &["application/json"] },
    ApiRoute { method: "PATCH",  path: "/v1/dashboards/:id",     auth: AuthRequirement::Bearer,         doc: "Update dashboard.", content_types: &["application/json"] },
    ApiRoute { method: "DELETE", path: "/v1/dashboards/:id",     auth: AuthRequirement::Bearer,         doc: "Delete dashboard.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/connectors",         auth: AuthRequirement::Bearer,         doc: "Create connector.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/connectors",         auth: AuthRequirement::Bearer,         doc: "List connectors.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/connectors/:id",     auth: AuthRequirement::Bearer,         doc: "Get connector.", content_types: &["application/json"] },
    ApiRoute { method: "PATCH",  path: "/v1/connectors/:id",     auth: AuthRequirement::Bearer,         doc: "Update connector.", content_types: &["application/json"] },
    ApiRoute { method: "DELETE", path: "/v1/connectors/:id",     auth: AuthRequirement::Bearer,         doc: "Delete connector.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/connectors/:id/pause",          auth: AuthRequirement::Bearer, doc: "Pause connector. Query: scope=sync|federation|all.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/connectors/:id/resume",         auth: AuthRequirement::Bearer, doc: "Resume connector.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/connectors/:id/trigger",        auth: AuthRequirement::Bearer, doc: "Force a connector tick.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/connectors/:id/status",         auth: AuthRequirement::Bearer, doc: "Structured health doc.", content_types: &["application/json"] },
    ApiRoute { method: "GET",    path: "/v1/connectors/:id/events",         auth: AuthRequirement::Bearer, doc: "Last 100 connector state-transition events.", content_types: &["application/json"] },
    ApiRoute { method: "POST",   path: "/v1/connectors/:id/test-connection", auth: AuthRequirement::Bearer, doc: "Run ExternalSource::health synchronously.", content_types: &["application/json"] },
    // /flight/* lives under tonic — skip from this list (separate Flight reference page).
];
```

- [ ] **Step 2: Architectural test in `kyma-server`** that asserts every `Router::new().route(...)` literal in `lib.rs` (and adjacent connector router files) appears in `ROUTES`. Same shape as the env metadata test.

- [ ] **Step 3: Implement `docs_export::api::write`**

```rust
use std::path::Path;
use serde::Serialize;
use kyma_server::api_metadata::{ROUTES, ApiRoute, AuthRequirement};

#[derive(Serialize)]
struct Out<'a> {
    method: &'a str,
    path: &'a str,
    auth: &'static str,
    doc: &'a str,
    content_types: &'a [&'a str],
}

pub fn write(out: &Path) -> anyhow::Result<()> {
    let entries: Vec<Out> = ROUTES.iter().map(|r| Out {
        method: r.method, path: r.path,
        auth: match r.auth { AuthRequirement::None => "none", AuthRequirement::Bearer => "bearer", AuthRequirement::BearerOptional => "bearer-optional" },
        doc: r.doc,
        content_types: r.content_types,
    }).collect();
    let json = serde_json::to_string_pretty(&entries)?;
    std::fs::write(out.join("api-routes.json"), json)?;
    Ok(())
}
```

- [ ] **Step 4: Run + commit**

```bash
cargo run -p kyma-cli -- docs-export --target api
ls docs/site/.vitepress/data/generated/api-routes.json
git add crates/kyma-server/src/api_metadata.rs crates/kyma-server/src/lib.rs crates/kyma-cli/src/docs_export/api.rs docs/site/.vitepress/data/generated/api-routes.json
git commit -m "feat(docs): docs-export api with architectural sync gate"
```

---

## Task 4: CLI export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/cli.rs`

- [ ] **Step 1: Walk the clap command tree.** clap exposes `Command::get_subcommands()`, `get_arguments()` etc. Walk recursively, emit:

```json
[
  {
    "name": "create-database",
    "about": "Create a new database in the catalog.",
    "args": [
      {"name": "name", "help": "Database name", "kind": "positional", "required": true}
    ]
  },
  {
    "name": "create-table",
    "about": "Create a new table.",
    "args": [
      {"name": "db", "help": "Database name", "kind": "long", "required": true, "long": "db"},
      ...
    ]
  }
]
```

- [ ] **Step 2: Run + verify**

```bash
cargo run -p kyma-cli -- docs-export --target cli
cat docs/site/.vitepress/data/generated/cli-commands.json
```

- [ ] **Step 3: Commit**

---

## Task 5: Env vars export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/env.rs`

- [ ] **Step 1:** Read `kyma_bin::env::KymaEnv::METADATA`; emit JSON `[{name, doc, default}, ...]`.

- [ ] **Step 2: Run + commit.**

---

## Task 6: KQL functions export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/kql.rs`
- Modify: `crates/kyma-kql/src/lib.rs` (add public registry walker)

- [ ] **Step 1: Expose registry from `kyma-kql`** — a public function `pub fn list_functions() -> Vec<KqlFunctionDoc>` that yields `(name, signature, doc, since)` for every registered KQL operator/function.

- [ ] **Step 2: Implement export.**

- [ ] **Step 3: Architectural test** — every operator parsed by the KQL grammar appears in the registry.

- [ ] **Step 4: Commit.**

---

## Task 7: Capabilities export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/capabilities.rs`

- [ ] **Step 1:** For each registered `ExternalSource` impl (Postgres, MySQL, Mongo): instantiate, call `.capabilities()`, serialize to JSON. Emit `pushdown-capabilities.json` keyed by `type_id`:

```json
{
  "postgres": {"filter_ops": ["Eq", "NotEq", ...], "agg_funcs": ["Count", "Sum", ...], "limit": true, "order_by": true, ...},
  "mysql":    {...},
  "mongo":    {...}
}
```

- [ ] **Step 2: Commit.**

---

## Task 8: Schema mappings export

**Files:**
- Modify: `crates/kyma-cli/src/docs_export/schema_mappings.rs`

- [ ] **Step 1:** For each engine, read its `TYPE_MAP` const array (already shipped in M1/M2/M3). Emit:

```json
{
  "postgres": [{"source": "smallint", "kyma": "int", "notes": "16-bit; widened"}, ...],
  "mysql":    [...],
  "mongo":    [...]
}
```

- [ ] **Step 2: Commit.**

---

## Task 9: Vue components for generated content

**Files:**
- Create: 6 Vue components under `docs/site/.vitepress/theme/components/`
- Modify: `docs/site/.vitepress/theme/index.ts` (register them)

- [ ] **Step 1: `ApiEndpoint.vue`** — props: `route` (e.g., `"POST /v1/query"`). Looks up the entry in `api-routes.json` and renders a structured block: method/path, auth requirement, content types, doc.

- [ ] **Step 2: `CliCommand.vue`** — prop `name`. Renders one CLI command with all flags as a table.

- [ ] **Step 3: `EnvVarTable.vue`** — no props. Renders the full env var table.

- [ ] **Step 4: `PushdownMatrix.vue`** — prop `engine` (e.g., `"postgres"`). Renders a matrix of supported operators / functions / agg / sort/limit/join.

- [ ] **Step 5: `SchemaMappingTable.vue`** — prop `engine`. Renders the type map as a sortable table.

- [ ] **Step 6: `KqlFunctionTable.vue`** — no props. Renders the full KQL function index.

- [ ] **Step 7: Register in `theme/index.ts`.**

- [ ] **Step 8: Verify** by adding one usage of each in a placeholder reference page.

- [ ] **Step 9: Commit.**

---

## Task 10: Reference section pages

**Files:**
- Create: `docs/site/reference/api/index.md`
- Create: `docs/site/reference/cli/index.md`
- Create: `docs/site/reference/env/index.md`
- Create: `docs/site/reference/kql-functions/index.md`
- Modify: `docs/site/reference/index.md`
- Modify: `docs/site/.vitepress/config.ts` (sidebar)

- [ ] **Step 1: Each reference subsection page** — short intro + the corresponding generated component. Example for `reference/api/index.md`:

```markdown
---
title: HTTP API reference
description: Every kyma HTTP endpoint, with authentication, content types, and request/response shapes.
---

# HTTP API reference

All endpoints are versioned under `/v1/`. Authentication is bearer token via
`Authorization: Bearer <token>` when `KYMA_AUTH_TOKENS` is set.

For each endpoint below, the auth column reads:

- **none** — public; no token required.
- **bearer** — token required.
- **bearer-optional** — token recommended; works without when `KYMA_AUTH_TOKENS` is empty.

<ApiEndpoint route="GET /health" />
<ApiEndpoint route="GET /metrics" />
<ApiEndpoint route="POST /v1/query" />
<ApiEndpoint route="POST /v1/ingest" />
<ApiEndpoint route="GET /v1/catalog/schema" />
<ApiEndpoint route="POST /v1/agent/ask" />
<ApiEndpoint route="GET /v1/agent/runs/:run_id" />
... etc.
```

- [ ] **Step 2: Wire sidebar.**

- [ ] **Step 3: Commit.**

---

## Task 11: CI gate — `verify-generated.mjs`

**Files:**
- Create: `docs/site/scripts/verify-generated.mjs`
- Modify: `docs/site/package.json` (`check:generated` script + add to `build`)
- Create: `.github/workflows/docs-generated.yml`

- [ ] **Step 1: Verify script**

Write `docs/site/scripts/verify-generated.mjs`:

```js
#!/usr/bin/env node
import { execSync } from 'node:child_process'
import { existsSync } from 'node:fs'

const generatedDir = '.vitepress/data/generated'
const required = ['api-routes.json', 'cli-commands.json', 'env-vars.json', 'kql-functions.json', 'pushdown-capabilities.json', 'schema-mappings.json']
let missing = required.filter(f => !existsSync(`${generatedDir}/${f}`))
if (missing.length > 0) {
  console.error(`Missing generated files: ${missing.join(', ')}\nRun: cargo run -p kyma-cli -- docs-export`)
  process.exit(1)
}
// Diff check
try {
  execSync(`git diff --exit-code -- ${generatedDir}`, { stdio: 'inherit' })
} catch {
  console.error(`Generated files are out of date.\nRun: cargo run -p kyma-cli -- docs-export\nThen commit the changes.`)
  process.exit(1)
}
console.log('generated files in sync ✓')
```

- [ ] **Step 2: Wire into build**

`docs/site/package.json`:

```json
"scripts": {
  "build": "npm run check:diagrams && npm run check:generated && vitepress build",
  "check:generated": "node scripts/verify-generated.mjs",
  ...
}
```

- [ ] **Step 3: GitHub workflow**

`.github/workflows/docs-generated.yml`:

```yaml
name: docs-generated
on:
  pull_request:
    paths:
      - 'crates/**'
      - 'docs/site/**'

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Regenerate
        run: cargo run -p kyma-cli -- docs-export
      - name: Verify clean diff
        run: git diff --exit-code -- docs/site/.vitepress/data/generated
```

- [ ] **Step 4: Commit.**

---

## Task 12: D2 acceptance smoke

- [ ] `cargo run -p kyma-cli -- docs-export` produces all six JSON files.
- [ ] `cargo test --workspace` includes the architectural sync tests for env, api routes, KQL.
- [ ] `pnpm build` in `docs/site/` runs the `check:generated` step and passes.
- [ ] Reference pages render generated tables correctly.
- [ ] `<PushdownMatrix engine="postgres" />` renders only when M1 (or later) has shipped — for D2-before-M1, the JSON is an empty object `{}` and the component shows "Engine not yet implemented."
- [ ] CI workflow `docs-generated.yml` runs on PRs and gates merges.
- [ ] Tag `docs-d2-generated-reference`.

---

## D2 Open Decisions

- **What if `kyma-cli` depending on `kyma-server` creates a circular dep?** Most likely path: `kyma-server` -> X -> `kyma-cli`? Unlikely (cli doesn't currently depend on server-side connectors). If it does emerge, factor the metadata into a leaf `kyma-meta` crate that all three populate. Decide at Task 2 time.
- **Whether to use a custom derive macro for `KymaEnv` or stick with the const-array + arch test.** Const array wins for simplicity in v1; revisit only if the env-var count exceeds ~30.
- **Frequency of CI gate.** `pull_request` paths filter included. Consider also `push` to main as a backstop.
