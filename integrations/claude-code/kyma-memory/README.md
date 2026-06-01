# kyma-memory — realtime memory for Claude Code

Turns Claude Code into a live source **and** consumer of your [Kyma](https://github.com/agentcylabs/kyma)
memory layer. As a session unfolds it:

- **Captures** the meaningful points of the conversation (prompts, tool actions,
  assistant turns) into a KQL/SQL-queryable firehose table, in realtime.
- **Recalls** relevant durable memories on every prompt and injects them as context.
- **Distills** durable memories (decisions, preferences, learnings) at session end, so
  later sessions — in any tool wired to the same Kyma — start with the right context.

It bundles a Kyma **MCP server**, so the model can also `recall_memory`, `save_memory`,
and run `run_kql` / `run_sql` against your data directly.

## Install

You need the `kyma` CLI on your PATH and a connection to your Kyma server:

```bash
kyma connect https://your-kyma.example.com --token <token>
kyma install-plugin            # writes the plugin into ~/.claude/skills/kyma-memory
```

Restart Claude Code. Run `/kyma-status` to confirm it's connected and capturing.

`kyma install-plugin` materializes this plugin (templating your server URL + token into
`.mcp.json`) so both the hooks and the bundled MCP server work out of the box.

## What gets captured

The firehose lands in `default.claude_code_events` (auto-created, schema-evolving):

| column | meaning |
|---|---|
| `ts`, `session_id`, `realm` | when / which session / which project |
| `kind` | `session_start` · `user_prompt` · `tool_use` · `assistant_turn` · `session_end` |
| `role`, `text`, `text_len` | message content (redacted; omitted in `metadata` mode) |
| `tool_name`, `tool_input`, `tool_response_preview` | for `tool_use` events |
| `cwd`, `source` | working dir, session source |

Query it like any Kyma table — it also shows up in Discover and the graph:

```kql
claude_code_events
| where realm == "kyma"
| summarize events = count() by kind
| order by events desc
```

## Slash commands

- `/kyma-recall <query>` — semantic recall from your durable memory.
- `/kyma-remember [note]` — save a memory, or distill the recent conversation.
- `/kyma-ask <question>` — ask Kyma about your data (logs/traces/tables/graph) in KQL/SQL.
- `/kyma-status` — connection, capture mode, and this project's event breakdown.

## Configuration (environment variables)

All optional — sensible defaults make it work with zero config.

| var | default | meaning |
|---|---|---|
| `KYMA_CC_CAPTURE` | `full` | `off` · `metadata` (no message/tool bodies) · `full` |
| `KYMA_CC_AUTO_RECALL` | `1` | inject recalled memories as context each prompt |
| `KYMA_CC_DISTILL` | `1` | distill durable memories at session end |
| `KYMA_CC_REALM` | project dir basename | memory/firehose namespace |
| `KYMA_CC_DB` / `KYMA_CC_TABLE` | `default` / `claude_code_events` | firehose target |
| `KYMA_CC_RECALL_LIMIT` | `5` | memories injected per recall |
| `KYMA_CC_MAXLEN` | `4000` | max bytes of any captured text field |
| `KYMA_CC_NO_REDACT` | unset | set to `1` to disable secret redaction (not recommended) |

## Privacy

In `full` mode, conversation content (prompts, tool I/O previews, assistant replies) is
sent to **your own** Kyma server. Common secret shapes (API keys, tokens, JWTs, private
keys) are redacted before egress, but redaction is best-effort — use `KYMA_CC_CAPTURE=metadata`
if you want event shape without bodies, or `off` to disable capture entirely. All hooks
fail open: if Kyma is unreachable, the session is never blocked.

## Uninstall

```bash
rm -rf ~/.claude/skills/kyma-memory
```

(or wherever you installed it via `--target`), then restart Claude Code.
