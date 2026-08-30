#!/usr/bin/env bash
# SessionEnd hook: distill the session into durable memories. Runs detached so
# it never delays shutdown; the kyma agent (which owns save_memory) does the work.
set -u
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$DIR/lib.sh"

input="$(cat 2>/dev/null)"
have jq || exit 0
[ "$KYMA_CC_DISTILL" = "1" ] || exit 0
[ "$KYMA_CC_CAPTURE" = "off" ] && exit 0
have kyma || exit 0

sid="$(printf '%s' "$input" | jq -r '.session_id // ""' 2>/dev/null)"
tp="$(printf '%s' "$input" | jq -r '.transcript_path // ""' 2>/dev/null)"
reason="$(printf '%s' "$input" | jq -r '.reason // ""' 2>/dev/null)"
realm="$(kyma_realm)"

# Firehose a session_end marker (best-effort) so the timeline/closes are visible.
evt="$(jq -nc --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" --arg reason "$reason" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"session_end",reason:$reason}' 2>/dev/null)"
[ -n "$evt" ] && kyma_emit "$evt"

# Claude Code file-memory sync: ingest any memory files this session wrote,
# then curate (promote high-value memories into native files + MEMORY.md).
# Detached and fail-open; the session just ended, so writeback can proceed.
if [ "$KYMA_CC_FILE_SYNC" = "1" ]; then
  ( kyma sync --cc-only >/dev/null 2>&1 & ) >/dev/null 2>&1
fi

[ -n "$tp" ] && [ -f "$tp" ] || exit 0

# Feed a bounded transcript tail to `kyma distill`, fully detached.
( tail -n 600 "$tp" 2>/dev/null \
    | kyma distill --session "$sid" --realm "$realm" >/dev/null 2>&1 & ) >/dev/null 2>&1
exit 0
