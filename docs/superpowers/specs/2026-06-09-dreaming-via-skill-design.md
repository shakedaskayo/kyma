# Dreaming via the Pensieve Skill — Design

**Date:** 2026-06-09
**Status:** Approved (autonomous — shared-substrate program, piece 2)
**Scope:** Deliver the dreaming procedure as a **pensieve skill** to the spawned Claude CLI (filesystem skill-delivery), replacing the hardcoded `dreaming_prompt()` playbook. The skill is the playbook; MCP tools stay the actuators. Agent-agnostic.

## Program context
Piece 2 of the shared-substrate program (4→1→3→2). Pieces 4, 1, 3 landed. The user's framing: "Dreaming via the pensieve skill instead of MCP tool-calling (needs Claude-CLI skill-delivery design)."

## Problem
Dreaming spawns the Claude CLI with a long hardcoded `dreaming_prompt()` (`dreaming.rs:252`) as the question + the pensieve MCP server, and **delivers zero skills** (`run_via_claude_cli`, `dreaming.rs:1022`). Only ADK agents get skills (system-prompt injection via `runner.rs::compose_system_prompt`); the Claude-CLI path has no skill delivery. The dreaming playbook is therefore frozen in Rust, can't be iterated like a skill, and doesn't compose with the user's other skills.

## Key enabler (no `claude_cli.rs` change)
`claude_cli::run_stream_with_pid(question, model, resume, cwd, mcp)` already takes `cwd` (sets `cmd.current_dir`). The spawned Claude Code **natively discovers `<cwd>/.claude/skills/`** (the file's own doc: "Code already owns its own … skills"). So delivering a skill = writing `SKILL.md` under a workdir + passing that workdir as `cwd`. No change to `claude_cli.rs` (which the user is concurrently editing for binary-location robustness — complementary, since dreaming needs to find the `claude` binary).

## Decisions (locked)
| Decision | Choice |
|---|---|
| Skill-delivery mechanism (Claude CLI) | **Filesystem.** Write each delivered skill to `<workdir>/.claude/skills/<name>/SKILL.md`; pass `cwd=<workdir>` to `run_stream_with_pid`. Native discovery + progressive disclosure; no prompt bloat; no `claude_cli.rs` edit. |
| Agent-agnostic | A `SkillDelivery` strategy keyed by engine kind: `ClaudeCli` → filesystem (this piece); `ADK` → existing system-prompt injection (`compose_system_prompt`, unchanged). The dreaming procedure is not hardcoded to Claude. |
| Dreaming playbook | Moves from the Rust `dreaming_prompt()` string into a **`pensieve-dreaming` SKILL.md** (the PHASE 1 review / PHASE 2 gap-fill / PHASE 3 graph-wiring procedure). The Rust prompt becomes a thin trigger: "Run a memory-consolidation (dreaming) pass; follow the `pensieve-dreaming` skill." Budgets/mode/realm-scope still injected as short structured context. |
| Skills composed | The dreaming run delivers `pensieve-dreaming` + the tenant's **enabled** pensieve skills (so dreaming benefits from the same skills agents use). Reuse `skills::discover_all` + the enabled-set store. |
| MCP | Unchanged — the pensieve MCP server is still wired (strict) as the actuator; the skill tells the agent which MCP tools to use. "Via the skill instead of MCP tool-calling" = the *playbook* comes from a skill, not a hardcoded prompt; tools remain. |
| Backward-compat / safety | If the workdir/skill write fails, fall back to the current hardcoded `dreaming_prompt()` (logged) — dreaming never breaks because skill delivery hiccupped. Existing dreaming outcome metrics + trace unchanged. |

## Architecture

### 1. Skill-delivery module — `crates/pensieve-server/src/agent/skill_delivery.rs` (new)
```rust
/// Materialize skills into a workdir the Claude CLI will discover.
pub struct DeliveredSkills { pub workdir: TempDir, /* kept alive for the run */ }

/// Write `<workdir>/.claude/skills/<name>/SKILL.md` for each skill, return the
/// workdir to pass as `cwd`. Agent-agnostic entry; ClaudeCli uses this path.
pub async fn deliver_to_workdir(skills: &[SkillDoc]) -> anyhow::Result<DeliveredSkills>;
```
- `SkillDoc { name, body }` — from `skills::discover_all()` filtered to the enabled set, plus the built-in `pensieve-dreaming`.
- Uses a `TempDir` (e.g. `tempfile`) so the workdir is cleaned after the run. The `DeliveredSkills` (holding the `TempDir`) must outlive the CLI process.

### 2. The `pensieve-dreaming` skill
- A `SKILL.md` (frontmatter `name: pensieve-dreaming`, `description: …`) whose body is the dreaming procedure currently in `dreaming_prompt()` — the three phases, the read-only/connector-budget rules, the graph-wiring + entity-maintenance + conflict-resolution steps, and which MCP tools to use for each. Authored in the developer/operator voice.
- Source: a bundled file under `integrations/claude-code/pensieve-memory/skills/pensieve-dreaming/SKILL.md` (so it ships with the pensieve plugin) AND/OR an embedded `const` the server can write even when the plugin isn't installed. Decision: **embedded `const` string** in the dreaming module is the source of truth (always available to the server); the bundled file is generated/kept in sync for users who browse skills. (v1: embedded const only; bundled copy is a follow-up.)

### 3. Wire dreaming — `dreaming.rs::run_via_claude_cli` (~1022)
- Gather skills: `pensieve-dreaming` (the const) + enabled pensieve skills (`discover_all` ∩ enabled set, via the same store ADK uses).
- `let delivered = skill_delivery::deliver_to_workdir(&skills).await` — on `Err`, log + fall back to the existing hardcoded-prompt path (no workdir, current behavior).
- Build a **thin** trigger prompt (mode + budgets + realm-scope as short structured context) that says to follow the `pensieve-dreaming` skill.
- Call `run_stream_with_pid(thin_prompt, model, resume, Some(delivered.workdir.path()), mcp)` — same call, now with `cwd`. Hold `delivered` until the stream ends.
- Everything downstream (event consumption, tool tallies, trace, outcome extraction) is unchanged.

### 4. Keep `dreaming_prompt()` as the fallback
- Don't delete `dreaming_prompt()`; it's the safety fallback when skill delivery fails. Add a short note that the skill is the primary path.

## Error handling
- Workdir create / skill write fails → fall back to hardcoded prompt (warn). Never abort the dreaming run for a delivery error.
- A malformed/oversized enabled skill → skip that skill (warn), still deliver `pensieve-dreaming`.
- `claude` binary not found → unchanged (the user's `locate_binary` WIP improves this independently).

## Testing
- **skill_delivery unit:** `deliver_to_workdir([{name:"pensieve-dreaming",body}])` writes `<workdir>/.claude/skills/pensieve-dreaming/SKILL.md` with the body; multiple skills each land in their own dir; the `TempDir` exists until dropped.
- **dreaming wiring unit:** the skill set passed to delivery includes `pensieve-dreaming` + the enabled skills (mock the enabled-set store); the thin prompt references the skill and carries the budgets/mode; on a forced delivery error, the code path selects the hardcoded `dreaming_prompt()` fallback. (Don't spawn a real `claude` — assert the assembled inputs, mirroring how existing dreaming tests assert the prompt.)
- **skill content:** `pensieve-dreaming` SKILL.md parses (valid frontmatter `name`/`description`) and contains the three phases + the connector-read-budget rule.
- Existing dreaming tests still pass.

## Out of scope / deferred
- A bundled (on-disk) copy of `pensieve-dreaming` in the plugin (v1 uses the embedded const).
- Filesystem skill delivery for the non-dreaming `/v1/agent/ask` Claude-CLI path (this piece is dreaming-scoped; the same module can extend there later).
- Remote skill registry (that's the A-series A4).

## File touch list
- `crates/pensieve-server/src/agent/skill_delivery.rs` (new) + `agent/mod.rs` (`pub mod`).
- `crates/pensieve-server/src/agent/dreaming.rs` (gather + deliver + thin prompt + `cwd`; keep `dreaming_prompt` as fallback). Possibly a new `dreaming_skill.rs` for the embedded `pensieve-dreaming` const.
- (Dep) `tempfile` in `pensieve-server/Cargo.toml` if not already present.
- **NOT** `claude_cli.rs` — uses the existing `cwd` param (avoids the user's concurrent edits).
