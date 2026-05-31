---
title: Engines
description: Provider abstraction for the Kyma agent — Anthropic, OpenAI, Ollama, and the Claude Code CLI engine.
---

# Engines

The agent's LLM backend is configurable per-deployment. There are four
engine kinds today; adding a fifth means adding one enum variant +
one `build()` function.

| Kind         | Models                                        | Auth                                          | Best for                                            |
| ------------ | --------------------------------------------- | --------------------------------------------- | --------------------------------------------------- |
| `anthropic`  | `claude-opus-4-7`, `claude-sonnet-4-6`, `claude-haiku-4-5`        | static API key (`sk-ant-api03-…`)             | Production. Best reasoning per dollar.              |
| `openai`     | `gpt-5`, `gpt-5-mini`, `gpt-4.1`, `o4-mini` | static API key (`sk-…`)                       | Production. Familiar.                               |
| `ollama`     | live-fetched from `${host}/api/tags`          | none                                          | Local-only / air-gapped. Use a tool-capable model.  |
| `claude_cli` | whatever Claude Code has access to             | inherits macOS Keychain OAuth from Claude Code | Local-dev. Reuses your existing Claude Code login. |

## Configure in the web app

`/settings#engine` shows a picker for kind + model + credential.
Live models populate from the server's `/v1/agent/engines` endpoint —
for Ollama, that's a real HTTP fetch against your local Ollama, so you
only see models you actually have on disk. **Test connection** runs a
1-token probe against the chosen engine. Save persists; the next
`/v1/agent/ask` uses the new config.

## Configure over HTTP

```bash
# Read the current config.
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/v1/agent/engine

# Write a new config.
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  http://localhost:8080/v1/agent/engine \
  -d '{
    "kind":"anthropic",
    "model":"claude-sonnet-4-6",
    "credential_id":"<uuid from /v1/credentials>"
  }'

# Probe the chosen config with a 1-token call.
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  http://localhost:8080/v1/agent/engine/test \
  -d '{ "kind":"anthropic", "model":"claude-sonnet-4-6", "credential_id":"…" }'
```

## Credential resolution

When the engine needs an API key, the server runs through a fixed
order:

1. **Explicit `credential_id`** on the engine config → looked up in
   `/v1/credentials`, decrypted, used as-is.
2. **Env var** — `ANTHROPIC_API_KEY` for `anthropic`, `OPENAI_API_KEY`
   for `openai`. Used if no `credential_id` was set.
3. **Claude Code Keychain** — for `anthropic`, the server reads the
   `Claude Code-credentials` Keychain entry on macOS. **Only works
   with static API keys.** OAuth tokens
   (`sk-ant-oat01-…`) cannot flow through this path because adk-rust
   sends them with `x-api-key`, not `Authorization: Bearer`. For OAuth
   tokens, use the `claude_cli` engine instead.
4. **No key needed** for `ollama` and `claude_cli`.

If nothing resolves, the engine returns a clear error in the
streaming response.

## Adding a new provider

Single-file change in `crates/kyma-server/src/agent/engine/`:

1. Add a variant to `EngineKind`.
2. Add a `build(cfg, key) -> Result<Arc<dyn Llm>>` and a
   `default_models()` to a new module (`mistral.rs`, `gemini.rs`, …).
3. Wire it in the `build_engine` match — the match is exhaustive, so
   the compiler forces you to handle the new variant everywhere.

The Settings UI inherits the new kind automatically via
`engine_catalogue()`.

## Catalogue and model discovery

`GET /v1/agent/engines` returns:

```json
{
  "available": [
    {"kind": "anthropic", "label": "Anthropic (Claude)",
     "models": ["claude-opus-4-7", "claude-sonnet-4-6", "claude-haiku-4-5"],
     "needs_key": true},
    {"kind": "ollama", "label": "Ollama (local)",
     "models": ["gemma4:latest", "llama4:latest", "qwen3:latest"],
     "needs_key": false},
    ...
  ],
  "active": { "kind": "ollama", "model": "gemma4:latest", "host": "http://localhost:11434" }
}
```

The Ollama branch live-fetches with a 2s timeout. If your Ollama is
down, the picker falls back to a small hardcoded default list — pick
one anyway and you'll get a clear error on the next `kyma query`.
