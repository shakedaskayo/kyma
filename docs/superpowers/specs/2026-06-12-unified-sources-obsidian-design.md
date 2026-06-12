# Unified Data Sources page + Obsidian vault source

**Date:** 2026-06-12
**Status:** Approved (autonomous run — user directive: "fully working, production ready, well tested and validated, then merge")

## Goal

1. **Merge the Data Sources page** — kill the Sources / File Watchers / Memory Sync / Memories tab split; one unified page where everything feeding the context graph lives in a single list.
2. **Claude Code as a pre-installed source** — the cc-sync watcher presents as a first-class, pre-installed data source row with the real Claude brand icon, an enable/pause toggle, live status, and its per-realm sync stats expandable inline.
3. **Obsidian vaults as a real end-to-end source** — user adds a vault by path; notes sync continuously into memory nodes (embedded, graph-linked, recallable) driven by a filesystem watcher (`notify`) with a periodic full-scan fallback; wikilinks become `RELATES_TO` edges; deletions archive.

Non-goals: server-side (Postgres control plane) execution of vault watchers on remote nodes (the node-daemon fabric can adopt the same source type later); Obsidian canvas/attachment ingestion (markdown notes only, v1).

## Decisions (alternatives considered)

| Decision | Chosen | Rejected because |
|---|---|---|
| Where Obsidian source state lives | A real `data_sources` row (type `obsidian`), created via the existing wizard/API | A second `watcher-settings.json`-style config would fork the CRUD/UI/credentials plumbing the data-sources stack already provides |
| How sync runs | New `drive_model = "watcher"`: excluded from the periodic scheduler; a local watcher manager spawns one notify-driven loop per enabled row | Scheduler-driven `run_once` can't hold an fs-watcher across ticks; pure polling misses the "continuous" requirement |
| What notes become | Memory nodes via `MemoryWriter` (like cc-sync): embedded, realm = vault, wikilink edges, archive-on-delete | Plain table rows would skip recall/graph — the whole point of a knowledge vault |
| Sync engine | New `vault_sync.rs` in `kyma-local`, modeled on `cc_sync.rs` but vault-flavored (frontmatter optional, recursive walk, Obsidian link syntax) | Generalizing `cc_sync.rs` itself would tangle Claude-specific loop-guard/writeback semantics into the vault path |
| UI shape | One list: pinned "pre-installed" Claude Code row + all configured sources (obsidian + remote) below; watcher liveness merged into rows; tab bar removed | Keeping a separate watchers section preserves the machinery-oriented split the user explicitly wants gone |
| Claude Code in catalog | Catalog entry `claude_code` with new `status: "installed"` (badge, not creatable) | Hiding it from the catalog makes the "pre-installed" story invisible at the add-source moment |

Agent-agnosticism: the pinned section is "synced from this machine" by *kind*, not hard-coded to Claude — cc_sync is one watcher kind; future agent-memory syncs (Cursor, Windsurf, …) appear as sibling rows with their own brand.

## Backend

### 1. `CatalogEntry.drive_model`

Add `pub drive_model: String` (serialized; `"periodic"` default in `minimal()` / `soon()`). `POST /v1/data-sources` stops hardcoding `"periodic"` and uses the registered source's `catalog().drive_model`. Periodic scheduling queries already filter `WHERE drive_model = 'periodic'`, so `"watcher"` rows are never ticked. The UI uses it to swap the interval picker for a "Continuous — file watcher" label.

### 2. `ObsidianDataSource` (`crates/kyma-datasources/src/obsidian.rs`)

- `type_id: "obsidian"`, label "Obsidian", category `knowledge`, brand `obsidian`, `auth_mode: "none"`, status `available`, `drive_model: "watcher"`.
- Fields: `vault_path` (text, required — absolute path or `~/…`).
- `validate_config`: `vault_path` present, non-empty string.
- `run_once`: returns `Err(Permanent)` — defensive; the scheduler never ticks watcher rows. Registered in kyma-local (and server registry, where it appears in the catalog; execution requires a node-side watcher).

### 3. Vault sync engine (`crates/kyma-local/src/vault_sync.rs`)

Modeled on `cc_sync.rs`, sharing its invariants:

- **Walk**: recursive over the vault root; only `*.md`; skip `.obsidian/`, `.trash/`, and dot-directories.
- **Frontmatter optional**: plain markdown ingests as-is; YAML frontmatter (when present) contributes `tags` (string or list); body excludes frontmatter.
- **Identity**: `topic_key = obsidian:<source_id>/<relative_path>`; realm = vault name (config `name` falling back to directory basename); `memory_type: Fact`; tags `["obsidian"] + frontmatter tags`; importance **0.6**; provenance records source id, vault path, rel path, content hash, ingested_at.
- **Idempotency**: normalized content hash per file in `sync_state` (`obsidian:hash:<source_id>:<rel_path>`); manifest per source (`obsidian:manifest:<source_id>`) drives archive-on-delete exactly like cc-sync.
- **Wikilinks**: new `wikilink::extract_normalized` handles Obsidian syntax — `[[Target|alias]]`, `[[Target#heading]]`, `[[Target^block]]`, embeds `![[…]]` — normalizing to the target note name; resolution by file stem (case-insensitive, Obsidian semantics); edges `RELATES_TO` with `{"via": "obsidian-wikilink"}`; only re-emitted for changed endpoints (deterministic ids).
- **Report**: `VaultSyncReport` → the watcher `last_scan` shape `{seen, processed, errors, duration_ms, at, detail: {realm, notes, edges_added, archived}}`.

### 4. Watcher manager (`crates/kyma-local/src/source_watchers.rs`, spawned from `lib.rs` next to the cc-sync loop)

- Reconciler loop (every ~15s): list `data_sources` rows with `drive_model = 'watcher'`; diff desired vs running by `(id, config_hash)`; spawn/stop/restart per-source tasks. The diff is a pure function (`reconcile_plan(running, desired) -> {start, stop}`) for unit-testability.
- Per-source task: `notify::recommended_watcher` (recursive) on the expanded vault path → debounce (2s) → incremental pass; plus a full-scan fallback every `max(schedule_ms, 30s)`. Each pass:
  - runs `vault_sync::run_once`,
  - upserts `LocalWatcherStatus` (kind `obsidian`, id `obsidian:<source_id>`, config `{root, poll_secs}`, heartbeat, last_scan),
  - records run outcome on the data-source row (`record_run_success/failure`, rows = upserted+user-edited) so the standard row UI shows Last sync / rows / error.
- `enabled = false` (pause) → task stops, watcher entry removed; resume restarts. Vault path missing → run failure recorded (`error` status surfaces in row), heartbeat continues.
- New dep: `notify = "6"` in `kyma-local`.

### 5. Claude Code catalog presence

Static catalog entry `claude_code` (label "Claude Code", category `knowledge`, brand `claude`, auth `none`, `status: "installed"`, drive_model `watcher`) merged in `coming_soon()`'s sibling `installed()` list. Not creatable (`POST` with an unregistered type still 400s — unchanged).

## Web UI

### 6. Unified page (`/data-sources`)

- Layout route drops `DataSourcesTabs`; old routes `watchers`, `sync` redirect to `/data-sources`; `memories` redirects to `/memory/search`. `$id` detail unchanged.
- Page body, one column:
  1. **Pre-installed row(s)** — `LocalSyncRow` for cc_sync: real `SiClaude` icon, "Pre-installed" badge, Live/Stale badge, watched root + node + last scan line, Pause/Enable toggle (existing watcher-settings API), chevron-expandable per-realm table (the old SyncTab content) + "Browse memories →" link. Rendered from `/v1/data-sources/watchers` + settings; shown (as paused) even when the watcher is off.
  2. **Configured sources** — existing `DataSourceRow`s for all `data_sources` rows. Obsidian rows join watcher liveness by id (`obsidian:<source_id>`): LiveBadge + "Continuous" instead of an interval, last-scan notes count.
  3. **Filedrop watcher rows** (when running) render with the folder icon as today, in the same list.
- "Add data source" button + wizard stay; catalog grid shows Obsidian (available) and Claude Code with an "Installed" badge (card disabled).
- Wizard: when `entry.drive_model === "watcher"`, hide the interval picker and show "Continuous — synced by a file watcher"; schedule_ms sent as the full-scan fallback (default 60s).

### 7. Icons

`BrandIcon`: add `claude: SiClaude`, `obsidian: SiObsidian` (pack already at 13.13.0); dark-mode colors (`claude` #D97757 both modes; `obsidian` #7C3AED with a lighter dark variant). cc_sync stops using the generic `Brain` glyph everywhere.

## Testing

- **kyma-ccmem**: wikilink normalization table tests (alias, heading, block, embed, dedupe, unterminated).
- **kyma-local `vault_sync`** (pattern: `cc_sync_unit_tests.rs`, stub embedder + tempdir engine): ingest with/without frontmatter; idempotent rescan (skip on hash); edit → same node new version; delete → archived + un-archive on reappear; wikilink edges incl. alias + case-insensitive resolution; `.obsidian/` exclusion; realm naming.
- **kyma-local `source_watchers`**: `reconcile_plan` unit tests (new row → start; disabled → stop; config change → restart; unchanged → no-op).
- **kyma-datasources**: obsidian `validate_config`; catalog includes drive_model; create honors registry drive_model (admin_it).
- **web (vitest)**: LocalSyncRow (toggle, expand realms, paused state), obsidian row liveness merge, wizard hides interval for watcher entries, redirects. Migrate/retire SyncTab/WatchersTab tests.
- **Validation**: `cargo build && cargo clippy && cargo test` (workspace), `pnpm typecheck && pnpm test && pnpm build` (web), then a live `kyma serve` pass against a real temp vault — add a note, edit it, link it, delete it — verifying memory nodes/edges/archive and the UI page end-to-end.

## Merge

Branch `feat/unified-sources-obsidian` (off main d549d226); local `git merge --no-ff` into main + push when validated.
