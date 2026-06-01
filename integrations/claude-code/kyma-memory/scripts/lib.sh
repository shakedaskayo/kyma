#!/usr/bin/env bash
# Shared helpers for the kyma-memory Claude Code plugin.
#
# DESIGN RULE: every hook is best-effort and MUST fail open. Any error here
# should leave the script exiting 0 with no stderr noise — capturing memory must
# never block, slow, or break the user's session. Ingest is fire-and-forget.
set -u

# ── config (env-overridable; sensible zero-config defaults) ──────────────────
KYMA_CC_CAPTURE="${KYMA_CC_CAPTURE:-full}"        # off | metadata | full
KYMA_CC_AUTO_RECALL="${KYMA_CC_AUTO_RECALL:-1}"   # 1 = inject recalled context per prompt
KYMA_CC_DISTILL="${KYMA_CC_DISTILL:-1}"           # 1 = distill memories at session end
KYMA_CC_DB="${KYMA_CC_DB:-default}"               # firehose target database
KYMA_CC_TABLE="${KYMA_CC_TABLE:-claude_code_events}"
KYMA_CC_RECALL_LIMIT="${KYMA_CC_RECALL_LIMIT:-5}"
KYMA_CC_MAXLEN="${KYMA_CC_MAXLEN:-4000}"          # max bytes of any captured text field

have() { command -v "$1" >/dev/null 2>&1; }

now_ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Resolve the memory realm (namespace) for this project. Override with
# KYMA_CC_REALM; otherwise the working directory's basename.
kyma_realm() {
  if [ -n "${KYMA_CC_REALM:-}" ]; then printf '%s' "$KYMA_CC_REALM"; return; fi
  local d="${CLAUDE_PROJECT_DIR:-$PWD}"
  basename "$d" 2>/dev/null || printf 'default'
}

# Redact common secret shapes from stdin → stdout. Disable with KYMA_CC_NO_REDACT=1.
kyma_redact() {
  if [ -n "${KYMA_CC_NO_REDACT:-}" ]; then cat; return; fi
  sed -E \
    -e 's/sk-[A-Za-z0-9_-]{16,}/[redacted-key]/g' \
    -e 's/gh[pousr]_[A-Za-z0-9]{20,}/[redacted-token]/g' \
    -e 's/AKIA[0-9A-Z]{16}/[redacted-aws]/g' \
    -e 's/xox[baprs]-[A-Za-z0-9-]{10,}/[redacted-slack]/g' \
    -e 's/eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/[redacted-jwt]/g' \
    -e 's/-----BEGIN[A-Z ]*PRIVATE KEY-----/[redacted-private-key]/g' \
    2>/dev/null || cat
}

# Clamp + redact a text blob passed as $1.
kyma_clean() {
  printf '%s' "${1:-}" | kyma_redact | head -c "$KYMA_CC_MAXLEN" 2>/dev/null
}

# Ship one compact NDJSON event line ($1) to the firehose. Detached so it
# survives the hook process exiting and never adds latency to the turn.
kyma_emit() {
  [ "$KYMA_CC_CAPTURE" = "off" ] && return 0
  have kyma || return 0
  ( printf '%s\n' "$1" | kyma ingest push --table "$KYMA_CC_TABLE" --db "$KYMA_CC_DB" \
      >/dev/null 2>&1 & ) >/dev/null 2>&1
  return 0
}
