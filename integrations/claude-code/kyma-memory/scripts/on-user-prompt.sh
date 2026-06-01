#!/usr/bin/env bash
# UserPromptSubmit hook: firehose the user's prompt, then recall semantically
# relevant memories and inject them as additionalContext for this turn.
set -u
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$DIR/lib.sh"

input="$(cat 2>/dev/null)"
have jq || exit 0

sid="$(printf '%s' "$input" | jq -r '.session_id // ""' 2>/dev/null)"
prompt="$(printf '%s' "$input" | jq -r '.prompt // ""' 2>/dev/null)"
realm="$(kyma_realm)"

# 1) firehose the prompt
text=""
[ "$KYMA_CC_CAPTURE" = "full" ] && text="$(kyma_clean "$prompt")"
plen="$(printf '%s' "$prompt" | wc -c | tr -d ' ')"
evt="$(jq -nc \
  --arg ts "$(now_ts)" --arg sid "$sid" --arg realm "$realm" \
  --arg text "$text" --argjson len "${plen:-0}" \
  '{ts:$ts,session_id:$sid,realm:$realm,kind:"user_prompt",role:"user",text:$text,text_len:$len}' \
  2>/dev/null)"
[ -n "$evt" ] && kyma_emit "$evt"

# 2) auto-recall → additionalContext
[ "$KYMA_CC_AUTO_RECALL" = "1" ] || exit 0
have kyma || exit 0
[ -z "$prompt" ] && exit 0
mem="$(kyma recall "$prompt" --realm "$realm" --limit "$KYMA_CC_RECALL_LIMIT" 2>/dev/null)"
if [ -n "$mem" ]; then
  ctx="$(printf 'Relevant Kyma memories (realm: %s) — consider before answering:\n%s' "$realm" "$mem")"
  jq -nc --arg c "$ctx" \
    '{hookSpecificOutput:{hookEventName:"UserPromptSubmit",additionalContext:$c},suppressOutput:true}' \
    2>/dev/null
fi
exit 0
