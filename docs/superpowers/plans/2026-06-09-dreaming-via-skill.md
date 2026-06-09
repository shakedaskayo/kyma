# Dreaming via the Kyma Skill — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. `- [ ]` checkboxes.

**Goal:** Deliver the dreaming playbook as the `kyma-dreaming` skill to the spawned Claude CLI via the filesystem (`<cwd>/.claude/skills/`), replacing the hardcoded `dreaming_prompt()`. No `claude_cli.rs` change (uses the existing `cwd` param). Agent-agnostic; safe fallback.

**Spec:** `docs/superpowers/specs/2026-06-09-dreaming-via-skill-design.md`.

**Co-working note:** the user is editing `claude_cli.rs` (binary-location). Piece 2 must NOT touch it. Never stage other uncommitted files.

---

### Task 1: `skill_delivery` module — materialize skills into a workdir

**Files:** `crates/kyma-server/src/agent/skill_delivery.rs` (new), `agent/mod.rs` (`pub mod`), maybe `Cargo.toml` (`tempfile`).

- [ ] **Step 1** — Confirm `tempfile` is a dep of `kyma-server` (`grep tempfile crates/kyma-server/Cargo.toml`; if absent, add it — check the root `[workspace.dependencies]` and use `tempfile.workspace = true` if present, else the version a sibling uses).
- [ ] **Step 2** — Failing test (`skill_delivery.rs` `#[cfg(test)]`):
  ```rust
  #[tokio::test]
  async fn writes_each_skill_under_claude_skills() {
      let skills = vec![
          SkillDoc { name: "kyma-dreaming".into(), body: "---\nname: kyma-dreaming\n---\nbody".into() },
          SkillDoc { name: "kyma-memory".into(), body: "x".into() },
      ];
      let d = deliver_to_workdir(&skills).await.unwrap();
      let p = d.workdir.path().join(".claude/skills/kyma-dreaming/SKILL.md");
      assert!(p.exists());
      assert!(std::fs::read_to_string(&p).unwrap().contains("name: kyma-dreaming"));
      assert!(d.workdir.path().join(".claude/skills/kyma-memory/SKILL.md").exists());
  }
  ```
  Run `cargo test -p kyma-server skill_delivery` → FAIL.
- [ ] **Step 3** — Implement:
  ```rust
  pub struct SkillDoc { pub name: String, pub body: String }
  pub struct DeliveredSkills { pub workdir: tempfile::TempDir }
  /// Write `<workdir>/.claude/skills/<name>/SKILL.md` for each skill. The
  /// returned workdir must outlive the spawned CLI (drop cleans it up).
  pub async fn deliver_to_workdir(skills: &[SkillDoc]) -> anyhow::Result<DeliveredSkills> {
      let workdir = tempfile::tempdir()?;
      for s in skills {
          let dir = workdir.path().join(".claude/skills").join(&s.name);
          tokio::fs::create_dir_all(&dir).await?;
          tokio::fs::write(dir.join("SKILL.md"), &s.body).await?;
      }
      Ok(DeliveredSkills { workdir })
  }
  ```
  Add `pub mod skill_delivery;` to `agent/mod.rs`.
- [ ] **Step 4** — `cargo test -p kyma-server skill_delivery` (pass) + build. Commit: `feat(agent): filesystem skill-delivery for the Claude CLI (workdir/.claude/skills)`.

---

### Task 2: the `kyma-dreaming` skill (embedded const + parse test)

**Files:** `crates/kyma-server/src/agent/dreaming_skill.rs` (new) + `agent/mod.rs`.

- [ ] **Step 1** — Read `dreaming.rs::dreaming_prompt` (~252-347) to capture the EXACT procedure (PHASE 1 review, PHASE 2 gap-fill with connector-read budget, PHASE 3 graph wiring / entity maintenance / conflict resolution) and which MCP tools each phase uses.
- [ ] **Step 2** — Failing test (`dreaming_skill.rs` `#[cfg(test)]`): `kyma_dreaming_skill()` returns a string whose frontmatter has `name: kyma-dreaming` and a `description:`, and whose body contains the three phase markers + "connector_read" budget rule. Assert it parses with the crate's skill frontmatter parser if one is reachable (`skills::` parse fn); else assert the `---`-delimited frontmatter + required substrings.
- [ ] **Step 3** — Implement: `pub fn kyma_dreaming_skill() -> &'static str` returning a `SKILL.md` const — developer voice, frontmatter (`name: kyma-dreaming`, one-line `description`), then the dreaming procedure rewritten as instructions to the agent ("Use `recall_memory`/`run_kql` to survey recent memories + `claude_code_events`… Use `connector_read` (READ-ONLY, budget N) to fill gaps… Rescore importance, merge duplicates, link entities via …"). Keep it faithful to `dreaming_prompt`'s phases. `pub mod dreaming_skill;` in `agent/mod.rs`.
- [ ] **Step 4** — `cargo test -p kyma-server dreaming_skill` + build. Commit: `feat(agent): kyma-dreaming skill (dreaming playbook as a delivered skill)`.

---

### Task 3: wire dreaming to deliver + invoke the skill (with fallback)

**Files:** `crates/kyma-server/src/agent/dreaming.rs` (`run_via_claude_cli` ~1022; keep `dreaming_prompt` as fallback).

- [ ] **Step 1** — Read `run_via_claude_cli` (~1022-1127): how it builds the prompt (calls `dreaming_prompt` ~655), the `mcp` config (~1038), the `run_stream_with_pid` call (~1054, current `cwd` arg), and how it reaches the enabled-skills store (mirror `runner.rs::compose_system_prompt`: `state.skills.get()` + `skills::discover_all()`).
- [ ] **Step 2** — Failing test (mirror existing dreaming tests that assert prompt assembly): a helper `gather_dreaming_skills(state) -> Vec<SkillDoc>` returns `kyma-dreaming` first + the enabled kyma skills (mock the store to enable one skill; assert both present). And: the thin trigger prompt builder references the `kyma-dreaming` skill + includes mode + the connector-read budget. Run → FAIL.
- [ ] **Step 3** — Implement in `run_via_claude_cli`:
  - `let skills = gather_dreaming_skills(state).await;` (kyma-dreaming const wrapped as `SkillDoc` + enabled skills filtered from `discover_all`).
  - `let delivered = match skill_delivery::deliver_to_workdir(&skills).await { Ok(d) => Some(d), Err(e) => { warn!(error=%e, "skill delivery failed; falling back to hardcoded dreaming prompt"); None } };`
  - Prompt: if `delivered.is_some()`, a THIN trigger (`dreaming_trigger_prompt(&mode, &settings)` — mode + budgets + realm-scope + "Follow the kyma-dreaming skill."); else the existing `dreaming_prompt(...)` (full, unchanged fallback).
  - `cwd`: `delivered.as_ref().map(|d| d.workdir.path())`.
  - Call `run_stream_with_pid(prompt, model, resume, cwd, mcp.as_ref())`; bind `delivered` in scope until the event loop finishes (don't drop early — the TempDir must live through the run).
  - Everything after (event consumption, trace, outcome) unchanged.
- [ ] **Step 4** — `cargo test -p kyma-server dreaming 2>&1 | tail -20` (new + existing dreaming tests pass) + `cargo build -p kyma-server`. Commit (only dreaming.rs + the new modules; NOT claude_cli.rs): `feat(agent): drive dreaming via the kyma-dreaming skill (filesystem delivery + fallback)`.

---

## Self-Review
- Filesystem skill delivery, no `claude_cli.rs` → Task 1 + Task 3 (existing `cwd`). ✓
- Dreaming playbook as a skill → Task 2; wired with fallback → Task 3. ✓
- Agent-agnostic (delivery keyed by engine; ADK path untouched) → per spec. ✓
- Safety fallback to `dreaming_prompt()` → Task 3 Step 3. ✓
- Co-working safety (no claude_cli.rs) → all tasks. ✓
