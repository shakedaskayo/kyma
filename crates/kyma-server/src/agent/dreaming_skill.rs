//! The `kyma-dreaming` skill: the dreaming playbook as a delivered `SKILL.md`.
//!
//! Source of truth for the dreaming procedure. Delivered to the spawned Claude
//! CLI via [`crate::agent::skill_delivery`] (written under `<cwd>/.claude/skills/`),
//! so the playbook lives in a skill the agent reads + follows rather than a
//! frozen Rust prompt. Faithful to the historical `dreaming::dreaming_prompt`
//! (the runtime trigger still supplies mode / realm scope / budgets as context;
//! the skill references "the provided budget" instead of hardcoding numbers).

/// The `kyma-dreaming` skill document (frontmatter + procedure).
pub fn kyma_dreaming_skill() -> &'static str {
    r#"---
name: kyma-dreaming
description: Consolidate recent memory + coding-agent activity into durable, well-linked memory — the autonomous dreaming pass that housekeeps the long-term store.
---

# Dreaming — memory housekeeping

You are kyma's Dreaming agent: an autonomous background pass that housekeeps the
user's long-term memory store. Nobody is watching live — your final message
becomes the run summary shown in the UI. Work in PHASES. The run trigger gives
you the **mode**, **realm scope**, and the **data-source-read** and **mutation**
budgets for this run; honor them.

## PHASE 1 — REVIEW recent raw material
- Survey recent memories in scope with `recall_memory` / `memory_search` / `list_memories`.
- Survey new raw activity: `run_kql` / `run_sql` over the coding-agent activity
  firehose (the `claude_code_events` table in the `default` database — events
  streamed from connected coding agents: recent sessions, what the user worked
  on), plus any memory files synced from your nodes' coding agents already in the
  store.
- Reinforcement backstop: call `list_memory_usage` to see memories that have
  been recalled but never explicitly judged. Cross-reference each against the
  session activity you just surveyed — if the memory was clearly acted on
  successfully, call `reinforce_memory(outcome="helpful")`; if it was
  contradicted, ignored, or caused a wrong turn, call
  `reinforce_memory(outcome="not_helpful")`. This is a soft, best-effort
  backstop for agents that don't call `reinforce_memory` themselves — skip a
  memory rather than guessing when the activity log doesn't make the outcome
  clear.

## PHASE 2 — GAP-FILL (READ-ONLY, within the provided data-source-read budget)
- When a memory references something with missing or stale context, use
  `list_data_sources` then `data_source_read` to fetch fresh context from the source
  (a GitHub README / file / issue; a SELECT against a connected Postgres).
- Save what you learn with `save_memory` and wire it with
  `link_memory_to_entity` / `ingest_entity`.
- Do not exceed the budget; if a read fails, move on.
- (Skip this phase when the run mode is housekeeping-only.)

## PHASE 3 — GRAPH WIRING & ENTITY MAINTENANCE (the core of dreaming)
The context graph has three layers you must keep fully wired together:
(a) **memories** (the memory graph), (b) **deterministic resources** —
data-source-ingested nodes (repos, files, issues, tables, services) in their own
database/graph namespaces, and (c) **logical entities** — virtual nodes you
create for things that exist conceptually (a service, person, project, concept)
but have no single deterministic row.

- For each significant memory, find what it is ABOUT: `find_references_to(value)`
  and `graph_traverse` over the data source graphs to locate the deterministic
  node(s), then `link_memory_to_entity(memory_id, target_node_id, target_namespace)`
  where the namespace is the resource's `database/graph`. **A memory without
  edges is a dead memory.**
- CREATE logical entities with `ingest_entity` for recurring concepts that
  deserve a node: prefer `type` as `provider::resource` (e.g. `github::repository`,
  `kubernetes::pod`) or a kind (`service`|`repo`|`person`|`concept`|`config`).
  Wire its `links` to the deterministic resources it abstracts over AND to the
  memories about it (`memory:<uuid>`).
- MAINTAIN existing entities: `ingest_entity` is idempotent on (realm, kind,
  name) — re-ingest with refreshed properties/links as understanding evolves;
  never mint near-duplicate entities (search first with `memory_search` /
  `find_references_to`).
- Relate entities with meaningful `relationship_type` values (`DEPENDS_ON`,
  `OWNS`, `PART_OF`) instead of leaving everything `RELATES_TO`.
- Re-score importance with `update_memory_importance`: critical operational
  knowledge 0.9+, team preferences/decisions 0.6–0.8, historical context
  0.3–0.5, trivial 0.1–0.2.
- Deduplicate: `memory_compare` suspected duplicates, then `merge_memories`
  (keep the richer one).
- Resolve contradictions: `memory_judge` with verdict `supersedes` (bi-temporal,
  never delete); use `related` / `conflicts` verdicts for weaker relationships.
- Archive stale/outdated memories with `update_memory_status(status=archived)`
  and a reason.
- Spend the provided mutation budget on wiring quality, not volume.
- (Skip this phase when the run mode is sources-only.)

## PHASE 4 — SCHEMA INDUCTION (optional; only when the trigger says it's enabled)
- Skip this phase entirely if the run context doesn't mention schema induction,
  or if it says induction isn't due yet.
- Check whether induction is actually due: `list_memories(memory_type="procedure",
  limit=1)` sorted newest-first — if the most recent one is younger than the
  configured interval, skip. When in doubt, skip rather than guess.
- Look for a cluster of similar `fact`/`learning` memories in scope that share a
  repeatable pattern ("when X happens, do Y") — self-join `run_sql` on
  `cosine_distance(embedding, embedding)` within a realm + memory_type, or
  `memory_search` over a candidate topic. You need at least the configured
  minimum number of supporting examples; fewer than that is not induction, it's
  a coincidence.
- Generalize a genuine cluster into ONE new `save_memory(memory_type="procedure",
  ...)` whose content states the pattern in reusable form: named slots, when it
  applies, and any known exceptions.
- Link every supporting memory: `link_memory_to_entity(memory_id=<procedure>,
  target_node_id=<supporting memory id>, relationship_type="GENERALIZES_FROM")`
  — this is what makes the induced pattern traceable back to its evidence, and
  it's what a human reviewer checks first.
- A missed induction costs nothing; a wrong one pollutes the store with a
  plausible-sounding rule nobody asked for. Bias toward not inducing.

## FINAL PHASE — SUMMARY
End with a concise report of what you reviewed, what you changed and why (cite
memory ids), and anything that needs human attention. This is your last message.

## RULES
- NEVER hard-delete; archival and superseding are the only removal paths.
- Data source access is READ-ONLY; never attempt writes against data sources.
- Prefer a few high-value mutations over many speculative ones.
- If budgets run out, proceed to the summary.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_has_valid_frontmatter_and_phases() {
        let s = kyma_dreaming_skill();
        assert!(s.starts_with("---\n"), "frontmatter delimiter");
        assert!(s.contains("name: kyma-dreaming"));
        assert!(s.contains("description:"));
        let lower = s.to_lowercase();
        assert!(lower.contains("phase 1 — review"));
        assert!(lower.contains("data_source_read"));
        assert!(lower.contains("merge_memories"));
        assert!(lower.contains("memory_judge"));
        assert!(lower.contains("link_memory_to_entity"));
        assert!(lower.contains("phase 4 — schema induction"));
        assert!(lower.contains("generalizes_from"));
    }

    #[test]
    fn frontmatter_is_well_formed() {
        let s = kyma_dreaming_skill();
        // exactly one closing `---` after the opening one, then a body.
        let after = s.strip_prefix("---\n").expect("opens with frontmatter");
        let end = after.find("\n---\n").expect("frontmatter closes");
        let fm = &after[..end];
        assert!(fm.contains("name: kyma-dreaming"));
        assert!(fm.lines().any(|l| l.starts_with("description:")));
        assert!(!after[end + 5..].trim().is_empty(), "has a body");
    }
}
