---
description: Show Kyma connection, capture mode, and what this session has ingested so far.
allowed-tools: Bash(kyma status:*), Bash(kyma query:*)
---

Report the Kyma memory plugin's status.

1. Connection + health:

!`kyma status 2>/dev/null || echo "no kyma connection — run: kyma connect <url>"`

2. Capture configuration (from the environment; defaults in parentheses):
   - capture mode: `${KYMA_CC_CAPTURE:-full}` (off | metadata | full)
   - auto-recall: `${KYMA_CC_AUTO_RECALL:-1}`
   - end-of-session distill: `${KYMA_CC_DISTILL:-1}`
   - firehose target: `${KYMA_CC_DB:-default}.${KYMA_CC_TABLE:-claude_code_events}`

3. What's been captured for this project so far:

!`kyma query "claude_code_events | summarize events=count() by kind | order by events desc" 2>/dev/null || true`

Summarize connection state, capture mode, and the event breakdown. If not connected,
tell the user to run `kyma connect <url>`.
