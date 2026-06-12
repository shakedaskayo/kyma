# Unified Sources Page + Obsidian Vault Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One unified Data Sources page (no tabs) with Claude Code as a pre-installed branded source, plus a complete Obsidian-vault data source synced continuously by a filesystem watcher into memory nodes + graph edges.

**Architecture:** Obsidian is a real `data_sources` row with a new `drive_model = "watcher"` (excluded from the periodic scheduler). A local watcher manager reconciles enabled rows into per-vault notify-driven sync loops; the sync engine (`vault_sync.rs`, modeled on `cc_sync.rs`) upserts notes as memory nodes with wikilink `RELATES_TO` edges and archive-on-delete. The UI folds the four tabs into one list and reads watcher liveness from the existing `/v1/data-sources/watchers` endpoint.

**Tech Stack:** Rust (axum, sqlx/SQLite, notify 6, tokio), React + TanStack Router/Query, vitest, `@icons-pack/react-simple-icons`.

**Spec:** `docs/superpowers/specs/2026-06-12-unified-sources-obsidian-design.md`

Branch: `feat/unified-sources-obsidian` (off main `d549d226`).

---

### Task 1: Obsidian wikilink normalization (kyma-ccmem)

**Files:**
- Modify: `crates/kyma-ccmem/src/wikilink.rs`

- [ ] **Step 1: Write failing tests** — append to the module (or its existing `#[cfg(test)]` block; create one if absent):

```rust
#[cfg(test)]
mod normalized_tests {
    use super::*;

    #[test]
    fn normalizes_obsidian_link_flavors() {
        let body = "See [[Plain]], [[Target|an alias]], [[Note#Heading]], [[Other^block1]], and ![[Embedded Note]].";
        assert_eq!(
            extract_normalized(body),
            vec!["Plain", "Target", "Note", "Other", "Embedded Note"]
        );
    }

    #[test]
    fn dedupes_after_normalization_and_drops_empty() {
        // `[[A|x]]` and `[[A#y]]` normalize to the same target; `[[|alias]]` is empty.
        let body = "[[A|x]] [[A#y]] [[|alias]] [[ ]]";
        assert_eq!(extract_normalized(body), vec!["A"]);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p kyma-ccmem normalized` → FAIL (`extract_normalized` not found).

- [ ] **Step 3: Implement** — add to `wikilink.rs`:

```rust
/// Extract Obsidian-flavored wikilink targets, normalized to the bare note
/// name: embeds (`![[…]]`) are caught by the plain `[[` scan; alias
/// (`|alias`), heading (`#heading`), and block (`^block`) suffixes are
/// stripped. First-occurrence order, de-duplicated after normalization.
pub fn extract_normalized(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in extract(body) {
        let t = raw
            .split(['|', '#', '^'])
            .next()
            .unwrap_or("")
            .trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}
```

- [ ] **Step 4: Run** — `cargo test -p kyma-ccmem` → PASS.
- [ ] **Step 5: Commit** — `feat(ccmem): obsidian-flavored wikilink normalization`

---

### Task 2: `drive_model` on `CatalogEntry` + create honors it + `installed` Claude Code entry

**Files:**
- Modify: `crates/kyma-datasources/src/catalog.rs`
- Modify: `crates/kyma-datasources/src/admin.rs` (create handler line ~99; catalog handler merge)
- Modify: every `CatalogEntry { … }` literal in data source impls (compiler-driven: add `drive_model: "periodic".into(),`)
- Test: `crates/kyma-datasources/tests/admin_it.rs` (existing patterns)

- [ ] **Step 1: Add the field** in `catalog.rs`:

```rust
/// `"periodic"` (scheduler-ticked) | `"watcher"` (synced by a node-local
/// file watcher; the scheduler never ticks it).
pub drive_model: String,
```

Set `drive_model: "periodic".into()` in `CatalogEntry::minimal()` and `soon()`. Build the workspace and fix every struct literal the compiler flags the same way (all existing sources are periodic).

- [ ] **Step 2: `installed()` list** in `catalog.rs` below `coming_soon()`:

```rust
/// Sources that ship with the engine itself (no create flow). Presented in
/// the catalog with `status: "installed"`; the UI renders them as
/// pre-installed rather than creatable. cc-sync is the first: agent-memory
/// sync kinds (Claude Code today, other coding agents as they land) run as
/// node-local watchers, not scheduled data sources.
pub fn installed() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        type_id: "claude_code".into(),
        label: "Claude Code".into(),
        category: "knowledge".into(),
        description: "Claude Code's file-based memory, synced continuously into recallable, graph-linked memory nodes. Pre-installed — runs as a local watcher.".into(),
        brand: "claude".into(),
        auth_mode: "none".into(),
        status: "installed".into(),
        drive_model: "watcher".into(),
        default_schedule_ms: 30_000,
        fields: Vec::new(),
        resource: None,
        default_target_table: None,
        config_defaults: None,
        graph_name: None,
        accepted_credential_kinds: Vec::new(),
    }]
}
```

- [ ] **Step 3: Merge + honor in admin.rs** — in `catalog()` handler, extend the merge: after collecting registered entries, append `crate::catalog::installed()` entries whose `type_id` isn't taken, then `coming_soon()` as today. Sort: `installed`/`available` before `coming_soon` (treat both non-coming_soon ranks equal), then category/label. In `create()`, replace the hardcoded `"periodic"` argument with `c.catalog().drive_model` (bind it to a local before the call).

- [ ] **Step 4: Tests** — in `admin_it.rs` add assertions to the existing catalog test (or a new `#[tokio::test]` following its setup pattern): response items include `claude_code` with `status == "installed"`, every item carries a non-empty `drive_model`, and creating a source still works (existing create test passes drive_model implicitly).

- [ ] **Step 5: Run** — `cargo test -p kyma-datasources` (testcontainers; requires Docker). Expected: PASS.
- [ ] **Step 6: Commit** — `feat(datasources): drive_model on catalog entries + installed claude_code entry`

---

### Task 3: `ObsidianDataSource`

**Files:**
- Create: `crates/kyma-datasources/src/obsidian.rs`
- Modify: `crates/kyma-datasources/src/lib.rs` (add `pub mod obsidian;`)
- Modify: `crates/kyma-local/src/lib.rs` (~line 661: register alongside the others)

- [ ] **Step 1: Test first** (inline `#[cfg(test)]` in `obsidian.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_is_watcher_driven() {
        let c = ObsidianDataSource.catalog();
        assert_eq!(c.type_id, "obsidian");
        assert_eq!(c.drive_model, "watcher");
        assert_eq!(c.status, "available");
    }

    #[test]
    fn validate_requires_vault_path() {
        assert!(ObsidianDataSource.validate_config(&json!({})).is_err());
        assert!(ObsidianDataSource.validate_config(&json!({"vault_path": "  "})).is_err());
        assert!(ObsidianDataSource.validate_config(&json!({"vault_path": "~/Vault"})).is_ok());
    }
}
```

- [ ] **Step 2: Implement**:

```rust
//! Obsidian vault data source — catalog/validation only. Watcher-driven
//! (`drive_model = "watcher"`): the local engine's vault watcher performs the
//! actual sync (`kyma-local/src/vault_sync.rs`); the periodic scheduler never
//! ticks these rows, and `run_once` refuses defensively.

use async_trait::async_trait;
use serde_json::Value;

use crate::catalog::{CatalogEntry, CatalogField};
use crate::types::{ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun};

pub struct ObsidianDataSource;

#[async_trait]
impl DataSource for ObsidianDataSource {
    fn type_id(&self) -> &'static str {
        "obsidian"
    }

    fn catalog(&self) -> CatalogEntry {
        let mut name = CatalogField::text("vault_name", "Vault name", "my-vault");
        name.required = false;
        name.help = Some("Realm the notes land in — defaults to the vault folder name.".into());
        CatalogEntry {
            type_id: "obsidian".into(),
            label: "Obsidian".into(),
            category: "knowledge".into(),
            description: "Notes from a local Obsidian vault, synced continuously by a file watcher — wikilinks become graph edges, deleted notes archive.".into(),
            brand: "obsidian".into(),
            auth_mode: "none".into(),
            status: "available".into(),
            drive_model: "watcher".into(),
            default_schedule_ms: 60_000, // full-scan fallback interval
            fields: vec![
                CatalogField::text("vault_path", "Vault path", "~/Documents/MyVault"),
                name,
            ],
            resource: None,
            default_target_table: None,
            config_defaults: None,
            graph_name: None,
            accepted_credential_kinds: Vec::new(),
        }
    }

    fn validate_config(&self, cfg: &Value) -> Result<(), ConfigError> {
        let path = cfg
            .get("vault_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if path.is_empty() {
            return Err(ConfigError("vault_path is required".into()));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        _ctx: &DataSourceCtx,
        _cfg: &Value,
        _cursor: Option<&Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        Err(DataSourceError::Permanent(
            "obsidian sources are synced by the local vault watcher, not the scheduler".into(),
        ))
    }
}
```

(Adjust the `types` import path/`ConfigError` constructor to the actual definitions in `types.rs` — `ConfigError(pub String)` per `admin.rs` usage `e.0`.)

- [ ] **Step 3: Register** — in `kyma-local/src/lib.rs` next to msfabric: `conn_reg.register(Arc::new(kyma_datasources::obsidian::ObsidianDataSource));`
- [ ] **Step 4: Run** — `cargo test -p kyma-datasources obsidian` → PASS; `cargo build -p kyma-local` → OK.
- [ ] **Step 5: Commit** — `feat(datasources): obsidian vault source (catalog + validation, watcher-driven)`

---

### Task 4: Vault sync engine

**Files:**
- Create: `crates/kyma-local/src/vault_sync.rs`
- Create: `crates/kyma-local/src/vault_sync_unit_tests.rs` (wired like `cc_sync_unit_tests.rs` — check how it's included from lib.rs and mirror)
- Modify: `crates/kyma-local/src/lib.rs` (`mod vault_sync;` + test module include)
- Modify: `crates/kyma-local/src/cc_sync.rs` (make `node_id_by_topic_key`, `archive_node` `pub(crate)` for reuse)

**Engine contract** (mirror `cc_sync.rs` structure and invariants):

```rust
//! Sync a local Obsidian vault into the memory store. Files are the source
//! of truth; notes upsert by topic key, wikilinks become RELATES_TO edges,
//! deleted notes archive. Modeled on cc_sync but vault-flavored:
//! frontmatter is OPTIONAL, the walk is recursive, and link syntax is
//! Obsidian's ([[Target|alias]], [[Target#h]], ![[embed]]).

pub(crate) struct VaultSyncOptions {
    /// data_sources row id — namespaces topic keys + sync_state.
    pub source_id: String,
    /// Expanded absolute vault root.
    pub vault_path: std::path::PathBuf,
    /// Realm notes land in (vault_name config or folder basename).
    pub realm: String,
}

#[derive(Debug, Default)]
pub(crate) struct VaultSyncReport {
    pub seen: usize,      // .md files found
    pub upserted: usize,  // created or new version
    pub skipped: usize,   // unchanged hash
    pub edges_added: usize,
    pub archived: usize,
    pub errors: usize,    // unreadable files (logged, non-fatal)
}

impl VaultSyncReport {
    /// Watcher `last_scan` shape, same contract as CcSyncReport.
    pub(crate) fn last_scan_value(&self, duration_ms: u64, at: chrono::DateTime<chrono::Utc>, realm: &str) -> serde_json::Value {
        serde_json::json!({
            "seen": self.seen,
            "processed": self.upserted,
            "errors": self.errors,
            "duration_ms": duration_ms,
            "at": at.to_rfc3339(),
            "detail": { "realm": realm, "edges_added": self.edges_added, "archived": self.archived },
        })
    }
}

pub(crate) async fn run_once(engine: &Engine, writer: &MemoryWriter, opts: &VaultSyncOptions) -> Result<VaultSyncReport>
```

Implementation rules:
- **Walk**: iterative directory stack from `vault_path`; skip any directory whose name starts with `.` (covers `.obsidian`, `.trash`); collect files with extension `md`. Rel path = path relative to root with `/` separators. Sort for determinism. Unreadable file → `errors += 1`, continue.
- **Frontmatter (optional)**: if the file starts with `---\n`, find the next line that is exactly `---`; YAML-parse the block (reuse what `kyma_ccmem::frontmatter` uses; if its parser demands CC-specific shape, parse leniently here with `serde_yaml::Value`). Extract `tags` (string → split on `,`/whitespace; sequence → strings) and `title`/`name` if present. Body = content after the block (whole file when no frontmatter or parse fails).
- **Note name** = file stem. Title = frontmatter title, else note name.
- **Hash/skip**: `kyma_ccmem::hash::content_hash(&note_name, None, &body)`; sync_state key `obsidian:hash:<source_id>:<rel_path>` — equal → `skipped`, still push a `ScanEntry`-like record (name, topic_key, wikilinks, changed=false) for the edge pass + manifest.
- **Upsert**: `topic_key = format!("obsidian:{}/{}", source_id, rel_path)`; `CreateMemory` with realm, `MemoryType::Fact`, `importance 0.6`, tags `["obsidian"] + fm_tags`, provenance `{"source":"obsidian","source_id":…,"vault_path":…,"rel_path":…,"content_hash":…,"ingested_at":…}`. Existing node by topic key → `writer.save_as(uuid, &cm)`; else `writer.save(&cm)`. Reuse `cc_sync::node_id_by_topic_key`.
- **Edges**: `wikilink::extract_normalized(&body)`; resolution map keyed by **lowercase** note name (Obsidian is case-insensitive); emit pairs only when either endpoint changed (cc_sync's exact algorithm) with props `{"via": "obsidian-wikilink"}`.
- **Deletions**: manifest (`obsidian:manifest:<source_id>`, JSON array of `{rel_path, topic_key}`) diffed old→new; gone → `cc_sync::archive_node` + reset hash key (so reappearing files re-ingest); count archived.

- [ ] **Step 1: Write the unit tests first** (`vault_sync_unit_tests.rs`, reusing the `TestEmbed` stub + `engine_at` helper pattern from `cc_sync_unit_tests.rs` — extract shared helpers into a `#[cfg(test)] mod test_support` in lib.rs if simpler, otherwise duplicate locally):
  1. `ingests_plain_and_frontmatter_notes` — vault with `Alpha.md` (plain text) + `sub/Beta.md` (frontmatter w/ `tags: [a,b]`); run; assert 2 memory_nodes with realm, topic keys `obsidian:<id>/Alpha.md` and `obsidian:<id>/sub/Beta.md`, tags contain `obsidian` (+ `a` for Beta), importance 0.6.
  2. `rescan_skips_unchanged` — second run: report.skipped == 2, upserted == 0; node count unchanged.
  3. `edit_creates_new_version_same_node` — modify Alpha.md; rerun; same node id (by topic key), latest body updated.
  4. `wikilinks_become_edges` — `Alpha.md` body `links to [[beta|B]] and ![[Gamma]]`; with `Beta.md`, `Gamma.md` present; assert 2 RELATES_TO edges (case-insensitive resolution), props via obsidian-wikilink. Self/unresolved links dropped.
  5. `deleted_note_archives_and_reappearing_unarchives` — delete Beta.md, rerun → archived 1, node status archived; recreate, rerun → upserted, status live again.
  6. `dot_dirs_excluded` — note under `.obsidian/templates/T.md` and `.trash/Old.md` never ingest.
- [ ] **Step 2: Run to verify failure** — `cargo test -p kyma-local vault_sync` → compile FAIL.
- [ ] **Step 3: Implement `vault_sync.rs`** per the contract above.
- [ ] **Step 4: Run** — `cargo test -p kyma-local vault_sync` → PASS (6 tests).
- [ ] **Step 5: Commit** — `feat(local): obsidian vault sync engine — notes→memory nodes, wikilink edges, archive-on-delete`

---

### Task 5: Watcher manager (reconciler + notify loops)

**Files:**
- Create: `crates/kyma-local/src/source_watchers.rs`
- Modify: `crates/kyma-local/src/lib.rs` (`mod source_watchers;` + spawn in `serve()` after the cc-sync block)
- Modify: `crates/kyma-local/Cargo.toml` (`notify = "6"`)

**Shape:**

```rust
//! Reconciles watcher-driven data source rows (drive_model = 'watcher')
//! into running per-vault sync loops. Poll-reconcile every 15s: new/changed
//! enabled rows spawn a loop (notify fs events, 2s debounce, plus a
//! schedule_ms full-scan fallback); disabled/deleted rows stop theirs.

/// What a watcher-driven row needs to run. `fingerprint` (config_json +
/// schedule_ms + name) detects config edits → restart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DesiredSource {
    pub id: String,
    pub fingerprint: String,
    pub vault_path: std::path::PathBuf, // expanded ~
    pub realm: String,
    pub scan_interval: std::time::Duration, // max(schedule_ms, 30s)
}

#[derive(Debug, PartialEq)]
pub(crate) enum PlanAction {
    Start(DesiredSource),
    Stop(String),
}

/// Pure diff: running (id → fingerprint) vs desired rows.
/// Changed fingerprint → Stop + Start.
pub(crate) fn reconcile_plan(
    running: &std::collections::HashMap<String, String>,
    desired: &[DesiredSource],
) -> Vec<PlanAction>
```

Manager (`pub(crate) async fn run_manager(engine: Engine, status: LocalWatcherStatus, control: Arc<SqliteDataSourceControl>, pool: SqlitePool)`):
- loop every 15s: `SELECT id, tenant_id, name, config_json, schedule_ms FROM data_sources WHERE drive_model = 'watcher' AND type = 'obsidian' AND enabled = 1` → build `DesiredSource` (expand `~` via `$HOME`; realm = config `vault_name` non-empty else folder basename; skip + warn on unparseable config). Apply `reconcile_plan`: Stop → abort `JoinHandle`, `status.remove(&format!("obsidian:{id}"))`; Start → spawn `run_source_loop`.
- `run_source_loop(engine, status, control, src)`:
  - register `notify::recommended_watcher` (RecursiveMode) on `vault_path`, forwarding only events whose paths have `.md` extension and no dot-dir component to a `tokio::sync::mpsc` channel (use `std::sync::mpsc`→tokio bridge or `futures` channel per notify docs; events sent from notify's thread via `blocking_send`/`try_send`).
  - loop: `tokio::select!` on `rx.recv()` (then drain + `sleep(2s)` debounce) and `interval(scan_interval)` tick → run a pass.
  - pass: build writer (`kyma_memory::shared_embedding()` + `MemoryWriter::new`, like `run_cc_phase`); `vault_sync::run_once`; on Ok: `control.mark_run_success(tenant, id, upserted as i64)`; on Err: `control.mark_run_failure(tenant, id, &msg)`; either way upsert `LocalWatcher { id: format!("obsidian:{id}"), kind: "obsidian", config: json!({"root": vault_path, "poll_secs": scan_interval.as_secs()}), last_scan, … }` with fresh heartbeat (errors land in `last_scan.errors`? keep `last_scan` from last success; heartbeat always bumps).
  - vault path missing → `mark_run_failure` with a clear message; keep looping (it may appear).
  - tenant = `kyma_core::tenant::DEFAULT_TENANT` (single-tenant local mode; lib.rs:351 precedent).

- [ ] **Step 1: Tests first** (inline `#[cfg(test)]` in `source_watchers.rs`) for `reconcile_plan`:

```rust
#[test] fn starts_new_rows() { /* empty running + 1 desired → [Start] */ }
#[test] fn stops_removed_or_disabled_rows() { /* running has id not in desired → [Stop] */ }
#[test] fn restarts_on_fingerprint_change() { /* same id, different fingerprint → [Stop, Start] */ }
#[test] fn noop_when_unchanged() { /* identical → [] */ }
```

(Write the four with real literals — `DesiredSource { id: "a".into(), fingerprint: "f1".into(), vault_path: "/v".into(), realm: "v".into(), scan_interval: Duration::from_secs(60) }` etc.)

- [ ] **Step 2: Run → FAIL**, **Step 3: implement `reconcile_plan` + manager + loop**, **Step 4: `cargo test -p kyma-local source_watchers` → PASS; `cargo build -p kyma-local` OK.**
- [ ] **Step 5: Spawn in `serve()`** (after the cc-sync watcher block, lib.rs ~878):

```rust
// Vault watchers: continuous sync loops for watcher-driven data sources
// (obsidian). Reconciles rows ↔ running loops; respects enable/pause.
{
    let eng = engine.clone();
    let status = watcher_status.clone();
    let control = ds_control.clone(); // move the Arc created above
    let pool_w = pool.clone();
    tokio::spawn(async move {
        source_watchers::run_manager(eng, status, control, pool_w).await;
    });
    info!("vault watcher manager running (obsidian sources)");
}
```

(`ds_control` is currently moved into `tick_deps` — clone it before, or create the manager's own `SqliteDataSourceControl::new(pool.clone())`.)

- [ ] **Step 6: Commit** — `feat(local): vault watcher manager — notify-driven continuous sync for watcher data sources`

---

### Task 6: SDK type updates (@kyma-ai/client)

**Files:**
- Modify: `packages/client/src/datasources.ts`

- [ ] **Step 1:** `CatalogEntry`: add `drive_model: "periodic" | "watcher" | string;` and widen `status: "available" | "coming_soon" | "installed";`. `DataSourceWatcher`: widen `kind: "filedrop" | "cc_sync" | "obsidian" | string;`.
- [ ] **Step 2:** Build the package: `pnpm --filter @kyma-ai/client build` (verify script name in `packages/client/package.json`). Expected: clean.
- [ ] **Step 3: Commit** — `feat(sdk): drive_model + installed status + obsidian watcher kind`

---

### Task 7: Brand icons (claude + obsidian)

**Files:**
- Modify: `web/src/features/datasources/BrandIcon.tsx`

- [ ] **Step 1:** Import `SiClaude`, `SiObsidian` from `@icons-pack/react-simple-icons`; add `claude: SiClaude, obsidian: SiObsidian` to `BRAND`; add dark-mode overrides `claude: "#D97757"` (Claude terracotta reads fine on dark; default light color also `#D97757` via the pack) and `obsidian: "#A88BFA"` (lightened from #7C3AED) to `DARK_BRAND_COLOR`.
- [ ] **Step 2:** `pnpm --dir web typecheck` → clean. Commit — `feat(web): claude + obsidian brand icons`

---

### Task 8: Unified page — local sync rows + list merge + tab removal

**Files:**
- Create: `web/src/features/datasources/LocalSyncRows.tsx` (CcSyncRow + FiledropRow + helpers; absorbs SyncTab's realm table + WatchersTab's card internals)
- Modify: `web/src/routes/_app.data-sources.index.tsx` (compose unified list)
- Modify: `web/src/routes/_app.data-sources.tsx` (drop tab bar, update subtitle)
- Modify: `web/src/routes/_app.data-sources.watchers.tsx`, `_app.data-sources.sync.tsx` → redirect to `/data-sources`; `_app.data-sources.memories.tsx` → redirect to `/memory/search`
- Modify: `web/src/features/datasources/DataSourceRow.tsx` (watcher-driven rows: liveness + "Continuous"; hide Sync-now)
- Delete: `web/src/features/datasources/DataSourcesTabs.tsx`, `SyncTab.tsx`, `WatchersTab.tsx`, `MemoriesSummaryTab.tsx` (move `LiveBadge`, `watchedTarget`, `formatScanDuration` into `LocalSyncRows.tsx`; fold MemoriesSummaryTab's value into the existing /memory pages — it's a provenance summary; drop it here)
- Tests: `web/src/features/datasources/LocalSyncRows.test.tsx` (replaces `SyncTab.test.tsx` + `WatchersTab.test.tsx` + `MemoriesSummaryTab.test.tsx`)

**CcSyncRow behavior** (rendered ALWAYS, even with no watcher data):
- Left: `BrandIcon brand="claude"`, name "Claude Code", badge "Pre-installed", sub-line `Watching <root> · every Ns · heartbeat <rel>` when live, or "Paused — memory sync is off" / "Starting…" states.
- Right: `LiveBadge` (when watcher present), pause/enable toggle via `useWatcherSettings`/`useUpdateWatcherSettings`, chevron expand.
- Expanded: per-realm table (exact columns from `SyncTab`: Upserted/Skipped/User edited/Edges/Archived from `last_scan.detail.realms`) + "Browse memories →" link to `/memory`.
- Driven by `useDataSourceWatchers()` rows `kind === "cc_sync"` + settings. Multiple cc_sync watchers (other nodes, server mode) → one row each; zero → the single paused/starting pre-installed row.

**FiledropRow**: previous WatcherCard rendering for `kind === "filedrop"` rows, same list.

**Obsidian liveness in `DataSourceRow`**: accept optional prop `watcher?: DataSourceWatcher`; when provided (parent joins by `w.id === \`obsidian:${ds.id}\``): replace the interval-ish "Last sync" line with `Continuous · last scan {processed}/{seen} notes · {relTime}` + `LiveBadge`; hide the Sync-now button when `entry.drive_model === "watcher"`.

**Index route composition** (inside the existing `ControlPlaneGate`):

```tsx
<div className="mx-auto flex max-w-3xl flex-col gap-2">
  <div className="mb-2 flex items-center justify-between">
    <h2 className="text-sm font-semibold text-muted-foreground">Sources</h2>
    <Button onClick={() => setAddOpen(true)}><Plus …/> Add data source</Button>
  </div>
  <CcSyncRow />
  {filedropWatchers.map(w => <FiledropRow key={w.id} watcher={w} />)}
  <DataSourcesList onAdd={…} watchers={watchers} />
</div>
```

(`DataSourcesList` threads `watchers` down to rows for the obsidian join; keep its empty state but it no longer represents an empty page since CcSyncRow always renders.)

**Redirect routes** (TanStack Router):

```tsx
import { createFileRoute, redirect } from "@tanstack/react-router";
export const Route = createFileRoute("/_app/data-sources/watchers")({
  beforeLoad: () => { throw redirect({ to: "/data-sources" }); },
});
```

(memories → `/memory/search`.)

- [ ] **Step 1: Write `LocalSyncRows.test.tsx` first** — mock `./useDataSources` hooks (follow the mocking pattern in the existing `SyncTab.test.tsx` before deleting it): renders pre-installed row with realms expanded after click; paused state shows Enable button which calls `updateSettings({cc_sync_enabled: true})`; filedrop row renders prefixes. Run `pnpm --dir web test` → FAIL.
- [ ] **Step 2: Implement** all file changes above.
- [ ] **Step 3:** `pnpm --dir web test && pnpm --dir web typecheck && pnpm --dir web lint` → PASS (delete the three old test files as part of this).
- [ ] **Step 4: Commit** — `feat(web): unified data sources page — pre-installed claude code row, watchers inline, tabs removed`

---

### Task 9: Wizard + catalog grid for watcher/installed entries

**Files:**
- Modify: `web/src/features/datasources/DataSourceCatalog.tsx` (status `installed` → "Installed" badge, card disabled/non-clickable)
- Modify: `web/src/features/datasources/AddDataSourceWizard.tsx` (drive_model `watcher` → no interval picker; show "Continuous — synced by a file watcher"; schedule_ms = entry.default_schedule_ms)
- Modify: `web/src/features/datasources/datasource-kinds.ts` (helper)
- Test: extend `LocalSyncRows.test.tsx` or new `datasource-kinds.test.ts`

- [ ] **Step 1:** add helper + test first:

```ts
/** Watcher-driven sources sync continuously — no interval to pick. */
export function isWatcherDriven(entry: CatalogEntry): boolean {
  return entry.drive_model === "watcher";
}
```

```ts
// datasource-kinds.test.ts
it("watcher-driven entries skip the interval picker", () => {
  expect(isWatcherDriven({ ...base, drive_model: "watcher" })).toBe(true);
  expect(isWatcherDriven({ ...base, drive_model: "periodic" })).toBe(false);
});
```

- [ ] **Step 2:** wire into wizard (conditional render around the interval section; review step shows "Continuous") and catalog grid (`status === "installed"` card: muted, badge, no onClick; `status === "coming_soon"` unchanged).
- [ ] **Step 3:** `pnpm --dir web test && pnpm --dir web typecheck` → PASS. Commit — `feat(web): wizard + catalog handle watcher-driven and installed sources`

---

### Task 10: Workspace-wide validation

- [ ] `cargo fmt --all -- --check` (fix if needed)
- [ ] `cargo clippy --workspace --all-targets` → no new warnings
- [ ] `cargo test --workspace` (Docker needed for testcontainers crates) → PASS
- [ ] `pnpm --dir web build` (tsc -b + vite) → PASS
- [ ] Commit any fixes — `chore: clippy/fmt fixes`

---

### Task 11: Live end-to-end verification (kyma serve + real vault)

- [ ] **Step 1:** Build with fresh embedded UI: `pnpm --dir web build && cargo build -p kyma-bin` (confirm kyma-web-assets picks up `web/dist`; check its build.rs/include path).
- [ ] **Step 2:** Temp env: `KYMA_HOME=$(mktemp -d) KYMA_LOCAL_DB=… kyma serve` on a free port; temp vault dir with `Alpha.md`, `Beta.md` (`[[Alpha|link]]`), `.obsidian/app.json`.
- [ ] **Step 3:** Via API: create the obsidian source (`POST /v1/data-sources` type obsidian, vault_path) → within ~20s assert: `/v1/data-sources/watchers` has `obsidian:<id>` non-stale; detail shows `last_success_at` + rows; memory_nodes contain both notes (query via the SQL endpoint); RELATES_TO edge exists.
- [ ] **Step 4:** Mutations: edit Alpha.md (watch loop picks it up — new version), add `Gamma.md` (appears), delete Beta.md (archived). Pause source → watcher row disappears; resume → returns.
- [ ] **Step 5:** UI: screenshot `/data-sources` (browser-harness) — single page, Claude row with real icon + expandable realms, obsidian row Continuous + Live, no tabs; catalog wizard shows Obsidian (no interval) + Claude Code "Installed".
- [ ] **Step 6:** Fix anything found; commit fixes.

---

### Task 12: Merge

- [ ] `git checkout main`-equivalent merge: main is checked out at `/private/tmp/kyma-main-demo`; merge from there or temporarily detach. Use: `git fetch origin && git merge --no-ff feat/unified-sources-obsidian` on main, then `git push origin main` (user's preferred local-merge flow). Resolve the worktree constraint by running the merge in `/private/tmp/kyma-main-demo` after `git pull`.
- [ ] Update memory (MEMORY.md + project file) with the shipped state.

---

## Self-review notes

- Spec coverage: drive_model (T2), obsidian source (T3), engine (T4), watcher manager (T5), SDK (T6), icons (T7), unified page + redirects + cc row (T8), wizard/catalog (T9), validation (T10–11), merge (T12). Wikilink normalization (T1). ✓
- cc_sync stays untouched apart from `pub(crate)` visibility — its loop-guard semantics are not generalized. ✓
- Watcher rows never enter the periodic scheduler (`list_due_periodic` filters `drive_model = 'periodic'`, both SQLite + Postgres) and `trigger` on an obsidian row enqueues a job whose run_once fails Permanent → acceptable; UI hides Sync-now for watcher rows. ✓
