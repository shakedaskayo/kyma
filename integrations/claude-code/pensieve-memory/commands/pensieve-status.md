---
description: Show Pensieve connection, capture mode, and what this session has ingested so far.
allowed-tools: Bash(pensieve status:*), Bash(pensieve query:*)
---

Report the Pensieve memory plugin's status.

1. Connection + health:

!`pensieve status 2>/dev/null || echo "no pensieve connection — run: pensieve connect <url>"`

2. Capture configuration (from the environment; defaults in parentheses):
   - capture mode: `${PENSIEVE_CC_CAPTURE:-full}` (off | metadata | full)
   - auto-recall: `${PENSIEVE_CC_AUTO_RECALL:-1}`
   - end-of-session distill: `${PENSIEVE_CC_DISTILL:-1}`
   - firehose target: `${PENSIEVE_CC_DB:-default}.${PENSIEVE_CC_TABLE:-claude_code_events}`

3. Hook capture health — check `~/.pensieve/capture-health.json`:
   - If the file is **absent**, capture is healthy (no recorded failures).
   - If the file is **present**, the last ingest attempt failed. Show its `ts` and `detail` fields
     and suggest: run `pensieve status` for details, or re-run `pensieve install-plugin` to refresh credentials.

4. What's been captured for this project so far:

!`pensieve query "claude_code_events | summarize events=count() by kind | order by events desc" 2>/dev/null || true`

Summarize connection state, capture mode, hook capture health, and the event breakdown.
If not connected, tell the user to run `pensieve connect <url>`.
If capture-health.json exists, warn the user that hook ingests have been silently failing
and show the timestamp and error detail.
