---
description: Ask Pensieve about your data — logs, traces, tables, schemas, code/graph — in KQL or SQL.
argument-hint: <question about your data>
allowed-tools: Bash(pensieve query:*)
---

Answer using live data from the user's Pensieve deployment: **$ARGUMENTS**

Use the bundled `pensieve` MCP server's query tools directly when you can:
- `explore_schema` / `describe_table` to find the right table and columns,
- `run_kql` or `run_sql` to compute the answer,
- `recall_memory` if prior context/decisions are relevant.

If you'd rather let Pensieve's own agent plan the query, shell out:

!`pensieve query "$ARGUMENTS" 2>/dev/null || echo "(run: pensieve connect <url>)"`

Summarize the result for the user. This is read-only — never attempt writes.
