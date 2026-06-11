# Data Sources — rename "connectors" end-to-end + sectioned module

**Date:** 2026-06-11
**Status:** Approved (brainstorm with user; all decisions locked)

## Summary

Rename the "connectors" concept to **Data Sources** across the entire stack — clean break, no
legacy aliases — and restructure the web surface into a tabbed **Data Sources** module with four
sections: Sources (today's connectors), File Watchers (new registry with node/identity
provenance), Claude Code Memory Sync, and a Memories sources-lens summary.

Decisions made with the user:

1. **Full rename, clean break** — no `/v1/connectors` aliases, no deprecated CLI subcommand,
   no client-lib re-exports. Kyma is v0.0.1; back-compat shims are dead weight.
2. **Everything is just "Data Sources"** — no separate kind-name ("Connectors"/"Integrations")
   for the GitHub/Jira-style pull integrations. The crate, trait, and types rename too.
3. **Watcher registry + heartbeat** — file watchers become first-class registered entities with
   persisted node + identity provenance (new feature, not just UI).
4. **Memories stay canonical at `/memory`** — Data Sources shows a provenance-grouped summary
   linking into the memory module; no duplicate browsing UI.
5. **Tabbed module layout** — like `/memory`: deep-linkable tabs, Sources as the default tab.

## 1. Naming map (no exceptions)

| Today | Becomes |
|---|---|
| crate `kyma-connectors` (`crates/kyma-connectors/`) | crate `kyma-datasources` (`crates/kyma-datasources/`) |
| crate `kyma-connector-core` | crate `kyma-datasource-core` |
| `Connector` trait, `ConnectorCtx`, `ConnectorRun`, `ConnectorRegistry`, `ConnectorRunner`, `ConnectorControl`, `ConnectorAdminState` | `DataSource`, `DataSourceCtx`, `DataSourceRun`, `DataSourceRegistry`, `DataSourceRunner`, `DataSourceControl`, `DataSourceAdminState` |
| DB tables `connectors`, `connector_cursors`, `connector_leases` | `data_sources`, `data_source_cursors`, `data_source_leases` |
| `background_tasks.kind = 'connector_tick'` | `'data_source_tick'` |
| REST `/v1/connectors/*` | `/v1/data-sources/*` (catalog, `:id`, pause/resume/trigger, `github/repos`) |
| CLI `kyma connector <list\|add\|show\|pause\|resume\|remove\|trigger>` | `kyma datasource <…>` |
| CLI `kyma ingest status --connector` | `--datasource` |
| client `packages/client/src/connectors.ts` (`listConnectors`, `ConnectorSummary`, `ConnectorDetail`, `ConnectorStatus`, `deriveStatus`, …) | `datasources.ts` (`listDataSources`, `DataSourceSummary`, `DataSourceDetail`, `DataSourceStatus`, …) |
| web `web/src/features/connectors/`, `web/src/sdk/connectors.ts`, routes `_app.connectors.*` | `features/datasources/`, `sdk/datasources.ts`, routes `_app.data-sources.*` (URL `/data-sources`) |
| MCP tools `list_connectors`, `connector_read` (`crates/kyma-server/src/agent/connector_tools.rs`) | `list_data_sources`, `data_source_read` (`datasource_tools.rs`) |
| capability field `Capabilities.connectors` (`kyma-server/src/capabilities.rs:17`, serialized key `connectors`) + web `useCapability("connectors")` | `Capabilities.data_sources` / `useCapability("data_sources")` |
| docs `docs/site/connectors/` (18 pages) | `docs/site/data-sources/` |
| Sidebar item "Connectors" (Plug icon, Build group) | "Data Sources" (Database icon, Build group) |

**Unchanged:** source type ids (`"github"`, `"prometheus"`, …) and per-type config schemas
(`config_jsonb` keys) — no stored-data rewrite beyond table renames. Credential model
(`credential_id`, pat/oauth/url/none) unchanged. Memory `provenance.source` values unchanged.
`SCHEDULE_MS_MIN/MAX` and the drive-model enum (`periodic`/`continuous`) unchanged.

**crates.io:** the old `kyma-connectors` / `kyma-connector-core` names stay dormant at their
published 0.0.1; new names publish at the next version tag. Publishing is out of scope for this
change.

## 2. Database migration

New migration `027_data_sources_rename.sql` (next number in `crates/kyma-catalog/migrations/`,
adjust if another migration lands first):

```sql
ALTER TABLE connectors RENAME TO data_sources;
ALTER TABLE connector_cursors RENAME TO data_source_cursors;
ALTER TABLE connector_leases RENAME TO data_source_leases;
-- rename FK constraints + connectors_enabled_drive_idx accordingly
UPDATE background_tasks SET kind = 'data_source_tick' WHERE kind = 'connector_tick';
-- recreate the background_tasks partial unique index against the new kind value
```

Plus the new watcher-registry table (§4). Migration must be tested against a database that has
pre-rename rows (existing connectors, cursors, queued ticks survive).

## 3. Web UI — tabbed Data Sources module

TanStack file routes, mirroring the `/memory` module pattern:

```
_app.data-sources.tsx            # layout: header + tab bar
_app.data-sources.index.tsx      # Sources tab (default) — today's list, grouped by category
_app.data-sources.$id.tsx        # source detail (status, config, runs) — unchanged behavior
_app.data-sources.watchers.tsx   # File Watchers tab
_app.data-sources.sync.tsx       # Claude Code Memory Sync tab
_app.data-sources.memories.tsx   # Memories summary tab
```

- **Sources** — existing list/add-wizard/detail (catalog grid, status badges, OAuth flow,
  repo picker) renamed and re-homed; behavior unchanged.
- **File Watchers** — table of registered watchers: kind (`filedrop` / `cc_sync`), node host,
  identity, watched prefixes/dirs, poll interval, last heartbeat, last-scan stats
  (seen/processed/errors), staleness badge.
- **Claude Code Memory Sync** — per-realm sync status from `ProjectSyncReport` data: last sync,
  upserted/skipped/user_edited/archived counts, edges added, resident-watcher on/off.
- **Memories** — counts grouped by `provenance.source` per realm + CC-sync rollup; cards link
  into `/memory` (search pre-filtered by source where possible). `/memory` remains canonical.

Capability gate `useCapability("data_sources")` wraps the module as today.

## 4. Watcher registry (new feature)

New table `data_source_watchers`:

```
id                uuid pk
kind              text      -- 'filedrop' | 'cc_sync'
node_host         text      -- hostname
node_id           text      -- stable node identifier (existing node id if present, else hostname)
identity          text      -- OS user; auth principal appended when available
config_jsonb     jsonb     -- prefixes/dirs, poll interval, flags (delete_after_ingest, …)
started_at        timestamptz
last_heartbeat_at timestamptz
last_scan_jsonb   jsonb     -- { seen, processed, errors, duration_ms, at }
```

- `FiledropWatcher::run()` (kyma-ingest-filedrop) and the cc-sync watcher (kyma-local,
  `KYMA_CC_WATCH`) register on startup (upsert by `(kind, node_id, config hash)`) and update
  `last_heartbeat_at` + `last_scan_jsonb` each poll cycle.
- Staleness is computed read-side: heartbeat older than 3× poll interval ⇒ stale badge.
  Rows with no heartbeat for 24h are pruned (server-side sweep on read or scheduled task).
- Exposed at `GET /v1/data-sources/watchers` → `{ items: [WatcherSummary] }`; included in the
  client lib (`listDataSourceWatchers()`).
- CC-sync status for the Sync tab is served from the same registry row (`last_scan_jsonb`
  carries the per-realm `ProjectSyncReport` rollup for cc_sync watchers).

## 5. Error handling

- Watcher registration/heartbeat failures must never break ingestion: log + continue
  (best-effort writes, no panic, no retry storm).
- Migration is transactional; on failure the old names remain and the server keeps working.
- API 404s on old paths are acceptable (clean break) — no redirects.

## 6. Testing

- Rename-following: existing integration tests (`admin_it`, `runner_it`, `scheduler_it`,
  `registry`, `validate_config`, `secret_store`, multi-table, prometheus) updated and green.
- Migration test: seed pre-rename schema + rows, run migration, assert data intact and
  scheduler ticks resume under `data_source_tick`.
- Watcher registry: unit/integration tests for register, heartbeat update, staleness
  computation, prune, and the `/v1/data-sources/watchers` endpoint.
- Web: e2e (on-demand suite) — tab navigation, sources list renders, watchers tab renders
  registry rows, memories tab shows provenance counts; grep gate: no user-facing "Connector"
  strings remain in web/ or docs/.
- CLI: `kyma datasource list/add/show/pause/resume/remove/trigger` smoke tests.

## 7. Out of scope

- Publishing renamed crates to crates.io (happens at next version tag).
- Any new source types or changes to source behavior.
- Memory module changes beyond the summary tab linking into it.
- Multi-node lease/HA work for watchers (registry is observational only).
