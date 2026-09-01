# Docs update: coding-agent integrations, dreaming, workers, graph, MCP + full alignment pass

## Context

The docs site (VitePress, `docs/site/`, now live at https://shakedaskayo.github.io/pensieve/) lags the engine: the worker fabric, dreaming runs, agentic memory (M0+M1), the per-agent-kind detector registry, the MCP server, and the graph layer all merged to main recently, but the Reference section documents none of their HTTP/CLI/env surface, there is no MCP or coding-agents overview page, and the graph layer — heavily advertised on the landing page — has no docs at all. Some existing pages also carry factual drift (e.g. `agent/engines.md` lists the wrong Opus model for the Claude-CLI engine). Goal: document the new surfaces and align every claim in the docs with code.

**User decisions:** graph docs live under **Query** (no new top-level section); scope includes **expanding the thin observability page**; FinOps/Security recipes are explicitly out of scope. Hard constraint (standing user feedback): pensieve is **coding-agent-agnostic** — Claude Code is one integration, never "the" integration.

**Out of scope:** Railway teardown (separate open thread), FinOps/Security recipes, any engine code changes.

## Ground truth (verified against code this session — re-verify in-task before writing)

- **Engine models differ per engine** (`crates/pensieve-server/src/agent/engine/*.rs`): anthropic → opus-4-7/sonnet-4-6/haiku-4-5 (default sonnet-4-6) — docs already correct; **claude_cli → opus-4-8**, sonnet-4-6, haiku-4-5, `default` — docs currently say 4-7, wrong; openai → gpt-5/gpt-5-mini/gpt-4.1/o4-mini — correct; ollama → gemma4 default, live-fetched list.
- **CLI** (`crates/pensieve-cli/src/main.rs`): `pensieve setup <agent>` is **positional** (`claude-code|cursor|windsurf|list`, `--print`); `pensieve worker` has two worlds — local sync service (`install/uninstall/status`) and fabric daemon (`run/create/list/revoke`); plus `sessions`, `recall`, `remember`, `entity`, `distill`, `mcp`, `deploy`.
- **Routes**: agent router in `crates/pensieve-server/src/agent/routes.rs` (sessions, engine(s), memory query/overview/settings/export/import/changes, dreaming run/runs/:id); fabric in `fabric_handler.rs` (`/v1/workers/register|heartbeat`, `/v1/jobs/claim|self|:id/{progress,complete,fail,lease}`, admin list/create/revoke); graph in `graph_handler.rs` (`/v1/graph`, `/v1/graph/{g}/overview|stats|schema|nodes/{id}|nodes/{id}/subgraph|neighbors|search`); MCP HTTP `/mcp/v1` (`pensieve-bin/src/main.rs`), stdio `pensieve mcp`.
- **MCP tools** (`crates/pensieve-mcp/src/tools.rs` + `connector_tools.rs`): 21 base + 2 connector = **23 — count from the registry at write time**, don't hardcode.
- **Memory defaults** (`memory_settings.rs`, `pensieve-memory/src/lib.rs`): dreaming OFF by default, mode `full|housekeeping_only|sources`, max_tool_calls 100, wall_clock 600s, connector_read_budget 25 / 4 MiB, mutation_cap 60; blend W_RRF 1.0, W_SEMANTIC 0.6, W_KEYWORD 0.3, W_GRAPH 0.5, W_IMPORTANCE 0.4, W_RECENCY 0.3, RRF_K 60, HALF_LIFE 30d.
- **Detector registry** (`crates/pensieve-local/src/agent_sources.rs`): `claude-code` full pipeline; `cursor`/`windsurf`/`codex` detection-only (source_sync no-op until pipelines land — frame as roadmap). Setup targets (`setup.rs`): claude-code→`.mcp.json`, cursor→`.cursor/mcp.json`, windsurf→`.codeium/windsurf/mcp_config.json`; no codex target yet.
- Claude Code hooks/plugin **are shipped** (`integrations/claude-code/pensieve-memory/`, `pensieve install-plugin`) — ignore stale M5 "deferred" note in the memory spec; trust code.
- Deferred (frame as roadmap, never as shipped): remote execution of `dreaming`/`connector_sync` on daemons; cursor/windsurf/codex ingestion; M2–M7 memory hardening items.

## Branch & workflow

New branch `docs/coding-agents-graph-mcp` off `origin/main`, worked in the existing worktree (`.claude/worktrees/feat+docs-github-pages` — currently at merged-main state; create the new branch there). Execute via subagent-driven development (implementer + spec review per task, with explicit worktree directory guards in every subagent prompt — subagents have escaped to the primary checkout before). Every task ends with `pnpm -C docs/site build` (dead-link check is part of the build) and a commit. Merge to main at the end auto-deploys via `.github/workflows/docs.yml`; finish with live-site verification.

Copy this plan to `docs/superpowers/plans/2026-06-07-docs-coding-agents-graph-mcp.md` on the branch (repo convention).

## Tasks

### 1. `reference/api.md` — add the missing endpoint families
Add sections (match existing table style): agent sessions (`GET/DELETE /v1/agent/sessions*`, `/turns`), engine config (`GET /v1/agent/engines`, `GET/PUT /v1/agent/engine`, `POST /v1/agent/engine/test`), memory (`POST /v1/agent/memory/query` fast|agentic, `GET overview|settings|export|changes`, `PUT settings`, `POST import`), dreaming (`POST /v1/agent/memory/dreaming/run` 202/200-deduped semantics, `GET runs`, `GET runs/:id`), worker fabric — worker-facing (`POST /v1/workers/register|heartbeat`, `POST /v1/jobs/claim|self`, `POST /v1/jobs/:id/progress|complete|fail|lease`, `kyw_` bearer) and admin (`GET /v1/workers`, `POST /v1/workers/:id/create|revoke`), graph (all 8 routes), `/mcp/v1` row pointing to the new MCP page.
Sources: `routes.rs`, `fabric_handler.rs`, `graph_handler.rs`. Cross-link `/agent/workers`, `/agent/dreaming`, `/agent/memory`, `/query/graph`, `/reference/mcp`.

### 2. `reference/env.md` + `reference/cli.md` — align to code
Env: `grep -rn 'PENSIEVE_[A-Z_]*' crates/ --include='*.rs'`; add the fabric/worker/session vars that exist (`PENSIEVE_FABRIC_LEASE_SECS|OFFLINE_SECS|SWEEP_SECS|WORKERS`, `PENSIEVE_WORKER_TOKEN`, `PENSIEVE_SERVER_URL`, `PENSIEVE_WORKER_INSECURE`, `PENSIEVE_SESSION_SUMMARY_EVERY`, `PENSIEVE_CC_SYNC_POLL_SECS`); drop any the grep can't find.
CLI: align every command/flag against the clap enums in `pensieve-cli/src/main.rs` — fix `pensieve setup` (positional), document both `pensieve worker` worlds, `pensieve mcp`, `pensieve deploy` ops, sessions/recall/remember/entity/distill flags.

### 3. `reference/kql-functions.md` — graph operators
Add `make-graph`, `graph-match`, `graph-traverse` (edge-type filters, multi-source seeds, max-hops) with one example each. Sources: `pensieve-kql` crate parser + `pensieve-graph/src/executor.rs`.

### 4. NEW `query/graph.md` + sidebar entry
Graph as a query surface: schema graph (free, synthetic from catalog) vs stored graphs (registered via `pensieve create-graph --db … --nodes … --edges …`); querying via KQL graph ops, MCP `graph_traverse`/`find_references_to`, HTTP `/v1/graph/*`; the `/graph` web UI (unified canvas incl. memory layer); how memory uses the graph. Frame landing's "million-node scale" claim accurately. Sidebar: add to `/query/` group in `.vitepress/config.ts`.

### 5. NEW `agent/coding-agents.md` + sidebar entry
The agnostic integration overview: integration-path matrix (Agent | Detected | Ingestion pipeline | Setup command | Extras) covering claude-code (full: plugin, hooks, firehose), cursor/windsurf (MCP setup target, detection-only sync), codex (detection-only, no setup target yet); detection-only vs full-pipeline callout; how the per-agent-kind detector registry works (`AgentDetector` trait, `source:<kind>` capability) and what "adding an agent" means; pointers to connect/connect-from-cli/claude-code-plugin/workers. Tone: Claude Code is one integration among several. Sidebar: after Connect in `/agent/` group.

### 6. NEW `reference/mcp.md` + sidebar entry
Transports (stdio `pensieve mcp` zero-infra vs HTTP `/mcp/v1` bearer-auth; tradeoffs); the generated `.mcp.json` shape from `setup.rs`; full tool catalog grouped Query / Memory / Graph / Connector (server-only) with one-line descriptions — exact count from the registry. Sidebar: after CLI in `/reference/` group.

### 7. `agent/memory.md` — settings + sync + query modes
Add: settings JSON (GET/PUT `/v1/agent/memory/settings`) with verified defaults and per-knob meaning (dreaming knobs incl. the three modes; blend weights with the RRF `1/(rrf_k+rank)` and recency `exp(-ln2·age/half_life)` formulas); export/import/changes incremental-sync loop; fast vs agentic `memory/query` modes. Keep existing lifecycle content; cross-link dreaming page.

### 8. `agent/engines.md`, `agent/dreaming.md`, `agent/workers.md` — alignment
engines.md: make model lists per-engine explicit, fix claude_cli opus → **claude-opus-4-8**, verify ollama defaults, brief CredentialResolver order + "adding a provider" pointer. dreaming.md: align knob names/defaults with settings; add manual-trigger API (`POST …/dreaming/run`) + dedup semantics + `/memory/dreaming` UI pointer. workers.md: already accurate — add cross-links (coding-agents page, fabric API section, env vars) and keep the remote-execution-deferred note.

### 9. `concepts/observability.md` — expand
`/metrics` Prometheus surface (histogram names from code grep `_duration_seconds|metrics`), request tracing (`X-Request-ID`), structured logs (`RUST_LOG`), agent run traces (`GET /v1/agent/runs/:id`), connector health (`pensieve_connector_health` queryable table), dreaming run observability (runs list + per-run activity). Example queries included.

### 10. Final sweep, merge, live verify
- Cross-page consistency grep: every page that names models/commands/endpoints (`grep -rn 'opus-4-7\|gpt-\|pensieve setup\|/v1/' docs/site/`) — verify or fix.
- Landing alignment: confirm the landing claims (graph, recursive intelligence, memory) now all have docs targets; update `index.md` feature links if needed.
- Clean `pnpm -C docs/site build`; spec-review subagent does a whole-branch pass.
- Merge to main (same plumbing approach if `/private/tmp/pensieve-main-demo` still holds main: `git merge-tree` → `commit-tree` → push by SHA), watch the `docs` workflow, then live-verify the three new URLs: `/pensieve/query/graph`, `/pensieve/agent/coding-agents`, `/pensieve/reference/mcp` (200s + spot-render via browser).

## Execution rules (every implementer subagent)

- **Directory guard**: every Bash invocation starts with `cd` into the worktree; verify `pwd` + `git branch --show-current` before edits.
- **Verify-then-write**: re-grep each claimed value in its cited source file in the same task before writing it into docs.
- Markdown links omit the `/pensieve/` base (VitePress prefixes); internal links like `/reference/mcp` — the build's dead-link check is the gate.
- Match existing voice: terse, example-first, tables for reference (model: `reference/api.md`, `agent/workers.md`).
- Deferred features framed as roadmap; never promise cursor/windsurf/codex ingestion, remote dreaming execution, or M2+ memory phases as shipped.

## Verification

1. Per task: `pnpm -C docs/site build` exits 0 (includes dead-link + diagram checks).
2. Final: consistency greps above; whole-branch review subagent.
3. Post-merge: `docs` workflow green; `curl -sI` the three new pages → 200; browser screenshot of `/pensieve/query/graph` and `/pensieve/agent/coding-agents`; confirm no unprefixed links (`grep 'href="/[a-z]' dist/index.html` → none).
