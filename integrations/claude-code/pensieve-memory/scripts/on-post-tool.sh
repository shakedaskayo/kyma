#!/usr/bin/env bash
# PostToolUse hook: firehose a tool-use event (name + redacted input/response
# preview). These are the "points in the conversation" — edits, commands, fetches.
set -u
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$DIR/lib.sh"

input="$(cat 2>/dev/null)"
have jq || exit 0

sid="$(printf '%s' "$input" | jq -r '.session_id // ""' 2>/dev/null)"
tool="$(printf '%s' "$input" | jq -r '.tool_name // ""' 2>/dev/null)"
realm="$(pensieve_realm)"
[ -z "$tool" ] && exit 0

ti=""
tr_=""
if [ "$PENSIEVE_CC_CAPTURE" = "full" ]; then
  ti_raw="$(printf '%s' "$input" | jq -c '.tool_input // {}' 2>/dev/null)"
  ti="$(pensieve_clean "$ti_raw")"
  tr_raw="$(printf '%s' "$input" | jq -r '
    (.tool_response // .tool_result // "") |
    if type=="object" or type=="array" then tojson else tostring end' 2>/dev/null)"
  tr_="$(pensieve_clean "$tr_raw")"
fi

evt="$(jq -nc \
  --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" \
  --arg tool "$tool" --arg ti "$ti" --arg tr "$tr_" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"tool_use",tool_name:$tool,tool_input:$ti,tool_response_preview:$tr}' \
  2>/dev/null)"
[ -n "$evt" ] && pensieve_emit "$evt"
exit 0
