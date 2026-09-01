---
title: Your agent remembers across sessions
description: Give a coding agent durable, graph-aware memory so it stops forgetting your codebase between sessions — decisions, conventions, and gotchas recalled into every prompt.
---

# Your agent remembers across sessions

## The problem

Every new session, your coding agent starts from zero. You re-explain the auth model, the
naming conventions, the one service that's load-bearing-and-fragile, the decision you made
last week and *why*. The agent is brilliant and amnesiac — and the re-explaining tax
compounds across a team.

pensieve gives it durable memory: facts you (or the agent) save once, recalled into the right
prompts forever, across sessions and machines.

## Setup

One binary, zero infra, then wire your agent ([full options](/agent/connect)):

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash
pensieve setup claude-code        # MCP — or: pensieve install-plugin (automatic capture + recall)
```

## Save what's durable

Save a fact explicitly from the CLI, or let the agent call `save_memory` over MCP:

```bash
pensieve remember "payments-svc deploys behind the Aurora gateway; error budget is 0.1%."
pensieve remember "Prefer KQL over SQL in examples." --type preference
```

Memory is typed (fact, preference, decision, …) and **graph-aware**: a memory about
`payments-svc` links to the real service node, its repo, and the traces it emits.
Wrap anything sensitive in `<private>…</private>` and it's stripped before it's ever
embedded or stored.

## Recall it — automatically

Ask for it explicitly:

```bash
pensieve recall "how do we deploy payments and what's the error budget?"
# → returns the memory, scored by vector + keyword + graph
```

…but the point is you mostly *don't*. With the [Claude Code plugin](/agent/claude-code-plugin),
hooks inject the most relevant memories into **every** prompt with no tool call — so the
agent already knows before you ask. Recall is graph-aware hybrid retrieval (vector +
keyword, fused with Reciprocal Rank Fusion, then expanded over the memory graph), so it
returns the surrounding context, not just a string match.

## What you'll see

A new session, cold, asked "how should I add a field to the checkout flow?" — and the
agent answers *with your conventions baked in*: it knows checkout calls payments-svc, that
you prefer KQL examples, that the error budget is tight. No prologue, no re-explaining.

## Scale it

- **Across machines & teammates** — [sync](/concepts/sync) keeps your laptop and the team
  control plane coherent: memory written on one shows up on the others.
- **Kept sharp automatically** — [dreaming](/agent/dreaming) reviews memories on a schedule,
  merges duplicates, supersedes contradictions (bi-temporal — nothing is hard-deleted), and
  wires memories to the resources they're about.
- **From the agent's own files** — `pensieve sync` also ingests Claude Code's file memory
  (`~/.claude/projects/*/memory`) into queryable pensieve memory.

## Next

- The recall internals: [Agentic Memory](/agent/memory).
- The full loop in action: [Debug a prod incident from your editor](/use-cases/debug-from-your-editor).
- How it all fits: [How pensieve works](/concepts/how-pensieve-works).
