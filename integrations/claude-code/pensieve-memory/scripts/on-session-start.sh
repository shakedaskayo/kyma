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
realm="$(pensieve_realm)"

# 1) firehose the session-start marker (best-effort)
evt="$(jq -nc \
  --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" \
  --arg src "$src" --arg cwd "$cwd" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"session_start",source:$src,cwd:$cwd}' \
  2>/dev/null)"
[ -n "$evt" ] && pensieve_emit "$evt"

# 2) Claude Code file-memory sync: pick up memory files edited since the
#    last session (detached; curation defers to the quiet window while this
#    session is live, so this is effectively ingest-only). Fail-open.
if [ "$PENSIEVE_CC_FILE_SYNC" = "1" ] && have pensieve; then
  ( pensieve sync --cc-only >/dev/null 2>&1 & ) >/dev/null 2>&1
fi

# 3) recall recap → stdout becomes additional session context
[ "$PENSIEVE_CC_AUTO_RECALL" = "1" ] || exit 0
have pensieve || exit 0
recap="$(pensieve recall "open threads, recent decisions, preferences and conventions for the $realm project" \
  --realm "$realm" --limit "$PENSIEVE_CC_RECALL_LIMIT" 2>/dev/null)"
if [ -n "$recap" ]; then
  printf '🧠 Pensieve memory — relevant context for "%s":\n%s\n' "$realm" "$recap"
fi

# 4) standing context, injected once per session: how the native memory files
# and pensieve relate, and when to reach for the MCP tools instead of relying on
# this auto-injected recap alone.
printf '\n📎 How this fits together: your `~/.claude/projects/*/memory/*.md` files ARE\n'
printf 'pensieve'"'"'s synced memory (see `pensieve sync`) — writing them already reaches pensieve,\n'
printf 'no separate save path needed for routine notes. The recap above is only the\n'
printf 'top %s recalled matches for this project. Call `recall_memory` / `memory_search`\n' "$PENSIEVE_CC_RECALL_LIMIT"
printf '(MCP server `pensieve`) directly for anything deeper — older history, live\n'
printf 'data/logs/traces, or the code graph — and `save_memory` for anything you want\n'
printf 'durable and recallable *this session*, rather than waiting on the next sync.\n'
if pensieve worker status 2>&1 | grep -q "worker: not installed"; then
  printf '\n⚠️  No background sync worker is installed (`pensieve worker install`) — local↔pensieve\n'
  printf 'sync currently only runs at session start/end, so mid-session file writes are\n'
  printf 'not durable in pensieve until this session ends cleanly.\n'
fi
exit 0
