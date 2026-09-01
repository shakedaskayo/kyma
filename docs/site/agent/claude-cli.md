---
title: Claude Code CLI engine
description: Use your local Claude Code OAuth, MCPs, and skills with Pensieve — no API key required.
---

# Claude Code CLI engine

The `claude_cli` engine lets Pensieve reuse your local Claude Code installation: OAuth credentials, tool loop, MCPs, and skills all come along for free. No API key required. Use it for local development; use the `anthropic` engine (with an API key) for production or multi-tenant deployments.

## When to use it

| Situation | Engine |
| ---------------------------------------- | --------------------- |
| Local dev, Claude Code already installed | `claude_cli` |
| Production deployment | `anthropic` (API key) |
| Air-gapped / offline | `ollama` |
| Multi-tenant SaaS | `anthropic` (API key) |

## What gets inherited

When you select this engine, the server runs:

```bash
claude --print [--model M] "<the question, with system prompt context>"
```

That single invocation inherits **everything** from your local Claude Code install:

- **Auth.** OAuth token from the macOS Keychain. Refreshes itself.
- **Skills.** Every skill under `~/.claude/skills/` and
  `~/.claude/plugins/cache/<vendor>/<plugin>/<version>/skills/`. Pensieve
  does NOT inject skills via system-prompt for this engine — Claude Code
  already knows how.
- **MCPs.** Any MCP server configured in
  `~/.claude/claude_code_settings.json`. See also [MCP reference](/reference/mcp).
- **Tool loop.** Edit, Read, Bash, search — all the standard Claude
  Code tools.

This makes it the most capable engine for local development: "answer questions about my Pensieve deployment" can lean on the entire agent stack you already have.

## How it works

Modern Claude Code (≥ v2.1) stores its OAuth credentials in the macOS
Keychain — `Claude Code-credentials`, account = `$USER`. The token in
that entry is an OAuth access token (`sk-ant-oat01-…`), not a static
API key.

OAuth tokens **don't work** with the standard `x-api-key` header that
the `anthropic` engine sends. They require:

```
Authorization: Bearer sk-ant-oat01-…
anthropic-beta: oauth-2025-04-20
```

The CLI engine sidesteps the issue entirely: `claude --print` already
knows how to authenticate. Pensieve's only job is to spawn it with the
right prompt and pipe the result through `/v1/agent/ask` as
`answer_delta` SSE events.

## Models

| Model | Notes |
| ----------------------- | ----------------------------------------- |
| `default` | Claude Code picks the model (recommended) |
| `claude-opus-4-8` | Most capable; slower |
| `claude-sonnet-4-6` | Balanced |
| `claude-haiku-4-5` | Fastest |

> **Note:** `claude_cli` ships `claude-opus-4-8`; the `anthropic` API
> engine ships `claude-opus-4-7`. They are not interchangeable.

## Gotchas

- **`claude` must be on `$PATH`.** The server's locator checks
  `$PATH` plus a few well-known install paths
  (`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`).
- **The Keychain entry is host-specific.** A Pensieve server running in
  Docker on macOS does NOT inherit the host's Keychain. Use this engine
  only when the Pensieve server process can see your user's Keychain — i.e.
  native `cargo run` or a `launchd`-managed service running as your
  user.
- **One process per turn.** Each agent turn spawns a fresh `claude
  --print`. Startup latency adds a few hundred ms; over the duration
  of a typical multi-step answer, it's negligible.
- **The "Test connection" button** sends `claude --print "ping"` and
  expects any non-empty stdout within 30 s. If your Claude Code is
  mid-install or in onboarding, expect a timeout.
- **Can't replay history / record-only limitation.** Because each turn
  is a fresh `claude --print` invocation, there is no persistent
  conversation history across turns — context must be re-injected by
  the caller.

See [all engines](/agent/engines) for a side-by-side comparison.
