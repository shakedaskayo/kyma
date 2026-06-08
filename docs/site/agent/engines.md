---
title: Engines
description: Provider abstraction for the Kyma agent — Anthropic, OpenAI, Ollama, and the Claude Code CLI engine.
---

# Engines

Pick an engine, configure it, and the agent uses it for every `/v1/agent/ask` call.
There are four engines today. Each has a fixed model list and its own credential path.

## Quick reference

| Engine | Default model | Models | Credential |
|--------|--------------|--------|------------|
| `anthropic` | `claude-sonnet-4-6` | `claude-opus-4-7`, `claude-sonnet-4-6`, `claude-haiku-4-5` | `ANTHROPIC_API_KEY` or stored credential |
| `openai` | `gpt-5` | `gpt-5`, `gpt-5-mini`, `gpt-4.1`, `o4-mini` | `OPENAI_API_KEY` or stored credential |
| `ollama` | `gemma4:latest` | live-fetched from your Ollama; fallback list below | none |
| `claude_cli` | `default` | `claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5`, `default` | inherits Claude Code OAuth from macOS Keychain |

**Ollama fallback list** (shown when Ollama is unreachable): `gemma4:latest`, `llama4:latest`, `qwen3:latest`, `mistral:latest`.

**`claude_cli` vs `anthropic` model lists differ.** `claude_cli` ships `claude-opus-4-8`; the `anthropic` API engine ships `claude-opus-4-7`. They are not interchangeable.

## Set the active engine

### In the web app

`/settings#engine` — pick kind, model, and credential. **Test connection** sends a 1-token probe. Save commits the config.

For Ollama, the model picker live-fetches `${host}/api/tags`, so you see only models you actually have on disk.

### Over HTTP

```bash
# Read the current engine config.
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/v1/agent/engine

# Write a new config.
curl -X PUT \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  http://localhost:8080/v1/agent/engine \
  -d '{
    "kind": "anthropic",
    "model": "claude-sonnet-4-6",
    "credential_id": "<uuid from /v1/credentials>"
  }'

# Probe the config with a 1-token call.
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  http://localhost:8080/v1/agent/engine/test \
  -d '{ "kind": "anthropic", "model": "claude-sonnet-4-6", "credential_id": "…" }'
```

See [API reference](/reference/api) for full request/response shapes.

### Discover available engines and models

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/v1/agent/engines
```

Returns:

```json
{
  "available": [
    {
      "kind": "anthropic",
      "label": "Anthropic (Claude)",
      "models": ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
      "needs_key": true
    },
    {
      "kind": "openai",
      "label": "OpenAI",
      "models": ["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini"],
      "needs_key": true
    },
    {
      "kind": "ollama",
      "label": "Ollama (local)",
      "models": ["gemma4:latest", "llama4:latest"],
      "needs_key": false
    },
    {
      "kind": "claude_cli",
      "label": "Claude Code CLI",
      "models": ["claude-opus-4-8", "claude-sonnet-4-6", "claude-haiku-4-5", "default"],
      "needs_key": false
    }
  ],
  "active": { "kind": "ollama", "model": "gemma4:latest", "host": "http://localhost:11434" }
}
```

The Ollama branch live-fetches with a 2 s timeout. If Ollama is down, the endpoint falls back to the hardcoded list — you can still select a model; you'll get a clear error on the next `kyma query`.

## Credential resolution

For engines that need an API key, the server checks in order:

1. **Explicit `credential_id`** on the engine config — looked up in `/v1/credentials`, decrypted, used as-is.
2. **Environment variable** — `ANTHROPIC_API_KEY` for `anthropic`, `OPENAI_API_KEY` for `openai`.
3. **Claude Code Keychain** — for `anthropic`, the server reads the `Claude Code-credentials` Keychain entry on macOS. Only works with static API keys (`sk-ant-api03-…`). OAuth tokens (`sk-ant-oat01-…`) cannot flow through this path because the SDK sends them via `x-api-key`, not `Authorization: Bearer`. Use the `claude_cli` engine for OAuth tokens.
4. **No key needed** — `ollama` and `claude_cli`.

If nothing resolves, the engine returns a clear error in the streaming response.

## Adding a new provider

Single-file change in `crates/kyma-server/src/agent/engine/`:

1. Add a variant to `EngineKind`.
2. Add a new module (`mistral.rs`, `gemini.rs`, …) with `build(cfg, key) -> Result<Arc<dyn Llm>>` and `default_models()`.
3. Wire it into the `build_engine` match — the match is exhaustive, so the compiler flags every unhandled variant.

The Settings UI inherits the new engine automatically via `engine_catalogue()`.
