#!/usr/bin/env bash
# Shared helpers for the pensieve-memory Claude Code plugin.
#
# DESIGN RULE: every hook is best-effort and MUST fail open. Any error here
# should leave the script exiting 0 with no stderr noise — capturing memory must
# never block, slow, or break the user's session. Ingest is fire-and-forget.
set -u

# ── config (env-overridable; sensible zero-config defaults) ──────────────────
PENSIEVE_CC_CAPTURE="${PENSIEVE_CC_CAPTURE:-full}"        # off | metadata | full
PENSIEVE_CC_AUTO_RECALL="${PENSIEVE_CC_AUTO_RECALL:-1}"   # 1 = inject recalled context per prompt
PENSIEVE_CC_DISTILL="${PENSIEVE_CC_DISTILL:-1}"           # 1 = distill memories at session end
PENSIEVE_CC_DB="${PENSIEVE_CC_DB:-default}"               # firehose target database
PENSIEVE_CC_TABLE="${PENSIEVE_CC_TABLE:-claude_code_events}"
PENSIEVE_CC_RECALL_LIMIT="${PENSIEVE_CC_RECALL_LIMIT:-5}"
PENSIEVE_CC_MAXLEN="${PENSIEVE_CC_MAXLEN:-4000}"          # max bytes of any captured text field
PENSIEVE_CC_FILE_SYNC="${PENSIEVE_CC_FILE_SYNC:-1}"       # 1 = sync ~/.claude/projects/*/memory files
# Where hook-side capture failures are recorded for `pensieve status` to surface.
PENSIEVE_CAPTURE_HEALTH="${PENSIEVE_CAPTURE_HEALTH:-$HOME/.pensieve/capture-health.json}"
# The file phase honors more env knobs (read by `pensieve sync`, not these hooks):
#   PENSIEVE_CC_CURATE=1                  curation + writeback (archive/promote/index)
#   PENSIEVE_CC_PROMOTE=1                 promote high-value memories to native files
#   PENSIEVE_CC_PROMOTE_MAX=15            hard cap on managed MEMORY.md entries
#   PENSIEVE_CC_PROMOTE_MIN_IMPORTANCE=0.6
#   PENSIEVE_CC_STALE_DAYS=90             LLM stale-review age gate (days)
#   PENSIEVE_CC_DUP_COSINE=0.97           exact-dup merge threshold
#   PENSIEVE_CC_QUIET_WINDOW=300          skip writeback if a session was active (s)
#   PENSIEVE_CC_SYNC_POLL_SECS=30         --watch poll interval
#   PENSIEVE_CC_SYNC_ON_MCP=1             opportunistic sync at `pensieve mcp` startup
#   PENSIEVE_CC_HOME=~/.claude            Claude Code home override

have() { command -v "$1" >/dev/null 2>&1; }

now_ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# Resolve the memory realm (namespace) for this project. Override with
# PENSIEVE_CC_REALM; otherwise the working directory's basename.
pensieve_realm() {
  if [ -n "${PENSIEVE_CC_REALM:-}" ]; then printf '%s' "$PENSIEVE_CC_REALM"; return; fi
  local d="${CLAUDE_PROJECT_DIR:-$PWD}"
  basename "$d" 2>/dev/null || printf 'default'
}

# Redact common secret shapes from stdin → stdout. Disable with PENSIEVE_CC_NO_REDACT=1.
pensieve_redact() {
  if [ -n "${PENSIEVE_CC_NO_REDACT:-}" ]; then cat; return; fi
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
pensieve_clean() {
  printf '%s' "${1:-}" | pensieve_redact | head -c "$PENSIEVE_CC_MAXLEN" 2>/dev/null
}

# Ship one compact NDJSON event line ($1) to the firehose. Detached so it
# survives the hook process exiting and never adds latency to the turn.
# Outcome is recorded to $PENSIEVE_CAPTURE_HEALTH: failures write a marker,
# the next success clears it — so a silent 401 streak is visible to
# `pensieve status` instead of vanishing.
pensieve_emit() {
  [ "$PENSIEVE_CC_CAPTURE" = "off" ] && return 0
  have pensieve || return 0
  (
    err=$(printf '%s\n' "$1" | pensieve ingest push --table "$PENSIEVE_CC_TABLE" --db "$PENSIEVE_CC_DB" 2>&1 >/dev/null)
    if [ $? -eq 0 ]; then
      rm -f "$PENSIEVE_CAPTURE_HEALTH" 2>/dev/null
    else
      mkdir -p "$(dirname "$PENSIEVE_CAPTURE_HEALTH")" 2>/dev/null
      # Sanitize for a JSON string: quotes → apostrophes, backslashes dropped,
      # control chars → spaces (keeps the marker parseable whatever pensieve prints).
      detail=$(printf '%s' "$err" | head -c 300 | tr '"' "'" | tr -d '\\' | tr -c '[:print:]' ' ')
      printf '{"ts":"%s","status":"error","detail":"%s"}\n' "$(now_ts)" "$detail" \
        >"$PENSIEVE_CAPTURE_HEALTH" 2>/dev/null
    fi
  ) >/dev/null 2>&1 &
  return 0
}
