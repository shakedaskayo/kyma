---
title: Skills
description: Local skill discovery for the Pensieve agent. Drop a SKILL.md in a discovery root and toggle it on in Settings.
---

# Skills

Enable skills to teach Pensieve domain-specific tasks — drop a `SKILL.md` in a
discovery root and toggle it on. The agent picks up the new behaviour on the
next turn, with no restart needed.

A **skill** is a markdown document that tells the agent how to do
something specific: "when the user asks about X, do Y."

## Enabling skills

`/settings#skills` lists every discovered skill grouped by source
(project / user / plugin). Toggle a skill on; the next agent turn
includes its body. The selection is stored in `enabled_skills` and
persists across server restarts.

You can also drive the toggle over HTTP:

```bash
# List discovered skills (~50 on a developer machine that uses
# Claude Code; just a handful on a server).
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/v1/agent/skills

# Toggle skills on/off (the array is the full set of enabled names).
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  http://localhost:8080/v1/agent/skills/enabled \
  -d '{"skills":["pensieve","brainstorming"]}'
```

## Discovery roots

The server walks six locations, with symlinks followed. Each
candidate dir is scanned for `<skill>/SKILL.md`.

| Path                                              | Source       | Purpose                                                    |
| ------------------------------------------------- | ------------ | ---------------------------------------------------------- |
| `./.skills/`                                      | `project`    | Skills checked into the current Pensieve deployment.           |
| `~/.pensieve/skills/`                                 | `user`       | Skills shared across every Pensieve server you run.            |
| `~/.skills/`                                      | `user`       | Vendor-neutral skills (Cursor / Aider / etc. can share).   |
| `./.claude/skills/`                               | `project`    | Opt-in compatibility with Claude Code project skills.      |
| `~/.claude/skills/`                               | `user`       | Opt-in compatibility with Claude Code user skills.         |
| `~/.claude/plugins/cache/<vendor>/<plugin>/<version>/skills/` | `plugin` | Opt-in compatibility with Claude Code plugin skills.       |

The walker uses `std::fs::metadata` rather than `entry.file_type()` so
**symlinks are followed** — Claude Code installs many skills as
symlinks into bundles, and the naive walker would miss them on
macOS.

## Writing your own

The fastest path: copy `~/.pensieve/skills/pensieve/SKILL.md` (written by
`pensieve install-skill`) and adapt. The frontmatter `description` is the
discovery trigger — be specific about the phrases that should fire the
skill.

### SKILL.md format

```markdown
---
name: my-skill
description: Use when the user wants X. Triggered by phrases A, B, C.
---

# My Skill

Steps to take...

## When to use

- bullet
- bullet

## What NOT to do

...
```

The `name` and `description` are required. The body is appended
verbatim to the system prompt when the skill is enabled.

## Skill priority

When multiple skills could apply to a request, the agent follows
the priority order spelled out by [superpowers using-superpowers](https://github.com/claude-plugins-official/superpowers):

1. Process skills (brainstorming, debugging) first — these set HOW.
2. Domain skills (frontend-design, pensieve) second — these set WHAT.

Pensieve doesn't enforce this; the LLM does, based on each skill's
self-described trigger conditions.

## How skills land in the prompt

For the `anthropic` / `openai` / `ollama` engines:

The server calls `compose_system_prompt(state)` before each turn. It
fetches the enabled skills, reads each `SKILL.md` body, and appends a
section to the base system prompt:

```markdown
## Skill: brainstorming

**When to use:** Use this before any creative work...

<the rest of the SKILL.md body>
```

For the **Claude CLI engine**, this injection is skipped — Claude Code
loads its own skills natively. The agent has access to anything you
have installed in `~/.claude/skills/`.
