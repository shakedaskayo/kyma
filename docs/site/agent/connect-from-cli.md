---
title: Connect a coding agent
description: Install the `pensieve` CLI + the Pensieve skill so Claude Code, Cursor, Aider, or any other coding agent can query your production data in real time.
---

# Connect a coding agent

Any coding agent that can shell out to a binary can use Pensieve as its
window into production data. The recipe is the same in every tool:

1. Install the `pensieve` CLI.
2. Connect it to your running server.
3. Install the Pensieve skill.

After that, "ask Pensieve what's broken in prod right now" works inside
Claude Code, Cursor, Aider, Continue, or any other coding assistant.

The web app at `/settings#connect` walks you through the same flow
with copy-paste snippets pre-filled with your server URL and a freshly
issued token. Use the panel from a logged-in browser session if you
don't want to copy a token by hand.

## 1 — Install

```bash
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash

# Verify:
pensieve version
```

The installer downloads the prebuilt binary to `~/.local/bin` (or `/usr/local/bin`).
Binary name is `pensieve` (not `pensieve-cli`).

From source (contributors): `cargo install --path crates/pensieve-cli` inside a checkout,
or `… | bash -s -- --from-source`.

## 2 — Connect

```bash
pensieve connect http://localhost:8080 --token "<bearer>"

pensieve status
# Endpoint:  http://localhost:8080
# Token:     configured
# Health:    {"status":"ok","version":"0.0.1"}
```

The token is the access token from `POST /v1/auth/login`. In the web
app, `/settings#connect` shows it pre-filled with a click-to-reveal.

Config persists at `~/.pensieve/config.json` (mode `0600`). Two env vars
override the file if set:

| Var               | Wins over file?              |
| ----------------- | ---------------------------- |
| `PENSIEVE_SERVER_URL` | yes — replaces endpoint      |
| `PENSIEVE_TOKEN`      | yes — replaces saved token   |

## 3 — Install the skill

```bash
pensieve install-skill --also-link-claude
# Wrote /Users/you/.pensieve/skills/pensieve/SKILL.md
# Linked /Users/you/.claude/skills/pensieve -> /Users/you/.pensieve/skills/pensieve
```

The skill is a `SKILL.md` with frontmatter that tells the agent
*when* to use the CLI. The body explains *how* (the `pensieve query`
syntax + examples).

`--also-link-claude` symlinks the directory into `~/.claude/skills/`
so Claude Code picks it up. For other tools the install paths differ:

| Tool        | Drop the file here                                                   |
| ----------- | -------------------------------------------------------------------- |
| Claude Code | `~/.claude/skills/pensieve/SKILL.md` (or symlink, as above)              |
| Cursor      | `~/.cursor/rules/pensieve.md` (cursor uses rules-format, slight adapt)   |
| Aider       | `~/.aider/system_prompt_extras.md` (append the body)                 |
| Continue    | `~/.continue/skills/pensieve.md`                                         |

For tools without first-class skill support, just point them at
`~/.pensieve/skills/pensieve/SKILL.md` as a system-prompt extra.

## 4 — Try it

In Claude Code:

```
> ask Pensieve what tables we have in the github database
```

Claude Code reads the skill, decides it applies, and runs
`pensieve query "what tables are in the github database?"`. The CLI
streams the answer to stdout and Claude Code displays it.

You can also call `pensieve query` directly:

```bash
pensieve query "how many rows in github_nodes?"
pensieve query "show me the top 10 contributors across all repos by commits"
pensieve query "any error logs from prod-api in the last 15 minutes?"
```

## How it works under the hood

1. The skill tells the coding agent: "For data questions, shell out to
   `pensieve query`."
2. `pensieve query` POSTs to `/v1/agent/ask` with the question.
3. The Pensieve server runs its own agent loop (with engine + tools +
   enabled skills) and streams `answer_delta` SSE events.
4. The CLI prints the deltas to stdout.
5. The coding agent captures stdout and threads it into its own
   conversation.

The chain is two levels of agent. The outer one (your coding tool) is
good at understanding your *intent*. The inner one (Pensieve's agent) is
good at writing KQL. Skills are how they hand off.
