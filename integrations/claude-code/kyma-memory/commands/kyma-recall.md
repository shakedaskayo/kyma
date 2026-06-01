---
description: Recall relevant memories from Kyma for a query (semantic search over your durable memory).
argument-hint: <what to recall>
allowed-tools: Bash(kyma recall:*)
---

Recall durable memories from Kyma that are relevant to: **$ARGUMENTS**

Prefer the bundled MCP tool `recall_memory` (server `kyma`) with:
- `query`: "$ARGUMENTS"
- `realms`: the current project realm (the working directory's basename) plus `global`
- `limit`: 8

If the MCP server is unavailable, fall back to the CLI:

!`kyma recall "$ARGUMENTS" --json 2>/dev/null || echo '{"note":"run: kyma connect <url>"}'`

Then present the results as a short ranked list (type, importance/score, and the memory content). If nothing relevant comes back, say so plainly — do not invent memories.
