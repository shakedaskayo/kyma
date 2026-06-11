# Data Sources Rename + Sectioned Module — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename Kyma's "connectors" concept to "Data Sources" everywhere (clean break, no aliases) and ship a tabbed `/data-sources` web module with Sources / File Watchers / Claude Code Memory Sync / Memories tabs, backed by a new watcher registry.

**Architecture:** Mechanical rename sweeps move bottom-up (crates → symbols → DB/SQL → routes/tools/CLI → TS client → web → docs), each leaving the workspace green. New functionality is three small verticals: a `data_source_watchers` Postgres table + `WatcherRegistry` (server mode) with an in-memory twin in kyma-local (cc-sync), a `GET /v1/data-sources/watchers` endpoint in both modes, and a memory source-summary aggregate endpoint. Web work re-homes the existing connectors UX as the Sources tab and adds three new tabs.

**Tech Stack:** Rust (axum, sqlx/Postgres, tokio, testcontainers), SQLite (kyma-local), TypeScript (`@kyma-ai/client`), React + TanStack Router/Query + Tailwind/shadcn, VitePress docs.

**Spec:** `docs/superpowers/specs/2026-06-11-data-sources-rename-design.md`

**Plan-level deviation from spec (approved rationale inline):** the spec put all watcher state in the Postgres table. kyma-local (where cc-sync runs) is embedded SQLite and never sees Postgres, so cc-sync watcher state is held in-process in kyma-local and served through the same endpoint shape. The Postgres table serves server-mode filedrop watchers (multi-node visibility is the point there); a single-process local server needs no persistence for live heartbeat state.

**Working rules for every task:**
- Repo root: `/Users/shakedaskayo/shaked/projects/kyma`. Branch off current `feat/federated-sources` (or a new `feat/data-sources-rename` branch — executor's choice, create once at start).
- `rg` = ripgrep. Use `perl -pi -e` for in-place edits (BSD sed on macOS mangles `-i`).
- After every Rust task: `cargo check --workspace` must pass before commit.
- Rust integration tests need Docker (testcontainers, `pgvector/pgvector:pg16`). Run targeted suites per task, full `cargo test --workspace` only in the final task.

---

## Phase 1 — Rust rename (mechanical, each task green)

### Task 1: Rename the crates

**Files:**
- Rename: `crates/kyma-connectors/` → `crates/kyma-datasources/`
- Rename: `crates/kyma-connector-core/` → `crates/kyma-datasource-core/`
- Modify: root `Cargo.toml` (workspace members + workspace deps), `crates/kyma-server/Cargo.toml`, `crates/kyma-bin/Cargo.toml`, `crates/kyma-jobs/Cargo.toml`, both renamed crates' `Cargo.toml`

- [ ] **Step 1: Move the crate directories**

```bash
cd /Users/shakedaskayo/shaked/projects/kyma
git mv crates/kyma-connectors crates/kyma-datasources
git mv crates/kyma-connector-core crates/kyma-datasource-core
```

- [ ] **Step 2: Rename packages and dependency references**

```bash
# package names inside the moved crates
perl -pi -e 's/name = "kyma-connectors"/name = "kyma-datasources"/' crates/kyma-datasources/Cargo.toml
perl -pi -e 's/name = "kyma-connector-core"/name = "kyma-datasource-core"/' crates/kyma-datasource-core/Cargo.toml
perl -pi -e 's/Stable extension API for third-party kyma connectors/Stable extension API for third-party kyma data sources/' crates/kyma-datasource-core/Cargo.toml

# every Cargo.toml that references them (workspace root + consumers)
rg -l 'kyma-connector' --glob 'Cargo.toml' | xargs perl -pi -e 's/kyma-connectors/kyma-datasources/g; s/kyma-connector-core/kyma-datasource-core/g'
```

- [ ] **Step 3: Fix the Rust `use`/`extern` paths (crate name only — symbols come in Task 2)**

```bash
rg -l 'kyma_connector' --glob '*.rs' | xargs perl -pi -e 's/kyma_connectors/kyma_datasources/g; s/kyma_connector_core/kyma_datasource_core/g'
```

- [ ] **Step 4: Verify build**

Run: `cargo check --workspace`
Expected: clean (warnings ok). If a feature gate like `kyma-bin`'s `github` feature references the old crate name (`crates/kyma-bin/Cargo.toml:58`), step 2's sweep already rewrote it — confirm with `rg 'kyma-connectors|kyma_connectors' --glob '!target' || echo CLEAN`.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor: rename kyma-connectors/kyma-connector-core crates to kyma-datasources/kyma-datasource-core"
```

### Task 2: Rename the Rust symbols

**Files:** all `*.rs` mentioning `Connector*` symbols (~91 files), plus module file renames:
- Rename: `crates/kyma-server/src/agent/connector_tools.rs` → `datasource_tools.rs`
- Rename: `crates/kyma-cli/src/connector.rs` → `datasource.rs`

This task renames **identifiers and doc comments only**. Do NOT touch string literals containing SQL (`FROM connectors`...), route paths (`/v1/connectors`), tool name strings (`"list_connectors"`), or env vars — those land in Tasks 3–6 together with their behavior change.

- [ ] **Step 1: Sweep type/trait/struct names**

```bash
cd /Users/shakedaskayo/shaked/projects/kyma
rg -l 'Connector' --glob '*.rs' | xargs perl -pi -e '
  s/\bConnectorRegistry\b/DataSourceRegistry/g;
  s/\bConnectorRunner\b/DataSourceRunner/g;
  s/\bConnectorControl\b/DataSourceControl/g;
  s/\bConnectorCtx\b/DataSourceCtx/g;
  s/\bConnectorRun\b/DataSourceRun/g;
  s/\bConnectorAdminState\b/DataSourceAdminState/g;
  s/\bConnectorToolCtx\b/DataSourceToolCtx/g;
  s/\bConnectorReadBudget\b/DataSourceReadBudget/g;
  s/\bConnector\b/DataSource/g;
'
```

- [ ] **Step 2: Sweep snake_case identifiers (fn names, vars, modules) — excluding string literals**

`perl` can't easily skip string literals; instead run the sweep, then re-fix literals from git diff. The literal patterns are few and known (SQL `connectors`, `/v1/connectors`, `connector_tick`, `list_connectors`, `connector_read`):

```bash
rg -l 'connector' --glob '*.rs' | xargs perl -pi -e '
  s/connector_tools/datasource_tools/g;
  s/\bconnector_admin_router\b/datasource_admin_router/g;
  s/\bconnector_id\b/data_source_id/g;
  s/\bconnector\b/data_source/g;
  s/\bconnectors\b/data_sources/g;
'
# Now restore the string literals this clobbered (they change in later tasks):
git diff -U0 -- '*.rs' | grep '^\+' | grep -E '"(.*data_source)' | head -50  # inspect
```

Manually revert (Edit tool, file by file) any changed **string literal** back to its original: SQL statements (`"SELECT ... FROM data_sources"` → back to `connectors`), route strings (`"/v1/data-sources"` → back), `"data_source_tick"` → back, tool names/descriptions, and the `payload->>'data_source_id'` SQL fragments. `rg '"' -e 'data_source' --glob '*.rs'` style queries help: `rg 'data_source' --glob '*.rs' -n | rg '"'` and check each hit is a literal we must restore. Comments may keep the new wording.

- [ ] **Step 3: Rename module files and their `mod` declarations**

```bash
git mv crates/kyma-server/src/agent/connector_tools.rs crates/kyma-server/src/agent/datasource_tools.rs
git mv crates/kyma-cli/src/connector.rs crates/kyma-cli/src/datasource.rs
rg -n 'mod connector' crates/kyma-server/src crates/kyma-cli/src
```

Update the `mod connector_tools;` / `mod connector;` declarations (now `mod datasource_tools;` / `mod datasource;`) and their use sites (`connector::Op` → `datasource::Op` etc. — step 2's sweep likely caught the paths; verify).

- [ ] **Step 4: Build + run unit tests for the renamed crate**

Run: `cargo check --workspace && cargo test -p kyma-datasources --lib`
Expected: compiles; lib unit tests pass. Fix residual compile errors (they will all be missed identifier/`mod` mismatches).

- [ ] **Step 5: Verify literals survived intact**

```bash
rg -n '"/v1/connectors' --glob '*.rs' | wc -l   # expect >0 (routes unchanged so far)
rg -n 'FROM connectors|INTO connectors|UPDATE connectors' --glob '*.rs' | wc -l  # expect >0
rg -n 'connector_tick' --glob '*.rs' | wc -l    # expect >0
```

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "refactor: rename Connector* symbols to DataSource* across Rust workspace"
```

### Task 3: DB migration 027 + SQL string sweep

**Files:**
- Create: `crates/kyma-catalog/migrations/027_data_sources_rename.sql` (if another migration landed first, use the next free number and adjust below)
- Modify: every `*.rs` with SQL touching `connectors` / `connector_cursors` / `connector_leases` / `connector_tick` / `payload->>'connector_id'` (admin.rs, catalog_sql.rs, runner.rs, scheduler.rs, agent/datasource_tools.rs, kyma-jobs, tests)
- Test: `crates/kyma-datasources/tests/migration_rename_it.rs` (new)

- [ ] **Step 1: Write the migration**

```sql
-- 027_data_sources_rename.sql
-- Clean-break rename: connectors -> data_sources (concept-wide).

ALTER TABLE connectors RENAME TO data_sources;
ALTER TABLE connector_cursors RENAME TO data_source_cursors;
ALTER TABLE connector_leases RENAME TO data_source_leases;

ALTER TABLE data_source_cursors RENAME COLUMN connector_id TO data_source_id;
ALTER TABLE data_source_leases RENAME COLUMN connector_id TO data_source_id;

ALTER INDEX connectors_enabled_drive_idx RENAME TO data_sources_enabled_drive_idx;

-- Pending tick payloads: rekey connector_id -> data_source_id while the old
-- kind is still in place, then flip the kind.
UPDATE background_tasks
   SET payload = (payload - 'connector_id')
                 || jsonb_build_object('data_source_id', payload->'connector_id')
 WHERE kind = 'connector_tick' AND payload ? 'connector_id';

UPDATE background_tasks SET kind = 'data_source_tick' WHERE kind = 'connector_tick';

DROP INDEX IF EXISTS background_tasks_connector_tick_uniq;
CREATE UNIQUE INDEX IF NOT EXISTS background_tasks_data_source_tick_uniq
    ON background_tasks ((payload->>'data_source_id'),
                         (payload->>'scheduled_for'))
    WHERE kind = 'data_source_tick' AND status IN ('pending', 'claimed');

-- Watcher registry (file watchers / cc-sync provenance). Server-mode only;
-- kyma-local keeps an in-process equivalent.
CREATE TABLE IF NOT EXISTS data_source_watchers (
    id                uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    kind              text NOT NULL CHECK (kind IN ('filedrop','cc_sync')),
    node_host         text NOT NULL,
    node_id           text NOT NULL,
    identity          text NOT NULL,
    config_jsonb      jsonb NOT NULL,
    config_hash       text NOT NULL,
    started_at        timestamptz NOT NULL DEFAULT now(),
    last_heartbeat_at timestamptz NOT NULL DEFAULT now(),
    last_scan_jsonb   jsonb,
    UNIQUE (kind, node_id, config_hash)
);
```

- [ ] **Step 2: Write the failing migration test**

Create `crates/kyma-datasources/tests/migration_rename_it.rs`. Pattern-match the container setup from `tests/admin_it.rs` (`pgvector/pgvector:pg16` via testcontainers). The test executes migration files in order directly from disk, seeds pre-rename rows after 026, applies 027, and asserts survival:

```rust
//! Verifies 027 renames tables in place: rows seeded under the old names
//! survive and queued ticks are rekeyed.
use sqlx::postgres::PgPoolOptions;
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};

#[tokio::test]
async fn rename_migration_preserves_rows() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();

    // Apply migrations from disk in filename order, stopping before 027.
    let mut files: Vec<_> = std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kyma-catalog/migrations"
    ))
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "sql"))
    .collect();
    files.sort();
    let split = files
        .iter()
        .position(|p| p.file_name().unwrap().to_str().unwrap().starts_with("027_"))
        .expect("027 migration present");
    for f in &files[..split] {
        let sql = std::fs::read_to_string(f).unwrap();
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap_or_else(|e| panic!("{f:?}: {e}"));
    }

    // Seed under the OLD names. tenant_id column exists (added post-005);
    // discover required columns at runtime if this INSERT fails and adjust.
    sqlx::raw_sql(
        "INSERT INTO connectors (tenant_id, name, type, target_database, target_table,
                                 config_jsonb, schedule_ms, drive_model)
         VALUES ('00000000-0000-0000-0000-000000000001', 'old-one', 'github', 'db', 't',
                 '{}'::jsonb, 60000, 'periodic');
         INSERT INTO connector_cursors (connector_id, cursor_jsonb)
         SELECT id, '{\"x\":1}'::jsonb FROM connectors WHERE name = 'old-one';
         INSERT INTO background_tasks (kind, status, payload)
         SELECT 'connector_tick', 'pending',
                jsonb_build_object('connector_id', id, 'scheduled_for', 'now')
         FROM connectors WHERE name = 'old-one';",
    )
    .execute(&pool)
    .await
    .unwrap();

    for f in &files[split..] {
        let sql = std::fs::read_to_string(f).unwrap();
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap_or_else(|e| panic!("{f:?}: {e}"));
    }

    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM data_sources WHERE name = 'old-one'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let (c,): (i64,) = sqlx::query_as("SELECT count(*) FROM data_source_cursors").fetch_one(&pool).await.unwrap();
    assert_eq!(c, 1);
    let (t,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM background_tasks
         WHERE kind = 'data_source_tick' AND payload ? 'data_source_id'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(t, 1);
    let (w,): (i64,) = sqlx::query_as("SELECT count(*) FROM data_source_watchers").fetch_one(&pool).await.unwrap();
    assert_eq!(w, 0);
}
```

Note: if `background_tasks` columns differ (check `rg -n 'CREATE TABLE.*background_tasks' -A15 crates/kyma-catalog/migrations/`), adjust the seed INSERT to its real NOT NULL columns. Same for `connectors` extra columns — read the later ALTERs (`rg -n 'ALTER TABLE connectors' crates/kyma-catalog/migrations/`).

- [ ] **Step 3: Run the test — expect failure** (027 doesn't exist yet if you wrote the test first; otherwise it fails on seeded INSERT if columns mismatch — fix seeds until the only failure is the missing migration, then add the migration file from Step 1)

Run: `cargo test -p kyma-datasources --test migration_rename_it -- --nocapture`

- [ ] **Step 4: Sweep the SQL strings + tick kind in code**

```bash
rg -ln "connector" --glob '*.rs' -e 'connectors' -e 'connector_tick' -e "connector_id"
```

For each hit that is a **string literal** (SQL or job kind), update:
- `FROM connectors` / `INTO connectors` / `UPDATE connectors` → `data_sources`
- `connector_cursors` → `data_source_cursors`, `connector_leases` → `data_source_leases`, `connector_id` column refs → `data_source_id`
- `'connector_tick'` / `"connector_tick"` → `data_source_tick`
- `payload->>'connector_id'` → `payload->>'data_source_id'`, and the Rust code building tick payloads (`json!({"connector_id": ...})` → `data_source_id`)

This includes `crates/kyma-datasources/src/{admin,catalog_sql,runner,scheduler}.rs`, `crates/kyma-server/src/agent/datasource_tools.rs` (its `SELECT ... FROM connectors` queries), kyma-jobs, and the integration tests' raw SQL.

- [ ] **Step 5: Run the suites**

Run: `cargo test -p kyma-datasources --test migration_rename_it --test runner_it --test scheduler_it`
Expected: PASS (admin_it still passes too — routes unchanged so far: `cargo test -p kyma-datasources --test admin_it`).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: migrate connectors tables to data_sources + watcher registry table (027)"
```

### Task 4: REST routes `/v1/data-sources`

**Files:**
- Modify: `crates/kyma-datasources/src/admin.rs` (router paths, lines ~22-34), `crates/kyma-server/src/lib.rs:90-108` (nest path if hardcoded), `crates/kyma-datasources/tests/admin_it.rs` (request URIs), any oauth route mentioning connectors

- [ ] **Step 1: Update tests first** — in `admin_it.rs` (and any other test issuing HTTP), replace `"/v1/connectors"` with `"/v1/data-sources"` (all variants: `/catalog`, `/:id`, `/pause`, `/resume`, `/trigger`, `/github/repos`).

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p kyma-datasources --test admin_it`
Expected: FAIL with 404s.

- [ ] **Step 3: Flip the router** — in `admin.rs`, change every route registration `"/v1/connectors..."` → `"/v1/data-sources..."`. Check `kyma-server/src/lib.rs` and `kyma-bin/src/main.rs` for `.nest("/v1/connectors", ...)` or similar mount strings and update. Also `rg -n '/v1/connectors' --glob '*.rs'` must end at zero hits.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p kyma-datasources --test admin_it`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat!: REST surface moves to /v1/data-sources (clean break, no aliases)"
```

### Task 5: MCP/agent tool rename

**Files:**
- Modify: `crates/kyma-server/src/agent/datasource_tools.rs`

- [ ] **Step 1: Rename tool name strings + descriptions**

- `FunctionTool::new("list_connectors", ...)` → `"list_data_sources"`; const `LIST_CONNECTORS_DESC` → `LIST_DATA_SOURCES_DESC`, text: "List the configured data sources (id, name, kind, enabled, target database). Use this to discover which external sources you can read from with `data_source_read` when filling memory gaps."
- `FunctionTool::new("connector_read", ...)` → `"data_source_read"`; const → `DATA_SOURCE_READ_DESC`, args doc `{connector_id, ...}` → `{data_source_id, ...}`, and the JSON arg parsing in the tool body (`args["connector_id"]` → `args["data_source_id"]`). Update "Use list_connectors first" → "Use list_data_sources first".

- [ ] **Step 2: Fix any callers/tests** — `rg -n 'list_connectors|connector_read' --glob '*.rs'` → zero hits after updates (agent prompt templates or dreaming skill docs may reference the tool names: `rg -rn 'list_connectors|connector_read' --glob '!target' .` and update markdown/skill prompts too).

- [ ] **Step 3: Build + targeted test**

Run: `cargo check --workspace && cargo test -p kyma-server agent`
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat!: agent tools renamed to list_data_sources / data_source_read"
```

### Task 6: CLI `kyma datasource`

**Files:**
- Modify: `crates/kyma-cli/src/main.rs` (~lines 24, 139-141, 511-513), `crates/kyma-cli/src/datasource.rs`

- [ ] **Step 1: Rename the subcommand**

In `main.rs`: variant `Connector { op: datasource::Op }` → `Datasource { op: datasource::Op }` (clap derives the literal `datasource` from the variant), doc comment "Manage data sources — add a GitHub/GitLab/Bitbucket repo, list, pause, resume, trigger, remove. See `kyma datasource --help`."; match arm `Command::Connector` → `Command::Datasource`.

In `datasource.rs`: `IngestOp::Status`/`Tail` flag `#[arg(long)] connector: Option<String>` → `#[arg(long)] datasource: Option<String>` (and uses). Update all user-facing strings: help text, table headers, error messages, confirmation prompts (`rg -n 'connector' crates/kyma-cli/src/ -i` → fix every hit). The HTTP paths it calls were updated to `/v1/data-sources` in Task 4 — verify: `rg -n '/v1/' crates/kyma-cli/src/datasource.rs`.

- [ ] **Step 2: Build + smoke**

Run: `cargo build -p kyma-cli && ./target/debug/kyma datasource --help && ./target/debug/kyma ingest status --help`
Expected: help shows `datasource` subcommand with list/add/show/pause/resume/remove/trigger; ingest status shows `--datasource`. `./target/debug/kyma connector --help` exits with "unrecognized subcommand".

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat!: CLI subcommand renamed to kyma datasource; ingest --datasource"
```

### Task 7: Capabilities rename

**Files:**
- Modify: `crates/kyma-server/src/capabilities.rs`

- [ ] **Step 1: Write failing test** (in the existing `mod tests` of capabilities.rs):

```rust
#[test]
fn capabilities_serialize_data_sources() {
    let json = serde_json::to_value(Capabilities::SERVER).unwrap();
    assert_eq!(json["data_sources"], true);
    assert!(json.get("connectors").is_none());
    let json = serde_json::to_value(Capabilities::LOCAL).unwrap();
    assert_eq!(json["data_sources"], false);
}
```

- [ ] **Step 2: Run to verify fail**: `cargo test -p kyma-server capabilities` → FAIL.

- [ ] **Step 3: Rename the field** `pub connectors: bool` → `pub data_sources: bool` in the struct and both consts (`SERVER: data_sources: true`, `LOCAL: data_sources: false`); update the module doc comment. Fix any Rust consumers (`rg -n '\.connectors' --glob '*.rs'`).

- [ ] **Step 4: Run to verify pass**: `cargo test -p kyma-server capabilities` → PASS. `cargo check --workspace` green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat!: capability key connectors -> data_sources"
```

---

## Phase 2 — Watcher registry + new endpoints

### Task 8: `WatcherRegistry` (Postgres, server mode)

**Files:**
- Create: `crates/kyma-datasources/src/watchers.rs`
- Modify: `crates/kyma-datasources/src/lib.rs` (add `pub mod watchers;`), `crates/kyma-datasources/Cargo.toml` (add `gethostname = "0.4"` — check workspace deps first: `rg gethostname Cargo.toml`; sha2 is already used by filedrop, add `sha2` + `hex` to this crate mirroring filedrop's versions)
- Test: `crates/kyma-datasources/tests/watchers_it.rs`

- [ ] **Step 1: Write failing test** `crates/kyma-datasources/tests/watchers_it.rs` (container setup copied from `admin_it.rs::state()` — pool only, no registry needed):

```rust
use kyma_datasources::watchers::{ScanStats, WatcherRegistry};
use sqlx::postgres::PgPoolOptions;
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};

async fn pool() -> (testcontainers::ContainerAsync<Postgres>, sqlx::PgPool) {
    let pg = Postgres::default().with_name("pgvector/pgvector").with_tag("pg16").start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    sqlx::migrate!("../kyma-catalog/migrations").run(&pool).await.unwrap();
    (pg, pool)
}

#[tokio::test]
async fn register_heartbeat_list_prune() {
    let (_pg, pool) = pool().await;
    let reg = WatcherRegistry::register(
        pool.clone(), "filedrop", "host-a", "node-a", "shaked",
        serde_json::json!({"prefixes": ["drops/"], "poll_secs": 5}),
    ).await.unwrap();

    // re-register same identity+config → same row (upsert)
    let reg2 = WatcherRegistry::register(
        pool.clone(), "filedrop", "host-a", "node-a", "shaked",
        serde_json::json!({"prefixes": ["drops/"], "poll_secs": 5}),
    ).await.unwrap();
    assert_eq!(reg.id(), reg2.id());

    reg.heartbeat(Some(&ScanStats {
        seen: 10, processed: 9, errors: 1, duration_ms: 120,
        at: "2026-06-11T00:00:00Z".into(), detail: None,
    })).await;

    let rows = WatcherRegistry::list(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "filedrop");
    assert_eq!(rows[0].identity, "shaked");
    let scan = rows[0].last_scan.as_ref().unwrap();
    assert_eq!(scan["processed"], 9);

    // prune: backdate the heartbeat past 24h, list() sweeps it
    sqlx::query("UPDATE data_source_watchers SET last_heartbeat_at = now() - interval '25 hours'")
        .execute(&pool).await.unwrap();
    let rows = WatcherRegistry::list(&pool).await.unwrap();
    assert!(rows.is_empty());
}
```

- [ ] **Step 2: Run to verify fail**: `cargo test -p kyma-datasources --test watchers_it` → FAIL (module missing).

- [ ] **Step 3: Implement** `crates/kyma-datasources/src/watchers.rs`:

```rust
//! Watcher registry — file watchers (filedrop, cc-sync) register themselves
//! with node + identity provenance and heartbeat each poll cycle. Read-side
//! the UI shows who is feeding the graph from where. Best-effort by design:
//! a registry failure must never break ingestion.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    pub seen: u64,
    pub processed: u64,
    pub errors: u64,
    pub duration_ms: u64,
    /// RFC 3339 timestamp of the scan.
    pub at: String,
    /// Watcher-specific extras (cc-sync packs per-realm report rollups here).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct WatcherRow {
    pub id: Uuid,
    pub kind: String,
    pub node_host: String,
    pub node_id: String,
    pub identity: String,
    pub config: serde_json::Value,
    pub started_at: String,
    pub last_heartbeat_at: String,
    pub last_scan: Option<serde_json::Value>,
    /// heartbeat older than 3x poll interval (poll_secs from config, default 30)
    pub stale: bool,
}

#[derive(Clone)]
pub struct WatcherRegistry {
    pool: PgPool,
    id: Uuid,
}

/// Hostname for registration; KYMA_NODE_ID overrides the node id.
pub fn node_identity() -> (String, String, String) {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let node_id = std::env::var("KYMA_NODE_ID").unwrap_or_else(|_| host.clone());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    (host, node_id, user)
}

impl WatcherRegistry {
    pub fn id(&self) -> Uuid {
        self.id
    }

    pub async fn register(
        pool: PgPool,
        kind: &str,
        node_host: &str,
        node_id: &str,
        identity: &str,
        config: serde_json::Value,
    ) -> Result<Self, sqlx::Error> {
        let config_hash = hex::encode(Sha256::digest(config.to_string().as_bytes()));
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO data_source_watchers
               (kind, node_host, node_id, identity, config_jsonb, config_hash)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (kind, node_id, config_hash) DO UPDATE SET
               node_host = EXCLUDED.node_host,
               identity = EXCLUDED.identity,
               config_jsonb = EXCLUDED.config_jsonb,
               started_at = now(),
               last_heartbeat_at = now()
             RETURNING id",
        )
        .bind(kind)
        .bind(node_host)
        .bind(node_id)
        .bind(identity)
        .bind(&config)
        .bind(config_hash)
        .fetch_one(&pool)
        .await?;
        Ok(Self { pool, id })
    }

    /// Best-effort heartbeat — logs on failure, never errors out to the caller.
    pub async fn heartbeat(&self, scan: Option<&ScanStats>) {
        let scan_json = scan.and_then(|s| serde_json::to_value(s).ok());
        let res = sqlx::query(
            "UPDATE data_source_watchers SET
               last_heartbeat_at = now(),
               last_scan_jsonb = COALESCE($2, last_scan_jsonb)
             WHERE id = $1",
        )
        .bind(self.id)
        .bind(scan_json)
        .execute(&self.pool)
        .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "watcher heartbeat failed");
        }
    }

    /// List live watchers; prunes rows silent for >24h on the way.
    pub async fn list(pool: &PgPool) -> Result<Vec<WatcherRow>, sqlx::Error> {
        sqlx::query("DELETE FROM data_source_watchers WHERE last_heartbeat_at < now() - interval '24 hours'")
            .execute(pool)
            .await?;
        let rows: Vec<(Uuid, String, String, String, String, serde_json::Value, String, String, Option<serde_json::Value>, bool)> =
            sqlx::query_as(
                "SELECT id, kind, node_host, node_id, identity, config_jsonb,
                        to_char(started_at, 'YYYY-MM-DD\"T\"HH24:MI:SSOF'),
                        to_char(last_heartbeat_at, 'YYYY-MM-DD\"T\"HH24:MI:SSOF'),
                        last_scan_jsonb,
                        last_heartbeat_at < now() - make_interval(secs =>
                          3 * COALESCE((config_jsonb->>'poll_secs')::float8, 30.0))
                 FROM data_source_watchers
                 ORDER BY kind, node_host",
            )
            .fetch_all(pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(id, kind, node_host, node_id, identity, config, started_at, last_heartbeat_at, last_scan, stale)| WatcherRow {
                id, kind, node_host, node_id, identity, config, started_at, last_heartbeat_at, last_scan, stale,
            })
            .collect())
    }
}
```

Add to `lib.rs`: `pub mod watchers;`. Note `sqlx::migrate!` path in the test references `../kyma-catalog/migrations` — match however `admin_it.rs`/`PostgresCatalog::connect` runs migrations (if `PostgresCatalog::connect` already migrates, use it instead of `sqlx::migrate!` in the test, exactly as `admin_it.rs` does).

- [ ] **Step 4: Run to verify pass**: `cargo test -p kyma-datasources --test watchers_it` → PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: watcher registry — register/heartbeat/list with node+identity provenance"
```

### Task 9: Filedrop registers + heartbeats

**Files:**
- Modify: `crates/kyma-ingest-filedrop/src/lib.rs` (scan hook + tick stats), `crates/kyma-bin/src/main.rs:1044-1064` (wire registry)
- Modify: `crates/kyma-ingest-filedrop/Cargo.toml` only if it lacks `serde` (the hook passes a plain struct; no dependency on kyma-datasources from filedrop — kyma-bin bridges them)
- Test: extend existing filedrop tests (`ls crates/kyma-ingest-filedrop/tests` / `#[cfg(test)]`) with a hook-invocation unit test

- [ ] **Step 1: Add scan stats + hook to filedrop**

In `crates/kyma-ingest-filedrop/src/lib.rs`:

```rust
/// Outcome of one poll cycle, handed to the optional scan hook.
#[derive(Debug, Clone, Default)]
pub struct FiledropScan {
    pub seen: u64,
    pub processed: u64,
    pub errors: u64,
    pub duration_ms: u64,
}

pub type ScanHook = std::sync::Arc<dyn Fn(FiledropScan) + Send + Sync>;
```

Add field `scan_hook: Option<ScanHook>` to `FiledropWatcher`, default `None` in `new()`, plus a builder:

```rust
pub fn with_scan_hook(mut self, hook: ScanHook) -> Self {
    self.scan_hook = Some(hook);
    self
}
```

Change `tick()` to return `Result<FiledropScan, ...>` (it already counts seen/processed/errors for its logs — surface those counters; wrap the tick body with `std::time::Instant::now()` for `duration_ms`). In `run()`'s poll arm:

```rust
_ = tokio::time::sleep(self.config.poll_interval) => {
    match self.tick().await {
        Ok(scan) => {
            if let Some(h) = &self.scan_hook { h(scan); }
        }
        Err(e) => {
            warn!(error = %e, "filedrop tick failed");
            if let Some(h) = &self.scan_hook {
                h(FiledropScan { errors: 1, ..Default::default() });
            }
        }
    }
}
```

- [ ] **Step 2: Unit test the hook** — in filedrop's existing test setup (mirror however current tests construct a watcher with an in-memory/local object store), assert the hook fires after a tick with `seen`/`processed` matching dropped files. If existing tests only test `tick()` internals, call `tick()` directly and assert the returned `FiledropScan` counts instead — that's the load-bearing part.

Run: `cargo test -p kyma-ingest-filedrop`
Expected: PASS.

- [ ] **Step 3: Wire registry in kyma-bin**

In `crates/kyma-bin/src/main.rs` (the `KYMA_FILEDROP_ENABLED` block). kyma-bin constructs the Postgres-backed catalog for the admin router — reuse its pool (`rg -n 'PostgresCatalog' crates/kyma-bin/src/main.rs` to find the variable; `.pool().clone()` it):

```rust
let config = FiledropConfig::from_env();
let mut watcher = FiledropWatcher::new(catalog.clone(), store.clone(), write_path.clone(), config.clone());
let (host, node_id, user) = kyma_datasources::watchers::node_identity();
match kyma_datasources::watchers::WatcherRegistry::register(
    pg_catalog.pool().clone(),
    "filedrop",
    &host,
    &node_id,
    &user,
    serde_json::json!({
        "prefixes": config.prefixes,
        "poll_secs": config.poll_interval.as_secs(),
        "delete_after_ingest": config.delete_after_ingest,
    }),
)
.await
{
    Ok(reg) => {
        let rt = tokio::runtime::Handle::current();
        watcher = watcher.with_scan_hook(std::sync::Arc::new(move |scan| {
            let reg = reg.clone();
            rt.spawn(async move {
                reg.heartbeat(Some(&kyma_datasources::watchers::ScanStats {
                    seen: scan.seen,
                    processed: scan.processed,
                    errors: scan.errors,
                    duration_ms: scan.duration_ms,
                    at: chrono::Utc::now().to_rfc3339(),
                    detail: None,
                }))
                .await;
            });
        }));
    }
    Err(e) => tracing::warn!(error = %e, "watcher registry unavailable; filedrop runs unregistered"),
}
```

(`chrono` — confirm it's a kyma-bin dep, `rg chrono crates/kyma-bin/Cargo.toml`; otherwise use whatever time formatting the file already uses.) Registration failure must not prevent the watcher from running — the `match` above runs the watcher either way.

- [ ] **Step 4: Build**: `cargo check --workspace` → green.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: filedrop watcher registers in watcher registry and heartbeats per scan"
```

### Task 10: `GET /v1/data-sources/watchers` (server) + TS client

**Files:**
- Modify: `crates/kyma-datasources/src/admin.rs` (new route + handler)
- Modify: `packages/client/src/connectors.ts` (still old name until Task 13 — add the function there; Task 13 renames the file)
- Test: `crates/kyma-datasources/tests/admin_it.rs` (new test fn)

- [ ] **Step 1: Failing test** in `admin_it.rs`:

```rust
#[tokio::test]
async fn watchers_list_empty_then_rows() {
    let (_pg, s) = state().await;
    let app = app(s.clone());

    let resp = app.clone().oneshot(
        axum::http::Request::builder().uri("/v1/data-sources/watchers")
            .body(axum::body::Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = body_json(resp).await; // reuse the file's existing body helper
    assert_eq!(body["items"].as_array().unwrap().len(), 0);

    kyma_datasources::watchers::WatcherRegistry::register(
        s.catalog.pool().clone(), "filedrop", "h", "n", "u", serde_json::json!({}),
    ).await.unwrap();

    let resp = app.clone().oneshot(
        axum::http::Request::builder().uri("/v1/data-sources/watchers")
            .body(axum::body::Body::empty()).unwrap(),
    ).await.unwrap();
    let body: serde_json::Value = body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["kind"], "filedrop");
}
```

(Adapt to the file's actual response-reading helper; if requests in this file attach a tenant Extension/auth header, copy that too.)

- [ ] **Step 2: Run to verify fail**: `cargo test -p kyma-datasources --test admin_it watchers` → FAIL 404.

- [ ] **Step 3: Implement handler** in `admin.rs`:

```rust
async fn list_watchers(State(s): State<DataSourceAdminState>) -> impl IntoResponse {
    match crate::watchers::WatcherRegistry::list(s.catalog.pool()).await {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
```

Route registration (place BEFORE the `/:id` route so `watchers` isn't captured as an id — axum 0.7 matches static over params, but keep ordering explicit anyway): `.route("/v1/data-sources/watchers", get(list_watchers))`.

- [ ] **Step 4: Run to verify pass**: `cargo test -p kyma-datasources --test admin_it` → PASS.

- [ ] **Step 5: TS client function** — append to `packages/client/src/connectors.ts`:

```typescript
// ── Watchers (file watchers / cc-sync provenance) ─────────────────────────────

export interface DataSourceWatcher {
  id: string;
  kind: "filedrop" | "cc_sync";
  node_host: string;
  node_id: string;
  identity: string;
  config: Record<string, unknown>;
  started_at: string;
  last_heartbeat_at: string;
  last_scan: {
    seen: number;
    processed: number;
    errors: number;
    duration_ms: number;
    at: string;
    detail?: Record<string, unknown>;
  } | null;
  stale: boolean;
}

export async function listDataSourceWatchers(t: KymaTransport): Promise<DataSourceWatcher[]> {
  const body = await handleResponse<{ items: DataSourceWatcher[] }>(
    await t.request("/v1/data-sources/watchers"),
  );
  return body.items ?? [];
}
```

Build: `cd packages/client && npm run build` (or the repo's build command — check `package.json` scripts).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: GET /v1/data-sources/watchers endpoint + client"
```

### Task 11: kyma-local cc-sync watcher status + local endpoint

**Files:**
- Create: `crates/kyma-local/src/watcher_status.rs`
- Modify: `crates/kyma-local/src/lib.rs:527-542` (watcher loop updates status; mount route where the other local `/v1` routers are nested — find with `rg -n 'Router::new\(\)|\.nest\(|\.merge\(' crates/kyma-local/src/lib.rs`)

- [ ] **Step 1: Implement the in-process status store** (`watcher_status.rs`):

```rust
//! Local-mode watcher registry: kyma-local is a single process with SQLite,
//! so cc-sync watcher state lives in memory and is served through the same
//! `/v1/data-sources/watchers` shape as the control plane's Postgres registry.

use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize)]
pub struct LocalWatcher {
    pub id: String,
    pub kind: String, // "cc_sync"
    pub node_host: String,
    pub node_id: String,
    pub identity: String,
    pub config: serde_json::Value,
    pub started_at: String,
    pub last_heartbeat_at: String,
    pub last_scan: Option<serde_json::Value>,
    pub stale: bool, // always false while the process lives; loop updates heartbeat
}

#[derive(Clone, Default)]
pub struct LocalWatcherStatus(Arc<RwLock<Vec<LocalWatcher>>>);

impl LocalWatcherStatus {
    pub fn upsert(&self, w: LocalWatcher) {
        let mut g = self.0.write().unwrap();
        match g.iter_mut().find(|x| x.id == w.id) {
            Some(slot) => *slot = w,
            None => g.push(w),
        }
    }

    pub fn router(&self) -> Router {
        let state = self.clone();
        Router::new().route(
            "/v1/data-sources/watchers",
            get(move |State(s): State<LocalWatcherStatus>| async move {
                let items = s.0.read().unwrap().clone();
                Json(serde_json::json!({ "items": items }))
            }),
        )
        .with_state(state)
    }
}
```

(Adjust the handler to however other kyma-local routes carry state — if local routers take a shared AppState, add the `LocalWatcherStatus` field there instead of a per-router state; copy the established pattern.)

- [ ] **Step 2: Feed it from the cc-sync loop** — in the `KYMA_CC_WATCH` block of `lib.rs`, create `let watcher_status = LocalWatcherStatus::default();` BEFORE router construction so it can be both mounted and moved into the loop. After each `run_cc_phase` call:

```rust
let started = chrono::Utc::now().to_rfc3339();
let (host, node_id, user) = (
    gethostname::gethostname().to_string_lossy().into_owned(),
    std::env::var("KYMA_NODE_ID").unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().into_owned()),
    std::env::var("USER").unwrap_or_else(|_| "unknown".into()),
);
// inside the loop, after run_cc_phase returns Ok(report):
status.upsert(LocalWatcher {
    id: "cc-sync".into(),
    kind: "cc_sync".into(),
    node_host: host.clone(),
    node_id: node_id.clone(),
    identity: user.clone(),
    config: serde_json::json!({ "poll_secs": poll.as_secs(), "root": "~/.claude/projects" }),
    started_at: started.clone(),
    last_heartbeat_at: chrono::Utc::now().to_rfc3339(),
    last_scan: Some(serde_json::to_value(report.summary()).unwrap_or_default()),
    stale: false,
});
```

`run_cc_phase` returns a `CcSyncReport` (aggregating `ProjectSyncReport`s). Add a `summary()` → `serde_json::Value` method on `CcSyncReport` in `cc_sync.rs` packing: total upserted/skipped/user_edited/edges_added/archived, plus `realms: [{realm, upserted, skipped, user_edited, archived, edges_added}]` from the per-project reports (derive or hand-roll; `ProjectSyncReport` isn't Serialize today — either add `Serialize` derives or build the json manually). If `run_cc_phase` currently discards the report, change the loop to capture it (`Ok(report) => { ...upsert... }`).

- [ ] **Step 3: Mount the router** — `.merge(watcher_status.router())` next to the other `/v1` routes in kyma-local's router construction.

- [ ] **Step 4: Test** — unit-test `LocalWatcherStatus::upsert` replaces by id (plain `#[test]` in the module), and `cargo test -p kyma-local` stays green. Manual check (optional, needs a local setup): `KYMA_CC_WATCH=1 kyma serve` then `curl localhost:<port>/v1/data-sources/watchers`.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: cc-sync watcher status + local /v1/data-sources/watchers endpoint"
```

### Task 12: Memory source-summary endpoint

**Files:**
- Modify: `crates/kyma-server/src/agent/memory_retrieve.rs` (or the module where memory list/recall HTTP handlers live — confirm with `rg -n 'memory' crates/kyma-server/src/agent/ -l` and pick the handler file that already serves the web memory UI)
- Modify: kyma-local's matching memory handler if local mode serves memory routes separately (`rg -n 'memory' crates/kyma-local/src/ -l`)
- Modify: `packages/client/src/` memory module (find: `ls packages/client/src | grep -i memo`)

The web memory UI works in BOTH modes today, so there is one memory HTTP surface reachable in both — implement the aggregate exactly where the existing memory-list endpoint lives, following its engine/dialect pattern.

- [ ] **Step 1: Locate the memory list handler + its query layer**

```bash
rg -n 'memory_nodes' crates/ --glob '*.rs' -l
rg -n 'fn.*(list|recall|search).*memor' crates/kyma-memory/src crates/kyma-server/src crates/kyma-local/src -i
```

Identify: (a) the HTTP route serving the memory UI's list, (b) the storage function it calls, (c) whether that storage function is dialect-abstracted (one impl for SQLite + PG) or split.

- [ ] **Step 2: Failing test** — alongside the existing storage-layer tests in kyma-memory (copy their setup; they run against SQLite in-memory or the PG container, whichever the suite uses):

```rust
#[tokio::test]
async fn source_summary_groups_by_provenance_source_and_realm() {
    // setup: same harness as the neighboring list/recall tests
    // insert three memories: two with provenance {"source":"claude-code"} realm "kyma",
    // one with no provenance (counts as "manual"), realm "default"
    // call source_summary()
    // expect: [("claude-code", "kyma", 2), ("manual", "default", 1)]
}
```

Fill the setup verbatim from the neighboring test in that file — the harness varies by storage backend, so copy it rather than inventing one.

- [ ] **Step 3: Implement storage fn** next to the list/recall query. SQL:

Postgres:
```sql
SELECT COALESCE(provenance->>'source', 'manual') AS source,
       realm,
       count(*) AS count
FROM memory_nodes
WHERE status != 'archived'
GROUP BY 1, 2
ORDER BY count DESC
```

SQLite:
```sql
SELECT COALESCE(json_extract(provenance, '$.source'), 'manual') AS source,
       realm,
       count(*) AS count
FROM memory_nodes
WHERE status != 'archived'
GROUP BY 1, 2
ORDER BY count DESC
```

(Match the table/column names actually present — verify `provenance` column name and `status` values from kyma-memory's schema/migrations before writing; adjust to the real enum spellings.)

Return type: `Vec<SourceSummary>` with `{ source: String, realm: String, count: i64 }` (Serialize).

- [ ] **Step 4: HTTP route** — `GET /v1/agent/memory/source-summary` (match the existing memory route prefix exactly — if list lives at e.g. `/v1/agent/memory/list`, mirror it) returning `{ "items": [...] }`. Mount in both server and local exactly where the sibling memory routes are mounted.

- [ ] **Step 5: Run tests**: targeted memory suite (`cargo test -p kyma-memory source_summary` + handler test if the module has HTTP tests) → PASS.

- [ ] **Step 6: TS client** — in the client memory module:

```typescript
export interface MemorySourceSummary {
  source: string;
  realm: string;
  count: number;
}

export async function memorySourceSummary(t: KymaTransport): Promise<MemorySourceSummary[]> {
  const body = await handleResponse<{ items: MemorySourceSummary[] }>(
    await t.request("/v1/agent/memory/source-summary"),
  );
  return body.items ?? [];
}
```

(Reuse that file's existing response helper; export from `packages/client/src/index.ts` if memory isn't `export *`'d already.)

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: memory source-summary aggregate endpoint (both modes) + client"
```

---

## Phase 3 — TypeScript client + web

### Task 13: Client lib rename

**Files:**
- Rename: `packages/client/src/connectors.ts` → `packages/client/src/datasources.ts`
- Modify: `packages/client/src/index.ts`, the client class that exposes `sessionClient().connectors` (find: `rg -n 'connectors' packages/client/src/`), `web/src/sdk/connectors.ts` → `web/src/sdk/datasources.ts`

- [ ] **Step 1: Rename + sweep the client package**

```bash
git mv packages/client/src/connectors.ts packages/client/src/datasources.ts
rg -l 'connector' packages/client/src | xargs perl -pi -e '
  s/\bConnectorSummary\b/DataSourceSummary/g;
  s/\bConnectorDetail\b/DataSourceDetail/g;
  s/\bCreateConnectorBody\b/CreateDataSourceBody/g;
  s/\bConnectorUpdate\b/DataSourceUpdate/g;
  s/\bConnectorStatus\b/DataSourceStatus/g;
  s/\blistConnectors\b/listDataSources/g;
  s/\bgetConnector\b/getDataSource/g;
  s/\bcreateConnector\b/createDataSource/g;
  s/\bpatchConnector\b/patchDataSource/g;
  s/\bdeleteConnector\b/deleteDataSource/g;
  s/\bpauseConnector\b/pauseDataSource/g;
  s/\bresumeConnector\b/resumeDataSource/g;
  s/\btriggerConnector\b/triggerDataSource/g;
  s/\bgetConnectorCatalog\b/getDataSourceCatalog/g;
  s/from "\.\/connectors"/from ".\/datasources"/g;
  s/\bconnectors\b/datasources/g;
'
```

Then fix the API paths the sweep clobbered: in `datasources.ts` all `t.request(...)` URLs must be `/v1/data-sources...` (the sweep turned them into `/v1/datasources` only if they were the bare word — verify every `t.request` line by eye; the server route is `data-sources` with a dash). Update the doc comment header. `CatalogEntry`/`CatalogField`/`CatalogResource`/`GitHubRepo`/`deriveStatus`/`SCHEDULE_MS_*` keep their names.

- [ ] **Step 2: Update the client class namespace** (`sessionClient().connectors.*` → `.datasources.*`) — find the class with `rg -n '\.connectors' packages/client/src web/src` and rename the property + its method bindings.

- [ ] **Step 3: Rename the web shim**

```bash
git mv web/src/sdk/connectors.ts web/src/sdk/datasources.ts
```

Apply the same symbol sweep to `web/src/sdk/datasources.ts` and to all importers (`rg -l "sdk/connectors|listConnectors|ConnectorSummary" web/src | xargs perl -pi -e '...same substitutions plus s\|sdk/connectors\|sdk/datasources\|g'`).

- [ ] **Step 4: Typecheck both packages**

Run: `cd packages/client && npm run build && cd ../../web && npx tsc --noEmit`
Expected: client builds; web typecheck fails ONLY in `features/connectors/` files (renamed exports) — those are Task 14's files; if the error list is all under `features/connectors/`, proceed (Tasks 13+14 can be one commit if preferred — otherwise fix imports in place now and commit).

- [ ] **Step 5: Commit** (with Task 14 if web typecheck demands it)

```bash
git add -A && git commit -m "feat!: client lib renamed — listDataSources/DataSourceSummary, /v1/data-sources paths"
```

### Task 14: Web feature directory rename

**Files:**
- Rename: `web/src/features/connectors/` → `web/src/features/datasources/` (15 files; component files renamed per map below)

- [ ] **Step 1: Move + rename files**

```bash
git mv web/src/features/connectors web/src/features/datasources
cd web/src/features/datasources
git mv AddConnectorWizard.tsx AddDataSourceWizard.tsx
git mv ConnectorCatalog.tsx DataSourceCatalog.tsx
git mv ConnectorConfigForm.tsx DataSourceConfigForm.tsx
git mv ConnectorConnectStep.tsx DataSourceConnectStep.tsx
git mv ConnectorCredentialPicker.tsx DataSourceCredentialPicker.tsx
git mv ConnectorDetail.tsx DataSourceDetail.tsx
git mv ConnectorRow.tsx DataSourceRow.tsx
git mv ConnectorsList.tsx DataSourcesList.tsx
git mv connector-kinds.ts datasource-kinds.ts
git mv useConnectors.ts useDataSources.ts
cd /Users/shakedaskayo/shaked/projects/kyma
```

(`BrandIcon.tsx`, `RepoPicker.tsx`, `StatusBadge.tsx`, `useOAuthConnect.ts`, `vendor-icons.tsx` keep their names.)

- [ ] **Step 2: Sweep symbols + imports across web**

```bash
rg -l -e 'features/connectors' -e 'Connector' web/src | xargs perl -pi -e '
  s|features/connectors|features/datasources|g;
  s/\bAddConnectorWizard\b/AddDataSourceWizard/g;
  s/\bConnectorCatalog\b/DataSourceCatalog/g;
  s/\bConnectorConfigForm\b/DataSourceConfigForm/g;
  s/\bConnectorConnectStep\b/DataSourceConnectStep/g;
  s/\bConnectorCredentialPicker\b/DataSourceCredentialPicker/g;
  s/\bConnectorDetail\b/DataSourceDetail/g;
  s/\bConnectorRow\b/DataSourceRow/g;
  s/\bConnectorsList\b/DataSourcesList/g;
  s/\buseConnectors\b/useDataSources/g;
  s/connector-kinds/datasource-kinds/g;
  s/useConnectors\b/useDataSources/g;
'
```

Then hand-sweep remaining lowercase identifiers and **user-facing strings** in `web/src/features/datasources/`: `rg -n -i 'connector' web/src/features/datasources/` — every hit gets renamed (`connector` → `data source` in UI copy: "Add data source", "No data sources yet", wizard step titles, aria-labels; variable names to `dataSource`/`ds`). Internal type-ids like `"github"` stay.

- [ ] **Step 3: Typecheck**: `cd web && npx tsc --noEmit` — remaining errors should only be the route files (Task 15). If zero route errors, run `npm run build` too.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor(web): features/connectors -> features/datasources, components renamed"
```

### Task 15: Routes, tab layout, sidebar, capability key

**Files:**
- Create: `web/src/features/datasources/DataSourcesTabs.tsx`
- Create: `web/src/routes/_app.data-sources.tsx`, `_app.data-sources.index.tsx`, `_app.data-sources.$id.tsx` (content moved from the old route files)
- Delete: `web/src/routes/_app.connectors.tsx`, `_app.connectors.index.tsx`, `_app.connectors.$id.tsx`
- Modify: `web/src/app/Sidebar.tsx:50`, `web/src/sdk/capabilities.ts` (+ its Capabilities type / FULL_CAPABILITIES), `web/src/features/capabilities/ControlPlaneGate.tsx`

- [ ] **Step 1: Tab bar component** `web/src/features/datasources/DataSourcesTabs.tsx`:

```tsx
import { Link } from "@tanstack/react-router";

const TABS = [
  { to: "/data-sources", label: "Sources", exact: true },
  { to: "/data-sources/watchers", label: "File Watchers" },
  { to: "/data-sources/sync", label: "Memory Sync" },
  { to: "/data-sources/memories", label: "Memories" },
] as const;

export function DataSourcesTabs() {
  return (
    <nav className="flex gap-1 border-b bg-background px-6">
      {TABS.map((t) => (
        <Link
          key={t.to}
          to={t.to}
          activeOptions={{ exact: "exact" in t && t.exact }}
          className="border-b-2 border-transparent px-3 py-2 text-sm text-muted-foreground hover:text-foreground"
          activeProps={{ className: "border-primary text-foreground font-medium" }}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}
```

(Match the exact active-link idiom used in the memory module's header — `rg -n 'activeProps|activeOptions' web/src/features/memory/MemoryHeader.tsx` — and copy that pattern if it differs.)

- [ ] **Step 2: Layout route** `web/src/routes/_app.data-sources.tsx`:

```tsx
import { createFileRoute, Outlet } from "@tanstack/react-router";
import { DataSourcesTabs } from "@/features/datasources/DataSourcesTabs";

export const Route = createFileRoute("/_app/data-sources")({
  component: DataSourcesLayout,
});

function DataSourcesLayout() {
  return (
    <div className="flex h-full flex-col bg-muted/20">
      <div className="border-b bg-background px-6 pt-4">
        <h1 className="text-lg font-semibold">Data Sources</h1>
        <p className="pb-3 text-sm text-muted-foreground">
          Everything feeding the context graph — connectors, file watchers, and memory sync.
        </p>
      </div>
      <DataSourcesTabs />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Outlet />
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Move list + detail routes** — create `_app.data-sources.index.tsx` from the old `_app.connectors.index.tsx` body: route id `"/_app/data-sources/"`, remove the per-page `<h1>` header block (the layout owns it now — keep the "Add data source" button, move it into the content area top-right), `useCapability("data_sources")`, `ControlPlaneGate feature="data_sources" title="Data Sources"`, navigate target `to: "/data-sources/$id"`. Create `_app.data-sources.$id.tsx` from `_app.connectors.$id.tsx` with route id `"/_app/data-sources/$id"`. Then:

```bash
git rm web/src/routes/_app.connectors.tsx web/src/routes/_app.connectors.index.tsx web/src/routes/_app.connectors.$id.tsx
rg -n '"/connectors' web/src  # update every remaining link target to /data-sources
```

- [ ] **Step 4: Sidebar + capability key**

`web/src/app/Sidebar.tsx:50` → `{ to: "/data-sources", label: "Data Sources", icon: Database, requires: "data_sources" },` (import `Database` from lucide-react, drop `Plug` if now unused).

`web/src/sdk/capabilities.ts`: rename `connectors` → `data_sources` in the Capabilities interface and `FULL_CAPABILITIES`. `ControlPlaneGate.tsx`: update the `feature` union type and any copy ("Connectors run on the control plane" → "Data sources run on the control plane").

- [ ] **Step 5: Typecheck + route-gen + build**

Run: `cd web && npx tsc --noEmit && npm run build`
Expected: TanStack route tree regenerates (`routeTree.gen.ts` — regenerated by the dev/build process) with `/data-sources` routes; build green.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(web): /data-sources tabbed module — Sources tab + layout, sidebar, capability key"
```

### Task 16: Watchers / Sync / Memories tabs

**Files:**
- Create: `web/src/features/datasources/WatchersTab.tsx`, `SyncTab.tsx`, `MemoriesSummaryTab.tsx`
- Create: `web/src/routes/_app.data-sources.watchers.tsx`, `_app.data-sources.sync.tsx`, `_app.data-sources.memories.tsx`
- Modify: `web/src/sdk/datasources.ts` (re-export `listDataSourceWatchers`, `DataSourceWatcher`, `memorySourceSummary`, `MemorySourceSummary` via the shim pattern)

- [ ] **Step 1: Shim exports** — mirror the existing shim style in `web/src/sdk/datasources.ts`:

```typescript
export type { DataSourceWatcher, MemorySourceSummary } from "@kyma-ai/client";

export function listDataSourceWatchers(_args: Base) {
  return sessionClient().datasources.listDataSourceWatchers();
}
export function memorySourceSummary(_args: Base) {
  return sessionClient().datasources.memorySourceSummary();
}
```

(Or whatever namespace the client class exposes for memory — match Task 12/13's wiring.)

- [ ] **Step 2: WatchersTab** `web/src/features/datasources/WatchersTab.tsx`:

```tsx
import { useQuery } from "@tanstack/react-query";
import { FolderSearch, Brain } from "lucide-react";
import { sessionClient } from "@/sdk/client";
import type { DataSourceWatcher } from "@kyma-ai/client";

const KIND_META: Record<string, { label: string; icon: typeof FolderSearch }> = {
  filedrop: { label: "File drop", icon: FolderSearch },
  cc_sync: { label: "Claude Code sync", icon: Brain },
};

export function WatchersTab() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["data-source-watchers"],
    queryFn: () => sessionClient().datasources.listDataSourceWatchers(),
    refetchInterval: 15_000,
  });

  if (isLoading) return <p className="p-6 text-sm text-muted-foreground">Loading watchers…</p>;
  if (error) return <p className="p-6 text-sm text-destructive">Watchers unavailable: {String(error)}</p>;
  if (!data?.length)
    return (
      <div className="p-6 text-sm text-muted-foreground">
        No file watchers are running. Enable the object-store drop watcher with{" "}
        <code className="rounded bg-muted px-1">KYMA_FILEDROP_ENABLED=1</code> or Claude Code memory
        sync with <code className="rounded bg-muted px-1">KYMA_CC_WATCH=1</code>.
      </div>
    );

  return (
    <div className="space-y-3 p-6">
      {data.map((w) => (
        <WatcherCard key={w.id} w={w} />
      ))}
    </div>
  );
}

function WatcherCard({ w }: { w: DataSourceWatcher }) {
  const meta = KIND_META[w.kind] ?? { label: w.kind, icon: FolderSearch };
  const Icon = meta.icon;
  const scan = w.last_scan;
  return (
    <div className="rounded-lg border bg-background p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Icon className="h-4 w-4 text-muted-foreground" />
          <span className="font-medium">{meta.label}</span>
          {w.stale ? (
            <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-600">stale</span>
          ) : (
            <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs text-emerald-600">live</span>
          )}
        </div>
        <span className="text-xs text-muted-foreground">
          heartbeat {new Date(w.last_heartbeat_at).toLocaleString()}
        </span>
      </div>
      <dl className="mt-3 grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-4">
        <div><dt className="text-xs text-muted-foreground">Node</dt><dd>{w.node_host}</dd></div>
        <div><dt className="text-xs text-muted-foreground">Identity</dt><dd>{w.identity}</dd></div>
        <div>
          <dt className="text-xs text-muted-foreground">Watching</dt>
          <dd className="truncate">{String((w.config.prefixes as string[] | undefined)?.join(", ") ?? w.config.root ?? "—")}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Last scan</dt>
          <dd>{scan ? `${scan.processed}/${scan.seen} files, ${scan.errors} errors` : "—"}</dd>
        </div>
      </dl>
    </div>
  );
}
```

- [ ] **Step 3: SyncTab** `web/src/features/datasources/SyncTab.tsx` — same query, filtered to `kind === "cc_sync"`, rendering the per-realm rollup out of `last_scan.detail.realms`:

```tsx
import { useQuery } from "@tanstack/react-query";
import { sessionClient } from "@/sdk/client";

interface RealmReport {
  realm: string;
  upserted: number;
  skipped: number;
  user_edited: number;
  edges_added: number;
  archived: number;
}

export function SyncTab() {
  const { data, isLoading } = useQuery({
    queryKey: ["data-source-watchers"],
    queryFn: () => sessionClient().datasources.listDataSourceWatchers(),
    refetchInterval: 15_000,
  });

  if (isLoading) return <p className="p-6 text-sm text-muted-foreground">Loading sync status…</p>;
  const cc = data?.find((w) => w.kind === "cc_sync");
  if (!cc)
    return (
      <div className="p-6 text-sm text-muted-foreground">
        Claude Code memory sync is not running. Start the local server with{" "}
        <code className="rounded bg-muted px-1">KYMA_CC_WATCH=1</code> to keep file memories synced
        into the graph.
      </div>
    );

  const realms = ((cc.last_scan?.detail as Record<string, unknown> | undefined)?.realms ??
    []) as RealmReport[];

  return (
    <div className="p-6">
      <div className="mb-4 text-sm text-muted-foreground">
        Watching <code className="rounded bg-muted px-1">~/.claude/projects</code> on{" "}
        <span className="font-medium text-foreground">{cc.node_host}</span> as{" "}
        <span className="font-medium text-foreground">{cc.identity}</span> — last sync{" "}
        {cc.last_scan ? new Date(cc.last_scan.at).toLocaleString() : "never"}.
      </div>
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b text-left text-xs text-muted-foreground">
            <th className="py-2 font-medium">Realm</th>
            <th className="py-2 font-medium">Upserted</th>
            <th className="py-2 font-medium">Skipped</th>
            <th className="py-2 font-medium">User edited</th>
            <th className="py-2 font-medium">Edges</th>
            <th className="py-2 font-medium">Archived</th>
          </tr>
        </thead>
        <tbody>
          {realms.map((r) => (
            <tr key={r.realm} className="border-b last:border-0">
              <td className="py-2 font-medium">{r.realm}</td>
              <td className="py-2">{r.upserted}</td>
              <td className="py-2">{r.skipped}</td>
              <td className="py-2">{r.user_edited}</td>
              <td className="py-2">{r.edges_added}</td>
              <td className="py-2">{r.archived}</td>
            </tr>
          ))}
          {realms.length === 0 && (
            <tr><td colSpan={6} className="py-4 text-muted-foreground">No realms synced yet.</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
```

- [ ] **Step 4: MemoriesSummaryTab** `web/src/features/datasources/MemoriesSummaryTab.tsx`:

```tsx
import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";
import { sessionClient } from "@/sdk/client";

export function MemoriesSummaryTab() {
  const { data, isLoading } = useQuery({
    queryKey: ["memory-source-summary"],
    queryFn: () => sessionClient().datasources.memorySourceSummary(),
  });

  if (isLoading) return <p className="p-6 text-sm text-muted-foreground">Loading…</p>;

  const bySource = new Map<string, { total: number; realms: { realm: string; count: number }[] }>();
  for (const row of data ?? []) {
    const e = bySource.get(row.source) ?? { total: 0, realms: [] };
    e.total += row.count;
    e.realms.push({ realm: row.realm, count: row.count });
    bySource.set(row.source, e);
  }

  return (
    <div className="p-6">
      <div className="mb-4 flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          Where memories come from. The full store lives in{" "}
          <Link to="/memory" className="inline-flex items-center gap-0.5 text-foreground underline">
            Memory <ArrowUpRight className="h-3 w-3" />
          </Link>
          .
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {[...bySource.entries()].map(([source, e]) => (
          <div key={source} className="rounded-lg border bg-background p-4">
            <div className="flex items-baseline justify-between">
              <span className="font-medium">{source}</span>
              <span className="text-2xl font-semibold tabular-nums">{e.total}</span>
            </div>
            <ul className="mt-2 space-y-0.5 text-sm text-muted-foreground">
              {e.realms.map((r) => (
                <li key={r.realm} className="flex justify-between">
                  <span className="truncate">{r.realm}</span>
                  <span className="tabular-nums">{r.count}</span>
                </li>
              ))}
            </ul>
          </div>
        ))}
        {bySource.size === 0 && (
          <p className="text-sm text-muted-foreground">No memories yet.</p>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Route files** — three thin routes:

```tsx
// web/src/routes/_app.data-sources.watchers.tsx
import { createFileRoute } from "@tanstack/react-router";
import { WatchersTab } from "@/features/datasources/WatchersTab";
export const Route = createFileRoute("/_app/data-sources/watchers")({ component: WatchersTab });
```

```tsx
// web/src/routes/_app.data-sources.sync.tsx
import { createFileRoute } from "@tanstack/react-router";
import { SyncTab } from "@/features/datasources/SyncTab";
export const Route = createFileRoute("/_app/data-sources/sync")({ component: SyncTab });
```

```tsx
// web/src/routes/_app.data-sources.memories.tsx
import { createFileRoute } from "@tanstack/react-router";
import { MemoriesSummaryTab } from "@/features/datasources/MemoriesSummaryTab";
export const Route = createFileRoute("/_app/data-sources/memories")({ component: MemoriesSummaryTab });
```

Static segments outrank `$id` in TanStack Router, so `/data-sources/watchers` never falls into the detail route — verify by visiting both after route-gen.

- [ ] **Step 6: Build + visual check**

Run: `cd web && npx tsc --noEmit && npm run build`
Then `npm run dev` against a local server, open `/data-sources` — confirm: tabs render, Sources lists existing sources, Watchers/Sync show empty-state copy (or live rows with `KYMA_CC_WATCH=1`), Memories shows counts. Screenshot via browser-harness if running headed.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat(web): File Watchers, Memory Sync, and Memories tabs on /data-sources"
```

---

## Phase 4 — Docs + final sweep

### Task 17: Docs site rename

**Files:**
- Rename: `docs/site/connectors/` → `docs/site/data-sources/`
- Modify: `docs/site/.vitepress/config.ts:106-128`, every doc page linking `/connectors/`

- [ ] **Step 1: Move + relink**

```bash
git mv docs/site/connectors docs/site/data-sources
rg -l '/connectors/' docs/ | xargs perl -pi -e 's|/connectors/|/data-sources/|g'
```

In `.vitepress/config.ts`: sidebar key `'/connectors/'` → `'/data-sources/'`, `text: 'Connectors'` → `'Data Sources'`, all item links (the sweep above caught the links; fix the `text:` labels by hand where they say "OAuth connectors" → "OAuth sources").

- [ ] **Step 2: Content sweep** — rename user-facing prose: `rg -n -i 'connector' docs/site/ | wc -l`, then `rg -l -i connector docs/site | xargs perl -pi -e 's/\bConnectors\b/Data Sources/g; s/\bconnectors\b/data sources/g; s/\bConnector\b/Data source/g; s/\bconnector\b/data source/g'` and review the diff for casualties (code blocks showing CLI/API examples must show the NEW commands/paths — they were already correct from earlier task sweeps only if docs duplicated them; fix `kyma connector` → `kyma datasource`, `/v1/connectors` → `/v1/data-sources` in fenced blocks). Rename framework.md's trait references to `DataSource`.

- [ ] **Step 3: Build the docs site** — find the build script (`cat docs/site/package.json` or root package.json) and run it; VitePress fails on dead links, which catches missed renames.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "docs: connectors -> data sources across the site"
```

### Task 18: Full verification sweep

- [ ] **Step 1: The grep gate** — remaining `connector` mentions must only be: this plan/spec, CHANGELOG/history, and the migration files ≤026 (history is immutable):

```bash
rg -n -i 'connector' --glob '!target' --glob '!node_modules' --glob '!*.lock' \
  --glob '!docs/superpowers/**' --glob '!crates/kyma-catalog/migrations/0[0-2][0-6]*' \
  --glob '!CHANGELOG*' . | grep -v '027_data_sources_rename'
```

Expected: empty (027 references old names by necessity). Fix any stragglers.

- [ ] **Step 2: Full test suite**

Run: `cargo test --workspace` (Docker up) and `cd web && npm run build && npx tsc --noEmit` and the client package build.
Expected: all green.

- [ ] **Step 3: CLI smoke against a local server** (if a dev server is handy): `kyma datasource list`, `kyma ingest status` — exercise one real request each.

- [ ] **Step 4: Update repo docs** — README and any quickstart mentioning `kyma connector` (`rg -n 'kyma connector' README.md docs/` — should already be clean from the gate).

- [ ] **Step 5: Final commit + summary**

```bash
git add -A && git commit -m "chore: final data-sources rename sweep"
git log --oneline feat/federated-sources..HEAD  # review the task commits
```

---

## Self-review notes (already applied)

- Spec §1 naming map → Tasks 1–7, 13–15, 17. Spec §2 migration → Task 3. Spec §3 web module → Tasks 15–16. Spec §4 watcher registry → Tasks 8–11. Memories summary (spec §3 item 4) → Tasks 12, 16. Spec §5 error handling → registry best-effort (Task 8 heartbeat, Task 9 unregistered fallback). Spec §6 testing → per-task tests + Task 18 gate. Local-mode cc-sync deviation documented in the header.
- Known adaptive points (flagged inline rather than guessed): exact memory handler file/route prefix (Task 12 Step 1 discovers it), kyma-local router state pattern (Task 11), background_tasks seed columns (Task 3), client class namespace shape (Task 13). Each has a discovery command and a copy-the-neighbor instruction.
