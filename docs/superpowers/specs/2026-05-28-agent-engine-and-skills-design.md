# Kyma Agent Engine + Skills — Master Design

**Status:** draft 2026-05-28. Decomposes into 5 phase plans (A1–A5). Each phase ships a working, testable slice on its own.

**Relationship to other specs:** This is an engine + web-UI feature track that builds on the existing `/v1/agent/*` surface (Ollama-only today). Sibling to `2026-05-25-graph-layer-context-graph-design.md` — the agent already calls the graph tools and will keep doing so. No conflict with the v1 production-readiness program.

---

## 1. Goal & non-goals

### Goal

Make **Ask Kyma** a properly configurable agent engine that:

1. Supports multiple LLM providers (Anthropic, OpenAI, Ollama, Bedrock, etc.) — pick one in the UI, swap any time.
2. Inherits credentials from the host when possible — `ANTHROPIC_API_KEY`/`OPENAI_API_KEY` env vars first, then `~/.claude/.credentials.json` (Claude Code's stored key), then the catalog credentials store. Users on a host that already runs Claude Code shouldn't have to type their key in again.
3. Has an optional **Claude Code CLI engine** that shells out to the host's `claude` binary instead of calling the API directly — for users who want Kyma to inherit *everything* (MCP servers, locally configured skills, file access, projects) Claude Code already has.
4. Discovers **skill upstreams** — local (`~/.claude/skills/*`) and remote (configurable git/HTTPS skill repos) — and lets the user enable a subset for the agent to use. Each enabled skill becomes available to the agent as additional context/tools.
5. Has a **better agent loop** — sessions persist to Postgres across restarts, mid-turn cancellation works, the SSE stream surfaces a clean reasoning trace, and tool/skill failures don't blow up the run.

The deliverable is **end-to-end and runnable**: a user with `ANTHROPIC_API_KEY` exported (or Claude Code installed) opens Kyma, lands on Ask Kyma, asks a question, and gets an answer from Claude Sonnet via the freshly-configured Anthropic engine — without ever leaving the UI to edit config.

### Non-goals

- **Multi-tenant model routing** (per-user keys, model quotas, billing) — deferred to the cloud-track program.
- **Fine-tuning / model hosting** — Kyma always *uses* a hosted/local model; never trains one.
- **Replacing the existing 8 inline tools.** Skills are additive context — they don't deprecate `run_kql`/`graph_traverse`/etc.
- **MCP servers as a separate concept.** Plan E (skills) treats remote MCPs the same as any other skill upstream; we don't ship a parallel MCP catalogue feature.

---

## 2. Architecture overview

```
                    ┌──────────────────────────────────────────────────────┐
 web/ (React)        │  /agent route → features/agent/                      │
                    │   AgentConsole · MessageCard · sse.ts                │
                    │   /settings/engine → EngineSettings (provider/model) │
                    │   /settings/skills → SkillsSettings (toggle list)    │
                    └─────────────┬────────────────────────────────────────┘
                                  │ HTTP + SSE (Bearer + X-Database)
                    ┌─────────────▼────────────────────────────────────────┐
 kyma-server         │  /v1/agent/ask                       (SSE)            │
                    │  /v1/agent/runs/{id}                                  │
                    │  /v1/agent/engine        (GET/PUT — provider config)  │
                    │  /v1/agent/engines       (GET — what's available)     │
                    │  /v1/skills             (GET — list discovered)       │
                    │  /v1/skills/sources     (GET/POST/DELETE — registry)  │
                    │  /v1/skills/enabled     (GET/PUT)                     │
                    └─────────────┬────────────────────────────────────────┘
                                  │
       ┌──────────────────────────┼──────────────────────────────────────┐
       │                          │                                      │
       ▼                          ▼                                      ▼
 EngineRegistry            CredentialResolver                      SkillRegistry
 (trait)                   (env → claude → catalog)              (local + remote)
       │                          │                                      │
       ├─ AnthropicEngine         │                                      ├─ LocalDiscovery (~/.claude/skills)
       ├─ OpenAIEngine            │                                      ├─ RemoteUpstream (git/HTTPS)
       ├─ OllamaEngine            │                                      └─ enabled set → injected as tools/system prompt
       ├─ ClaudeCliEngine ────────┴── spawns `claude` CLI; pipes JSON
       └─ … (bedrock, gemini, etc. via adk-rust features)
```

### Key shapes

**`EngineConfig`** — persisted in catalog, swappable at runtime:

```rust
pub struct EngineConfig {
    pub kind: EngineKind,             // anthropic | openai | ollama | claude_cli | …
    pub model: String,                // "claude-sonnet-4-6", "gpt-5", "gemma4:latest"
    pub credential_id: Option<Uuid>,  // None = use env / claude-code creds / no-key (ollama)
    pub host: Option<String>,         // ollama host override; None = default
    pub extras: serde_json::Value,    // provider-specific knobs
}
```

**`CredentialResolver` lookup order** (first match wins):

1. The engine's `credential_id` if set — explicit override always honoured.
2. Provider-specific env var: `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GOOGLE_API_KEY` / `GROQ_API_KEY` / `OPENROUTER_API_KEY`.
3. `~/.claude/.credentials.json` if the engine is Anthropic-flavoured — read the active subscription's API key. Same file shape Claude Code writes.
4. For Ollama: no key needed; the host URL is the config.
5. For `claude_cli`: the CLI handles its own auth — Kyma just spawns it.

**`SkillSpec`** — what a discovered skill looks like (matches Claude Code's frontmatter format):

```yaml
---
name: my-skill
description: when to use; the agent picks based on this
---
markdown body…
```

Local discovery walks `~/.claude/skills/`, `~/.claude/plugins/<plugin>/skills/`, and project-local `.claude/skills/` for these files. Remote upstreams clone a git repo with the same layout. Each enabled skill is injected as either (a) extra system-prompt context, or (b) a tool that returns the skill body when called (depending on size — short skills inline, long ones gate behind a tool call).

---

## 3. Phases

Each phase produces working software you can ship.

### Phase A1 — Engine providers + credential discovery + Settings UI

**Spans:** `docs/superpowers/plans/2026-05-28-a1-agent-engine-providers-and-settings.md`

**Ships:**

- `EngineConfig` persisted in catalog (new migration `engine_config` row, single global row for v1).
- `EngineRegistry` trait + 3 impls: `Anthropic`, `OpenAI`, `Ollama` (existing, refactored behind the trait).
- `CredentialResolver` — env vars, `~/.claude/.credentials.json`, catalog credentials.
- `GET /v1/agent/engines` (lists available providers + their default models) and `GET /PUT /v1/agent/engine` (current config).
- Web UI: a new **Settings → Agent Engine** card. Pick provider, pick model from provider's catalogue, pick credential (existing creds dropdown), test connection.
- Ask Kyma keeps working with whatever's configured. If nothing is configured, falls back to Ollama at `localhost:11434` (current behaviour) so the demo path still works.

**Done when:** user with `ANTHROPIC_API_KEY` exported (or Claude Code installed) opens Ask Kyma, the engine auto-resolves to Anthropic via that key, and a question gets a Claude Sonnet answer.

### Phase A2 — Claude Code CLI engine

**Spans:** `docs/superpowers/plans/2026-05-28-a2-claude-cli-engine.md` (to be written after A1 lands)

**Ships:**

- A new engine kind `claude_cli` that locates `claude` on the host's `PATH`, spawns it per turn with `--print --output-format stream-json`, pipes the conversation and tool descriptors as JSON, and reads streaming events back.
- Inherits everything Claude Code has on the host: MCPs, locally configured skills, project context, the user's API key (Kyma doesn't manage the key — Claude Code does).
- Honoured by the same `/v1/agent/ask` SSE handler — the CLI's stream-json events get translated into Kyma's existing `ToolCall` / `AnswerDelta` / `RunFinished` event types.
- UI: appears as a provider option in the engine dropdown with a "uses your local Claude Code" subtitle. Disabled if `claude` isn't on `PATH`.

**Done when:** picking "Claude Code CLI" in Settings → Engine and asking Kyma a question runs the local `claude` binary; tool calls show up in the SSE stream; cancelling the run kills the subprocess.

### Phase A3 — Local skill discovery + injection

**Spans:** `docs/superpowers/plans/2026-05-28-a3-skills-local.md`

**Ships:**

- `SkillRegistry` trait + `LocalSkillSource` impl that walks `~/.claude/skills/`, `~/.claude/plugins/*/skills/`, and the working-directory's `.claude/skills/`.
- `GET /v1/skills` (lists discovered skills with `{ name, description, source }`), `GET /PUT /v1/skills/enabled` (toggled subset).
- Enabled skills inject into the agent run as:
  - **Short skills** (`<2k tokens`) — appended verbatim to the system prompt.
  - **Long skills** — exposed as `get_skill(name)` tool calls so the agent can pull them on demand.
- UI: **Settings → Skills** page — a list of discovered skills with checkboxes + description previews.

**Done when:** dropping a skill file in `~/.claude/skills/foo.md` makes it appear in the Skills page; enabling it influences the agent's behaviour on the next question.

### Phase A4 — Remote skill upstreams (git/HTTPS)

**Spans:** `docs/superpowers/plans/2026-05-28-a4-skills-remote.md`

**Ships:**

- `RemoteSkillSource` — clone a git repo (https or ssh + deploy key) into Kyma's data dir, watch for upstream changes, periodically `git pull`.
- `POST /v1/skills/sources { kind: "git", url, credential_id? }` and `DELETE /v1/skills/sources/{id}` for registry management.
- UI: **Settings → Skills → Sources** — add a remote URL, see sync status, manually trigger a refresh.

**Done when:** the user adds `https://github.com/some/skills.git` as a source; its skill files appear in the Skills page; pulling the repo refreshes them.

### Phase A5 — Session persistence + loop hardening

**Spans:** `docs/superpowers/plans/2026-05-28-a5-agent-loop.md`

**Ships:**

- Sessions persist to Postgres via the existing `agent_runs` table + a new `agent_messages` table.
- Multi-turn conversations work: the SSE handler picks up the prior turn's context on a follow-up.
- Cancellation: a `DELETE /v1/agent/runs/{id}` aborts an in-flight run, propagates a CancellationToken into adk-rust, kills any subprocess.
- Retry on transient provider errors (5xx, rate-limit) with a capped exponential backoff.
- Better reasoning visibility: the SSE stream surfaces every tool call's elapsed time, every retry, every cancellation.

**Done when:** restarting `kyma-bin` mid-conversation doesn't lose the chat; pressing the cancel button mid-stream stops the run cleanly; rate-limit errors get retried instead of failing the run.

---

## 4. Out-of-scope or deferred

- **Per-user engine config** — v1 has one global engine. Multi-tenant per-user routing is in the cloud-track program (`2026-05-25-kyma-cloud-platform-design.md`).
- **Tool authoring UI** — users can't write new tools from the web app. Skills are the extensibility surface.
- **Embedding/vector store for skills** — we index by description text only (substring/keyword), not semantic similarity. Adding ranking is a small future change to `SkillRegistry::select_for_query`.
- **Skill permissioning** — every enabled skill is available to every run. Per-database / per-route skill scoping is a future enhancement.

---

## 5. Open questions

- **System prompt sprawl with many skills.** If the user enables 50 short skills, the system prompt balloons. Phase A3 mitigates with the inline/tool-gated split, but we'll need to measure and possibly raise the inline-skill threshold or change the dispatch heuristic.
- **Cancellation across the CLI engine.** Killing the `claude` subprocess works, but mid-tool-call cancellation needs the subprocess to flush partial output cleanly. A5 task notes this as a verify-before-merge item.
- **Remote skill auth.** Plan A4 supports ssh deploy keys via the existing `CredentialStore` (`Pat` variant works for HTTPS tokens; we may need a new `SshKey` variant for ssh).

---

## 6. Sequencing

A1 first (engine + creds + Settings UI) — everything else builds on it. A3 (local skills) before A4 (remote) because the local path proves the registry shape. A2 (Claude CLI) and A5 (loop hardening) can land in either order after A1.
