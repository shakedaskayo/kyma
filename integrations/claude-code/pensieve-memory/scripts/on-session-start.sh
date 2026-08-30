#!/usr/bin/env bash
# SessionStart hook: mark the session in the firehose and inject a memory recap
# (recent decisions / preferences / open threads) into the session context.
set -u
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$DIR/lib.sh"

input="$(cat 2>/dev/null)"
have jq || exit 0

sid="$(printf '%s' "$input" | jq -r '.session_id // ""' 2>/dev/null)"
src="$(printf '%s' "$input" | jq -r '.source // ""' 2>/dev/null)"
cwd="${CLAUDE_PROJECT_DIR:-$PWD}"
realm="$(kyma_realm)"

# 1) firehose the session-start marker (best-effort)
evt="$(jq -nc \
  --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" \
  --arg src "$src" --arg cwd "$cwd" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"session_start",source:$src,cwd:$cwd}' \
  2>/dev/null)"
[ -n "$evt" ] && kyma_emit "$evt"

# 2) Claude Code file-memory sync: pick up memory files edited since the
#    last session (detached; curation defers to the quiet window while this
#    session is live, so this is effectively ingest-only). Fail-open.
if [ "$KYMA_CC_FILE_SYNC" = "1" ] && have kyma; then
  ( kyma sync --cc-only >/dev/null 2>&1 & ) >/dev/null 2>&1
fi

# 3) recall recap → stdout becomes additional session context
[ "$KYMA_CC_AUTO_RECALL" = "1" ] || exit 0
have kyma || exit 0
recap="$(kyma recall "open threads, recent decisions, preferences and conventions for the $realm project" \
  --realm "$realm" --limit "$KYMA_CC_RECALL_LIMIT" 2>/dev/null)"
if [ -n "$recap" ]; then
  printf '🧠 Kyma memory — relevant context for "%s":\n%s\n' "$realm" "$recap"
fi

# 4) standing context, injected once per session: how the native memory files
# and kyma relate, and when to reach for the MCP tools instead of relying on
# this auto-injected recap alone.
printf '\n📎 How this fits together: your `~/.claude/projects/*/memory/*.md` files ARE\n'
printf 'kyma'"'"'s synced memory (see `kyma sync`) — writing them already reaches kyma,\n'
printf 'no separate save path needed for routine notes. The recap above is only the\n'
printf 'top %s recalled matches for this project. Call `recall_memory` / `memory_search`\n' "$KYMA_CC_RECALL_LIMIT"
printf '(MCP server `kyma`) directly for anything deeper — older history, live\n'
printf 'data/logs/traces, or the code graph — and `save_memory` for anything you want\n'
printf 'durable and recallable *this session*, rather than waiting on the next sync.\n'
if kyma worker status 2>&1 | grep -q "worker: not installed"; then
  printf '\n⚠️  No background sync worker is installed (`kyma worker install`) — local↔kyma\n'
  printf 'sync currently only runs at session start/end, so mid-session file writes are\n'
  printf 'not durable in kyma until this session ends cleanly.\n'
fi
exit 0
