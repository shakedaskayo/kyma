---
title: Connect a coding agent
description: Install the `kyma` CLI + the Kyma skill so Claude Code, Cursor, Aider, or any other coding agent can query your production data in real time.
---

# Connect a coding agent

Any coding agent that can shell out to a binary can use Kyma as its
window into production data. The recipe is the same in every tool:

1. Install the `kyma` CLI.
2. Connect it to your running server.
3. Install the Kyma skill.

After that, "ask Kyma what's broken in prod right now" works inside
Claude Code, Cursor, Aider, Continue, or any other coding assistant.

The web app at `/settings#connect` walks you through the same flow
with copy-paste snippets pre-filled with your server URL and a freshly
issued token. Use the panel from a logged-in browser session if you
don't want to copy a token by hand.

## 1 — Install

```bash
# From a checkout of the kyma repo:
cargo install --path crates/kyma-cli

# Verify:
kyma version
```

Requires Rust ≥ 1.74. The binary is named `kyma`, not `kyma-cli`.

## 2 — Connect

```bash
kyma connect http://localhost:8080 --token "<bearer>"

kyma status
# Endpoint:  http://localhost:8080
# Token:     configured
# Health:    {"status":"ok","version":"0.0.1"}
```

The token is the access token from `POST /v1/auth/login`. In the web
app, `/settings#connect` shows it pre-filled with a click-to-reveal.

Config persists at `~/.kyma/config.json` (mode `0600`). Two env vars
override the file if set:

| Var               | Wins over file?              |
| ----------------- | ---------------------------- |
| `KYMA_SERVER_URL` | yes — replaces endpoint      |
| `KYMA_TOKEN`      | yes — replaces saved token   |

## 3 — Install the skill

```bash
kyma install-skill --also-link-claude
# Wrote /Users/you/.kyma/skills/kyma/SKILL.md
# Linked /Users/you/.claude/skills/kyma -> /Users/you/.kyma/skills/kyma
```

The skill is a `SKILL.md` with frontmatter that tells the agent
*when* to use the CLI. The body explains *how* (the `kyma query`
syntax + examples).

`--also-link-claude` symlinks the directory into `~/.claude/skills/`
so Claude Code picks it up. For other tools the install paths differ:

| Tool        | Drop the file here                                                   |
| ----------- | -------------------------------------------------------------------- |
| Claude Code | `~/.claude/skills/kyma/SKILL.md` (or symlink, as above)              |
| Cursor      | `~/.cursor/rules/kyma.md` (cursor uses rules-format, slight adapt)   |
| Aider       | `~/.aider/system_prompt_extras.md` (append the body)                 |
| Continue    | `~/.continue/skills/kyma.md`                                         |

For tools without first-class skill support, just point them at
`~/.kyma/skills/kyma/SKILL.md` as a system-prompt extra.

## 4 — Try it

In Claude Code:

```
> ask Kyma what tables we have in the github database
```

Claude Code reads the skill, decides it applies, and runs
`kyma query "what tables are in the github database?"`. The CLI
streams the answer to stdout and Claude Code displays it.

You can also call `kyma query` directly:

```bash
kyma query "how many rows in github_nodes?"
kyma query "show me the top 10 contributors across all repos by commits"
kyma query "any error logs from prod-api in the last 15 minutes?"
```

## How it works under the hood

1. The skill tells the coding agent: "For data questions, shell out to
   `kyma query`."
2. `kyma query` POSTs to `/v1/agent/ask` with the question.
3. The Kyma server runs its own agent loop (with engine + tools +
   enabled skills) and streams `answer_delta` SSE events.
4. The CLI prints the deltas to stdout.
5. The coding agent captures stdout and threads it into its own
   conversation.

The chain is two levels of agent. The outer one (your coding tool) is
good at understanding your *intent*. The inner one (Kyma's agent) is
good at writing KQL. Skills are how they hand off.
