#!/usr/bin/env bash
# End-to-end test for the kyma ⇄ Claude Code file-memory sync.
#
# Exercises the FULL lifecycle against the real binary in a throwaway
# sandbox (synthetic ~/.claude home + isolated kyma store — your real
# files are never touched):
#
#   ingest (frontmatter→nodes, wikilinks→edges, realm via transcript cwd)
#   → recall over file memories (MCP memory_search)
#   → idempotent re-scan
#   → promotion (save_memory → native kyma-*.md + managed MEMORY.md region)
#   → user edit of a promoted file (pulled back, user-owned, never rewritten)
#   → file edit (new node version, no duplicate)
#   → file deletion (node archived) + duplicate files (merged, tombstoned)
#   → quiet window (active session defers writeback)
#   → the plugin session hooks end-to-end (detached sync fires)
#   → final idempotency + audit log
#
# Prereqs: jq; `cargo build -p kyma-cli` already run (or KYMA=<path>).
# The first run downloads the fastembed ONNX model into the sandbox.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
KYMA="${KYMA:-$REPO/target/debug/kyma}"
command -v jq >/dev/null || { echo "jq required"; exit 1; }
[ -x "$KYMA" ] || { echo "kyma binary not found at $KYMA (cargo build -p kyma-cli)"; exit 1; }

# ------------------------------------------------------------------
# Sandbox
# ------------------------------------------------------------------
SANDBOX="$(mktemp -d /tmp/kyma-cc-e2e.XXXXXX)"
trap 'rm -rf "$SANDBOX"' EXIT
export KYMA_HOME="$SANDBOX/kyma"
export KYMA_CC_HOME="$SANDBOX/cc"
# Reuse an already-downloaded model cache when present (faster, offline-safe).
mkdir -p "$KYMA_HOME"
[ -d "$REPO/.fastembed_cache" ] && cp -R "$REPO/.fastembed_cache" "$KYMA_HOME/.fastembed_cache"

PROJ="$SANDBOX/work/e2e-proj"
SLUG="$(printf '%s' "$PROJ" | sed -E 's/[^A-Za-z0-9]/-/g')"
MEM="$KYMA_CC_HOME/projects/$SLUG/memory"
mkdir -p "$MEM" "$KYMA_CC_HOME/sessions" "$PROJ"
# Realm resolution falls back to the transcript cwd for unknown slugs.
echo "{\"cwd\":\"$PROJ\"}" > "$KYMA_CC_HOME/projects/$SLUG/0000-session.jsonl"

cat > "$MEM/MEMORY.md" <<'EOF'
# Memory index

- [Auth model](auth-model.md) — session tokens
EOF
cat > "$MEM/auth-model.md" <<'EOF'
---
name: auth-model
description: Auth model decision
metadata:
  type: project
  originSessionId: 11111111-1111-1111-1111-111111111111
---

We use session tokens, not JWTs. See [[build-notes]].
EOF
cat > "$MEM/build-notes.md" <<'EOF'
---
name: build-notes
description: Build and test preferences
metadata:
  type: user
---

Always run the suite with cargo nextest.
EOF

PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "  ✓ $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  ✗ $1"; }
check(){ local d="$1"; shift; if "$@" >/dev/null 2>&1; then ok "$d"; else bad "$d"; fi; }
step() { echo; echo "── $1"; }

# One-shot MCP call: $1 = tools/call params JSON. Prints the result line.
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}'
mcp() {
  printf '%s\n{"jsonrpc":"2.0","id":2,"method":"tools/call","params":%s}\n' "$INIT" "$1" \
    | KYMA_CC_SYNC_ON_MCP=0 "$KYMA" mcp 2>/dev/null | tail -1
}
sql() { mcp "{\"name\":\"run_sql\",\"arguments\":{\"database\":\"memory\",\"sql\":$(jq -Rs . <<<"$1")}}" \
        | jq -r '.result.structuredContent'; }
sync_out() { "$KYMA" sync --cc-only 2>&1 | grep 'cc-sync' || true; }

LATEST="WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) rn FROM memory_nodes)"

# ------------------------------------------------------------------
step "1. initial ingest (files → embedded nodes, wikilinks → edges)"
out="$(sync_out)"; echo "  $out"
nodes="$(sql "$LATEST SELECT realm, title, topic_key FROM latest WHERE rn=1 AND topic_key LIKE 'claude-md:%'")"
check "two file memories ingested"        test "$(jq '.rows | length' <<<"$nodes")" = 2
check "realm resolved from transcript"    test "$(jq -r '.rows[0].realm' <<<"$nodes")" = "e2e-proj"
edges="$(sql "SELECT DISTINCT id FROM memory_edges WHERE type = 'RELATES_TO'")"
check "wikilink became a RELATES_TO edge" test "$(jq '.rows | length' <<<"$edges")" = 1

step "2. recall over file memories (hybrid search)"
hit="$(mcp '{"name":"memory_search","arguments":{"query":"which test runner should I use for the suite","realm":"e2e-proj"}}' \
      | jq -r '.result.structuredContent.memories[0].title')"
echo "  top hit: $hit"
check "build preference ranks first" grep -qi "build and test preferences" <<<"$hit"

step "3. idempotent re-scan"
out="$(sync_out)"; echo "  $out"
check "everything skipped, nothing re-embedded" grep -q "0 ingested, 2 unchanged" <<<"$out"

step "4. promotion: high-value kyma memories become native files"
mcp '{"name":"save_memory","arguments":{"content":"Releases go through the staging gate; never tag before the staging smoke suite is green.","title":"Staging Gate Rule","memory_type":"decision","importance":0.95,"realm":"e2e-proj","sync":true}}' >/dev/null
mcp '{"name":"save_memory","arguments":{"content":"Error messages must include the failing resource id, never just the error class.","title":"Error Message Convention","memory_type":"preference","importance":0.85,"realm":"e2e-proj","sync":true}}' >/dev/null
out="$(sync_out)"; echo "  $out"
check "two promoted files written"       test -f "$MEM/kyma-staging-gate-rule.md" -a -f "$MEM/kyma-error-message-convention.md"
check "promoted file is kyma-stamped"    grep -q "source: kyma" "$MEM/kyma-staging-gate-rule.md"
check "user MEMORY.md line intact"       grep -q "\[Auth model\](auth-model.md)" "$MEM/MEMORY.md"
check "managed region lists promotions"  grep -q "kyma-staging-gate-rule.md" "$MEM/MEMORY.md"
check "managed markers present"          grep -q "BEGIN kyma-managed" "$MEM/MEMORY.md"

step "5. user edits a promoted file → kyma pulls it back, stops rewriting"
sed -i '' 's/never tag before/NEVER EVER tag before/' "$MEM/kyma-staging-gate-rule.md" 2>/dev/null \
  || sed -i 's/never tag before/NEVER EVER tag before/' "$MEM/kyma-staging-gate-rule.md"
out="$(sync_out)"; echo "  $out"
owned="$(sql "$LATEST SELECT content, provenance FROM latest WHERE rn=1 AND title = 'Staging Gate Rule'")"
check "edit pulled into the node"  grep -q "NEVER EVER" <<<"$(jq -r '.rows[0].content' <<<"$owned")"
check "node marked user-owned"     test "$(jq -r '.rows[0].provenance | fromjson | .cc_user_owned' <<<"$owned")" = "true"
out="$(sync_out)"
check "file not rewritten on the next pass" grep -q "NEVER EVER" "$MEM/kyma-staging-gate-rule.md"

step "6. editing a file-born memory updates the same node"
cat > "$MEM/auth-model.md" <<'EOF'
---
name: auth-model
description: Auth model decision
metadata:
  type: project
---

We use session tokens AND refresh tokens. See [[build-notes]].
EOF
out="$(sync_out)"; echo "  $out"
ids="$(sql "SELECT DISTINCT id FROM memory_nodes WHERE topic_key = 'claude-md:$SLUG/auth-model'")"
check "still exactly one node"  test "$(jq '.rows | length' <<<"$ids")" = 1
cur="$(sql "$LATEST SELECT content FROM latest WHERE rn=1 AND topic_key = 'claude-md:$SLUG/auth-model'")"
check "latest version has the edit" grep -q "refresh tokens" <<<"$(jq -r '.rows[0].content' <<<"$cur")"

step "7. deletion archives; duplicates merge with tombstones"
rm "$MEM/build-notes.md"
cat > "$MEM/dup-a.md" <<'EOF'
---
name: dup-a
metadata:
  type: project
---

Deploys run from CI only, never from laptops.
EOF
sed 's/name: dup-a/name: dup-b/' "$MEM/dup-a.md" > "$MEM/dup-b.md"
out="$(sync_out)"; echo "  $out"
gone="$(sql "$LATEST SELECT status FROM latest WHERE rn=1 AND topic_key = 'claude-md:$SLUG/build-notes'")"
check "deleted file's node archived"   test "$(jq -r '.rows[0].status' <<<"$gone")" = "archived"
check "duplicate file tombstoned"      test -f "$MEM/archive/dup-b.md"
check "tombstone keeps the body"       grep -q "CI only" "$MEM/archive/dup-b.md"
check "tombstone says why"             grep -q "archived_reason" "$MEM/archive/dup-b.md"
merged="$(sql "SELECT DISTINCT id FROM memory_edges WHERE type = 'MERGED_INTO'")"
check "MERGED_INTO edge recorded"      test "$(jq '.rows | length' <<<"$merged")" -ge 1

step "8. quiet window: an active session defers writeback"
NOW_MS=$(( $(date +%s) * 1000 ))
echo "{\"pid\":1,\"cwd\":\"$PROJ\",\"updatedAt\":$NOW_MS,\"status\":\"busy\"}" > "$KYMA_CC_HOME/sessions/live.json"
mcp '{"name":"save_memory","arguments":{"content":"Quiet-window probe memory.","title":"Quiet Probe","memory_type":"decision","importance":0.9,"realm":"e2e-proj","sync":true}}' >/dev/null
out="$(sync_out)"; echo "  $out"
check "no file written while session active" test ! -f "$MEM/kyma-quiet-probe.md"
check "deferral recorded in audit log" grep -q '"skipped_quiet":true' "$KYMA_HOME/cc-curation.log"
rm "$KYMA_CC_HOME/sessions/live.json"
out="$(sync_out)"
check "written after the session ends" test -f "$MEM/kyma-quiet-probe.md"

step "9. plugin session hooks fire the sync (detached, fail-open)"
export PATH="$(dirname "$KYMA"):$PATH"
echo '{"session_id":"e2e","source":"startup"}' \
  | bash integrations/claude-code/kyma-memory/scripts/on-session-start.sh >/dev/null 2>&1
check "session-start hook exits 0" test $? = 0
cat > "$MEM/hook-note.md" <<'EOF'
---
name: hook-note
metadata:
  type: project
---

Written between sessions; the hook sync should pick this up.
EOF
echo '{"session_id":"e2e","reason":"exit"}' \
  | bash integrations/claude-code/kyma-memory/scripts/on-session-end.sh >/dev/null 2>&1
check "session-end hook exits 0" test $? = 0
deadline=$(( $(date +%s) + 90 ))
until sql "$LATEST SELECT id FROM latest WHERE rn=1 AND topic_key = 'claude-md:$SLUG/hook-note'" | jq -e '.rows | length == 1' >/dev/null 2>&1; do
  [ "$(date +%s)" -ge "$deadline" ] && break
  sleep 2
done
hooked="$(sql "$LATEST SELECT id FROM latest WHERE rn=1 AND topic_key = 'claude-md:$SLUG/hook-note'")"
check "hook-triggered sync ingested the new file" test "$(jq '.rows | length' <<<"$hooked")" = 1

step "10. final idempotency + audit trail"
out="$(sync_out)"; echo "  $out"
check "steady state: nothing to do" grep -q "0 ingested" <<<"$out"
check "audit log has the full history" test "$(wc -l < "$KYMA_HOME/cc-curation.log")" -ge 5

echo
echo "══ $PASS passed, $FAIL failed ══"
exit "$([ "$FAIL" = 0 ] && echo 0 || echo 1)"
