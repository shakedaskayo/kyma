#!/usr/bin/env bash
# rename-to-pensieve.sh — the Kyma → Pensieve rebrand codemod.
#
# This script is the reviewable artifact for the rename, not the ~15,000-line
# diff it produces. It is idempotent: running it twice is a no-op.
#
#   ./scripts/rename-to-pensieve.sh paths     # Phase 1 — git mv, zero content change
#   ./scripts/rename-to-pensieve.sh content   # Phase 2 — text replace
#   ./scripts/rename-to-pensieve.sh verify    # report what still says "kyma"
#
# Everything it touches comes from `git ls-files`, so untracked and ignored
# trees (node_modules, target, dist, .claude/worktrees) can never be hit.
#
# Four things this script deliberately does NOT do — see the plan, Phases 3-5:
#   • extent magic bytes  b"KYMA\x01"  (must stay 4 bytes -> PNSV)
#   • SQL migrations 001-034           (sqlx checksums them; add 035 instead)
#   • the Tailwind `ky-` prefix        (not a kyma substring; use pv-prefix.mjs)
#   • binary assets                    (mark, icons, screenshots)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# ── Paths excluded from the CONTENT pass ─────────────────────────────────────
# Regenerated, vendored, checksum-sensitive, or hand-edited elsewhere.
is_excluded() {
  case "$1" in
    # this script itself (it documents the old name on purpose)
    scripts/rename-to-pensieve.sh)                return 0 ;;
    # sqlx checksums every applied migration — freeze them, add 035 instead
    crates/*/migrations/*.sql)                    return 0 ;;
    # 4-byte on-disk magic, hand-edited in Phase 3
    crates/*-format-tlm/src/lib.rs)               return 0 ;;
    # generated: Pulumi SDK carries packageName inside a base64 blob
    deploy/pulumi/typescript/sdks/*)              return 0 ;;
    # generated build output committed into the tree
    crates/*-web-assets/embedded/*)               return 0 ;;
    */dist/*|dist/*)                              return 0 ;;
    # lockfiles regenerate
    Cargo.lock|pnpm-lock.yaml|*/package-lock.json) return 0 ;;
    # binaries
    *.png|*.gif|*.ico|*.icns|*.jpg|*.jpeg|*.webp|*.woff|*.woff2|*.ttf) return 0 ;;
  esac
  return 1
}

xform() { sed -e 's/KYMA/PENSIEVE/g' -e 's/Kyma/Pensieve/g' -e 's/kyma/pensieve/g'; }

# ─────────────────────────────────────────────────────────────────────────────
do_paths() {
  local n=0

  # Pass A: directories, shallowest first, one at a time (each git mv
  # invalidates the path list, so we re-derive after every rename).
  while :; do
    local dir
    dir=$(git ls-files | awk -F/ '{
            p="";
            for (i = 1; i < NF; i++) {
              p = (p == "" ? $i : p "/" $i);
              if (tolower($i) ~ /kyma/) { print NF-i "\t" p; break }
            }
          }' | sort -rn -k1,1 | head -1 | cut -f2)
    [ -z "$dir" ] && break
    local new; new=$(printf '%s' "$dir" | xform)
    [ "$dir" = "$new" ] && { echo "!! no-op dir transform: $dir" >&2; exit 1; }
    mkdir -p "$(dirname "$new")"
    git mv "$dir" "$new"
    echo "DIR   $dir -> $new"
    n=$((n + 1))
  done

  # Pass B: files whose basename carries the name.
  local f b d nb
  while IFS= read -r f; do
    b=$(basename "$f"); d=$(dirname "$f")
    nb=$(printf '%s' "$b" | xform)
    [ "$b" = "$nb" ] && continue
    git mv "$f" "$d/$nb"
    echo "FILE  $f -> $d/$nb"
    n=$((n + 1))
  done < <(git ls-files | grep -i kyma || true)

  # Pass C: the Tailwind prefix codemod (ky- is not a kyma substring).
  for f in scripts/ky-prefix.mjs scripts/ky-prefix.test.mjs; do
    if [ -f "$f" ]; then
      git mv "$f" "${f/ky-prefix/pv-prefix}"
      echo "FILE  $f -> ${f/ky-prefix/pv-prefix}"
      n=$((n + 1))
    fi
  done

  echo "paths: $n renamed"
}

# ─────────────────────────────────────────────────────────────────────────────
do_content() {
  local n=0 f
  while IFS= read -r f; do
    is_excluded "$f" && continue
    [ -f "$f" ] || continue
    grep -qi kyma "$f" 2>/dev/null || continue
    # Skip anything that isn't text (belt and braces over the extension list).
    file --mime "$f" | grep -q 'charset=binary' && continue
    xform < "$f" > "$f.pensieve-tmp" && mv "$f.pensieve-tmp" "$f"
    n=$((n + 1))
  done < <(git ls-files)
  echo "content: $n files rewritten"
}

# ─────────────────────────────────────────────────────────────────────────────
do_verify() {
  echo "=== tracked paths still containing kyma ==="
  git ls-files | grep -i kyma || echo "  (none)"
  echo
  echo "=== tracked files still containing kyma ==="
  git grep -il kyma || echo "  (none)"
  echo
  echo "total occurrences: $(git grep -io kyma 2>/dev/null | wc -l | tr -d ' ')"
}

case "${1:-}" in
  paths)   do_paths   ;;
  content) do_content ;;
  verify)  do_verify  ;;
  *) echo "usage: $0 {paths|content|verify}" >&2; exit 2 ;;
esac
