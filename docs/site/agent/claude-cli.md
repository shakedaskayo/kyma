---
title: Claude Code CLI engine
description: How the `claude_cli` engine inherits your local Claude Code OAuth, tool loop, MCPs, and skills — without going through adk-rust.
---

# The Claude Code CLI engine

`EngineKind::ClaudeCli` is the odd one out. Instead of routing through
adk-rust's `Llm` trait, it shells out to the local `claude` binary per
turn and pipes stdout straight into the SSE response.

## Why it exists

Modern Claude Code (≥ v2.1) stores its OAuth credentials in the macOS
Keychain — `Claude Code-credentials`, account = `$USER`. The token in
that entry is an OAuth access token (`sk-ant-oat01-…`), not a static
API key.

OAuth tokens **don't work** with the standard `x-api-key` header that
adk-rust's `AnthropicClient` sends. They require:

```
Authorization: Bearer sk-ant-oat01-…
anthropic-beta: oauth-2025-04-20
```

So OAuth-backed tokens can't flow through the `anthropic` engine.

The CLI engine sidesteps the issue: `claude --print` already knows how
to authenticate, run the tool loop, load MCPs, apply skills, and stream
output. Kyma's only job is to spawn it with the right prompt and pipe
the result through `/v1/agent/ask` as `answer_delta` SSE events.

## What gets inherited

When you select the Claude CLI engine, the server runs:

```bash
claude --print [--model M] "<the question, with system prompt context>"
```

That single invocation inherits **everything** from the user's local
Claude Code install:

- **Auth.** OAuth token from Keychain. Refreshes itself.
- **Skills.** Every skill under `~/.claude/skills/` and
  `~/.claude/plugins/cache/<vendor>/<plugin>/<version>/skills/`. The
  Kyma server does NOT inject skills via system-prompt for this engine
  — Claude Code already knows how.
- **MCPs.** Any MCP server configured in
  `~/.claude/claude_code_settings.json`.
- **Tool loop.** Edit, Read, Bash, search — all the standard Claude
  Code tools.

This is the most powerful engine for local development, because
"answer questions about my Kyma deployment" can lean on the entire
agent stack the user already has.

## When to use it

| Situation                                | Engine                |
| ---------------------------------------- | --------------------- |
| Local dev, Claude Code already installed | `claude_cli`          |
| Production deployment                    | `anthropic` (API key) |
| Air-gapped / offline                     | `ollama`              |
| Multi-tenant SaaS                        | `anthropic` (API key) |

## Gotchas

- **`claude` must be on `$PATH`.** The server's locator looks at
  `$PATH` plus a few well-known install paths
  (`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`).
- **The Keychain entry is host-specific.** A Kyma server running in
  Docker on macOS does NOT inherit the host's Keychain. Use this
  engine only when the Kyma server process can see your user's
  Keychain — i.e. native `cargo run` or a `launchd`-managed service
  running as your user.
- **One process per turn.** Each agent turn spawns a fresh `claude
  --print`. Startup latency adds a few hundred ms; over the duration
  of a typical multi-step answer, it's negligible.
- **The "Test connection" button** sends `claude --print "ping"` and
  expects any non-empty stdout within 30s. If your Claude Code is mid-
  install or in onboarding, expect a timeout.
