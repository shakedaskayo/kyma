---
title: The Kyma Agent
description: A swappable LLM agent that runs over your live Kyma deployment — from the web app or any coding tool that can shell out to `kyma query`.
---

# The Kyma Agent

Kyma ships with a built-in **agent** that turns natural-language
questions into KQL/SQL queries over your live data. It runs inside the
web app at `/agent` and is also reachable from your terminal via the
`kyma` CLI — which means any coding tool that can shell out (Claude
Code, Cursor, Aider, Continue, …) can query your Kyma deployment in
real time.

## Three things to know

1. **The provider is swappable.** Pick Anthropic, OpenAI, Ollama, or
   "Claude Code CLI" (which inherits your local OAuth and tool loop) in
   `/settings#engine`. Each provider lists its installed models live —
   for Ollama, the picker calls `${host}/api/tags` and shows what's
   actually on disk.

2. **Skills are first-class.** Drop a `SKILL.md` into one of the
   discovery roots and it shows up under
   `/settings#skills`, ready to enable. Enabled skills are injected into
   the system prompt for every agent turn (or, for the Claude CLI
   engine, picked up natively by Claude Code).

3. **The CLI is the integration surface.** `kyma query "…"` streams an
   answer to stdout. `kyma install-skill` writes a `SKILL.md` that
   teaches any coding agent to use this CLI as its window into your
   production data.

## Quick tour

```bash
# 1. Connect this terminal to a running Kyma server.
kyma connect http://localhost:8080 --token <bearer>

# 2. Ask a question — streams to stdout.
kyma query "what databases do we have, and how big is each one?"

# 3. Make Claude Code aware of your Kyma server.
kyma install-skill --also-link-claude
# Now in any Claude Code session, "ask Kyma what's slow today" works.
```

## In this section

- **[Engines](./engines)** — provider abstraction, model picker, credential
  resolution order, the `/v1/agent/engine` API.
- **[Claude Code engine](./claude-cli)** — how the Claude CLI engine
  inherits Keychain OAuth, MCPs, and skills; when to use it.
- **[Skills](./skills)** — local discovery (`./.skills/`, `~/.kyma/skills/`,
  `~/.skills/`, plus opt-in Claude paths), the `SKILL.md` format, and
  how enabled skills are injected.
- **[Connect from CLI](./connect-from-cli)** — install `kyma`, install the
  Kyma skill, and wire it into Claude Code / Cursor / Aider.

## How does this relate to `/v1/agent/ask`?

`/v1/agent/ask` is the wire-level streaming HTTP endpoint. Everything
above is built on top of it:

- The web `/agent` page is a React client of `/v1/agent/ask`.
- `kyma query` is a CLI client of `/v1/agent/ask`.
- The installed skill teaches *other* coding agents to shell out to
  `kyma query`, which is a client of `/v1/agent/ask`.

If you want to wire your own client, see
[Query → Agent endpoint](/query/agent-endpoint) for the SSE schema.
