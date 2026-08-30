#!/usr/bin/env bash
# Stop hook: firehose the assistant's final message for this turn (best-effort
# extraction from the transcript JSONL).
set -u
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$DIR/lib.sh"

input="$(cat 2>/dev/null)"
have jq || exit 0

sid="$(printf '%s' "$input" | jq -r '.session_id // ""' 2>/dev/null)"
tp="$(printf '%s' "$input" | jq -r '.transcript_path // ""' 2>/dev/null)"
realm="$(kyma_realm)"

text=""
if [ "$KYMA_CC_CAPTURE" = "full" ] && [ -n "$tp" ] && [ -f "$tp" ]; then
  # Last assistant text block in the transcript tail. Schema-tolerant: handle
  # both {message:{content:[{type:text,text}]}} and a flat {content} string.
  last="$(tail -n 300 "$tp" 2>/dev/null | jq -rc '
    select(.type=="assistant")
    | ( .message.content // .content // [] )
    | if type=="array" then (map(select(.type=="text") | .text) | join("\n"))
      elif type=="string" then .
      else "" end' 2>/dev/null | awk 'NF' | tail -n 1)"
  text="$(kyma_clean "$last")"
fi

evt="$(jq -nc \
  --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" --arg text "$text" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"assistant_turn",role:"assistant",text:$text}' \
  2>/dev/null)"
[ -n "$evt" ] && kyma_emit "$evt"
exit 0
