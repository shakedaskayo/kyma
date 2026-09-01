---
title: Claude Code memory plugin
description: A Claude Code plugin that turns your coding sessions into a realtime, KQL/SQL-queryable memory layer in Pensieve — with auto-recall, durable memory distillation, and a bundled MCP server.
---

# The pensieve-memory Claude Code plugin

Install the plugin to turn Claude Code into a live source **and** consumer of Pensieve's
memory layer — it captures your session in realtime, auto-recalls relevant memories on
each prompt, and distills learnings into durable memory at session end.

```bash
pensieve connect https://your-pensieve.example.com --token <token>
pensieve install-plugin            # → ~/.claude/skills/pensieve-memory
```

Restart Claude Code and run `/pensieve-status`. `install-plugin` templates your server
URL + token into the plugin's `.mcp.json` so both the hooks and the MCP server work
out of the box. It lives in-repo at `integrations/claude-code/pensieve-memory/`.

> **Note — engine vs. plugin:** This is distinct from the [Claude Code engine](/agent/claude-cli)
> (Pensieve using `claude` as an LLM backend). Here it's the other direction: **Claude Code using Pensieve**
> as its memory and data backend.

## What it does

Two lanes, one connection (it reuses your `pensieve connect` credentials — the plugin
stores none of its own):

| Lane | Mechanism | Result |
| --- | --- | --- |
| **Firehose** | hooks → `pensieve ingest push` → `POST /v1/ingest` | Every meaningful turn (prompts, tool actions, assistant replies) lands in `default.claude_code_events`, queryable in KQL/SQL and visible in Discover/graph — in realtime. |
| **Recall** | `UserPromptSubmit` hook → `pensieve recall` (MCP `recall_memory`) | Relevant durable memories are injected as context on each prompt. |
| **Distill** | `SessionEnd` hook + `/pensieve-remember` → `pensieve distill` → `/v1/agent/ask` | The pensieve agent extracts durable memories (decisions, preferences, learnings) at session end. |
| **Direct tools** | bundled MCP server → `/mcp/v1` | The model can call `recall_memory`, `save_memory`, `run_kql`, `run_sql`, and the other Pensieve tools natively. |

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

- `/pensieve-recall <query>` — semantic recall from durable memory.
- `/pensieve-remember [note]` — save a memory, or distill the recent conversation.
- `/pensieve-ask <question>` — ask Pensieve about your data (logs/traces/tables/graph) in KQL/SQL.
- `/pensieve-ingest [source | resources]` — pull from a data source on demand, or create virtual
  graph entities (`ingest_entity`) wired to memory and existing resources.
- `/pensieve-status` — connection, capture mode, and this project's event breakdown.

## The capture table

`default.claude_code_events` (auto-created, schema-evolving):

```kql
claude_code_events
| where realm == "pensieve"
| summarize events = count() by kind
| order by events desc
```

`kind` is one of `session_start` · `user_prompt` · `tool_use` · `assistant_turn` ·
`session_end`. `realm` defaults to the project directory's basename.

## Configuration & privacy

Behavior is controlled by environment variables (`PENSIEVE_CC_CAPTURE` =
`off`/`metadata`/`full`, `PENSIEVE_CC_AUTO_RECALL`, `PENSIEVE_CC_DISTILL`, `PENSIEVE_CC_REALM`,
…). In `full` mode conversation content is sent to **your own** Pensieve server; common
secret shapes are redacted before egress, and `metadata` mode captures event shape
without bodies. See the plugin's `README.md` for the full table.

## Uninstall

```bash
rm -rf ~/.claude/skills/pensieve-memory
```
