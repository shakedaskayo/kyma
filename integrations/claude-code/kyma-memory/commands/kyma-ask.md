---
description: Ask Kyma about your data — logs, traces, tables, schemas, code/graph — in KQL or SQL.
argument-hint: <question about your data>
allowed-tools: Bash(kyma query:*)
---

Answer using live data from the user's Kyma deployment: **$ARGUMENTS**

Use the bundled `kyma` MCP server's query tools directly when you can:
- `explore_schema` / `describe_table` to find the right table and columns,
- `run_kql` or `run_sql` to compute the answer,
- `recall_memory` if prior context/decisions are relevant.

If you'd rather let Kyma's own agent plan the query, shell out:

!`kyma query "$ARGUMENTS" 2>/dev/null || echo "(run: kyma connect <url>)"`

Summarize the result for the user. This is read-only — never attempt writes.
