---
title: Claude Code memory plugin
description: A Claude Code plugin that turns your coding sessions into a realtime, KQL/SQL-queryable memory layer in Kyma — with auto-recall, durable memory distillation, and a bundled MCP server.
---

# The kyma-memory Claude Code plugin

`kyma install-plugin` installs a proper [Claude Code plugin](https://docs.claude.com/en/docs/claude-code/plugins)
that makes Claude Code a live source **and** consumer of your Kyma
[Agentic Memory](/concepts/the-agent-loop) layer. It lives in-repo at
`integrations/claude-code/kyma-memory/`.

This is distinct from the [Claude Code engine](/agent/claude-cli) (Kyma using
`claude` as an LLM). Here it's the other direction: **Claude Code using Kyma** as
its memory and data backend.

## What it does

Two lanes, one connection (it reuses your `kyma connect` credentials — the plugin
stores none of its own):

| Lane | Mechanism | Result |
| --- | --- | --- |
| **Firehose** | hooks → `kyma ingest push` → `POST /v1/ingest` | Every meaningful turn (prompts, tool actions, assistant replies) lands in `default.claude_code_events`, queryable in KQL/SQL and visible in Discover/graph — in realtime. |
| **Recall** | `UserPromptSubmit` hook → `kyma recall` (MCP `recall_memory`) | Relevant durable memories are injected as context on each prompt. |
| **Distill** | `SessionEnd` hook + `/kyma-remember` → `kyma distill` → `/v1/agent/ask` | The kyma agent extracts durable memories (decisions, preferences, learnings) at session end. |
| **Direct tools** | bundled MCP server → `/mcp/v1` | The model can call `recall_memory`, `save_memory`, `run_kql`, `run_sql`, and the other Kyma tools natively. |

## Install

```bash
kyma connect https://your-kyma.example.com --token <token>
kyma install-plugin            # → ~/.claude/skills/kyma-memory
```

Restart Claude Code and run `/kyma-status`. `install-plugin` templates your server
URL + token into the plugin's `.mcp.json` so both the hooks and the MCP server work
out of the box.

## Hooks

The plugin registers five lifecycle hooks (all fail open — capture never blocks a
session):

| Event | Action |
| --- | --- |
| `SessionStart` | firehose a session marker + inject a memory recap |
| `UserPromptSubmit` | firehose the prompt + inject recalled context |
| `PostToolUse` | firehose tool events (`Edit`/`Write`/`Bash`/`Task`/`WebFetch`/`NotebookEdit`) |
| `Stop` | firehose the assistant's final message |
| `SessionEnd` | distill the session into durable memories (detached) |

## Slash commands

- `/kyma-recall <query>` — semantic recall from durable memory.
- `/kyma-remember [note]` — save a memory, or distill the recent conversation.
- `/kyma-ask <question>` — ask Kyma about your data (logs/traces/tables/graph) in KQL/SQL.
- `/kyma-status` — connection, capture mode, and this project's event breakdown.

## The capture table

`default.claude_code_events` (auto-created, schema-evolving):

```kql
claude_code_events
| where realm == "kyma"
| summarize events = count() by kind
| order by events desc
```

`kind` is one of `session_start` · `user_prompt` · `tool_use` · `assistant_turn` ·
`session_end`. `realm` defaults to the project directory's basename.

## Configuration & privacy

Behavior is controlled by environment variables (`KYMA_CC_CAPTURE` =
`off`/`metadata`/`full`, `KYMA_CC_AUTO_RECALL`, `KYMA_CC_DISTILL`, `KYMA_CC_REALM`,
…). In `full` mode conversation content is sent to **your own** Kyma server; common
secret shapes are redacted before egress, and `metadata` mode captures event shape
without bodies. See the plugin's `README.md` for the full table.

## Uninstall

```bash
rm -rf ~/.claude/skills/kyma-memory
```
