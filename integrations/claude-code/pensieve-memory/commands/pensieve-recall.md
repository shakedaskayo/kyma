---
description: Recall relevant memories from Pensieve for a query (semantic search over your durable memory).
argument-hint: <what to recall>
allowed-tools: Bash(pensieve recall:*)
---

Recall durable memories from Pensieve that are relevant to: **$ARGUMENTS**

Prefer the bundled MCP tool `recall_memory` (server `pensieve`) with:
- `query`: "$ARGUMENTS"
- `realms`: the current project realm (the working directory's basename) plus `global`
- `limit`: 8

If the MCP server is unavailable, fall back to the CLI:

!`pensieve recall "$ARGUMENTS" --json 2>/dev/null || echo '{"note":"run: pensieve connect <url>"}'`

Then present the results as a short ranked list (type, importance/score, and the memory content). If nothing relevant comes back, say so plainly — do not invent memories.
