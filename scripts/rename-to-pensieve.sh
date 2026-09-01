#!/usr/bin/env bash
# rename-to-pensieve.sh — the Kyma → Pensieve rebrand codemod.
#
# This script is the reviewable artifact for the rename, not the ~15,000-line
# diff it produces. It is idempotent: running it twice is a no-op.
#
#   ./scripts/rename-to-pensieve.sh paths     # Phase 1 — git mv, zero content change
#   ./scripts/rename-to-pensieve.sh content   # Phase 2 — text replace
#   ./scripts/rename-to-pensieve.sh prefix    # Phase 3 — Tailwind ky- -> pv-
#   ./scripts/rename-to-pensieve.sh verify    # report what still says "kyma"
#
# Everything it touches comes from `git ls-files`, so untracked and ignored
# trees (node_modules, target, dist, .claude/worktrees) can never be hit.
#
# Four things this script deliberately does NOT do — see the plan, Phases 3-5:
#   • extent magic bytes  b"KYMA\x01"  (must stay 4 bytes -> PNSV)
#   • SQL migrations 001-034           (sqlx checksums them; add 035 instead)
#   • the Tailwind class prefix        (not a kyma substring; see do_prefix)
#   • binary assets                    (mark, icons, screenshots)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# ── Paths excluded from the CONTENT pass ─────────────────────────────────────
# Regenerated, vendored, checksum-sensitive, or hand-edited elsewhere.
is_excluded() {
  case "$1" in
    # this script itself (it documents the old name on purpose)
    scripts/rename-to-pensieve.sh)                return 0 ;;
    # The migration guide's whole job is to name the old thing next to the new
    # one. A second run of this script rewrote 78 of its lines and turned
    # "`pv-` instead of `ky-`" into "`pv-` instead of `pv-`", quietly gutting
    # the page. Its sidebar entry in the VitePress config links to it by the
    # old filename, so that has to be spared too.
    docs/site/reference/migrating-from-kyma.md)   return 0 ;;
    docs/site/.vitepress/config.ts)               return 0 ;;
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

# Paths that must keep the old name in their FILENAME too, so the path pass
# leaves them alone. Excluding a file from the content pass is not enough:
# do_paths would happily rename migrating-from-kyma.md out from under the
# sidebar link that points at it.
is_pinned_path() {
  case "$1" in
    docs/site/reference/migrating-from-kyma.md) return 0 ;;
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
    is_pinned_path "$f" && continue
    b=$(basename "$f"); d=$(dirname "$f")
    nb=$(printf '%s' "$b" | xform)
    [ "$b" = "$nb" ] && continue
    git mv "$f" "$d/$nb"
    echo "FILE  $f -> $d/$nb"
    n=$((n + 1))
  done < <(git ls-files | grep -i kyma || true)

  # Pass C: the Tailwind prefix codemod. Its old name carries the old class
  # prefix, which is not a "kyma" substring, so the rules above miss it.
  local old new2
  for old in scripts/ky-prefix.mjs scripts/ky-prefix.test.mjs; do
    if [ -f "$old" ]; then
      new2=${old/ky-prefix/pv-prefix}
      git mv "$old" "$new2"
      echo "FILE  $old -> $new2"
      n=$((n + 1))
    fi
  done

  echo "paths: $n renamed"
}

# ─────────────────────────────────────────────────────────────────────────────
# Rewrite the Tailwind utility prefix ky- -> pv- across every tracked file.
#
# MUST be anchored: a blind s/ky-/pv-/ corrupts Tailwind's own `sky-*` colour
# scale (19 occurrences in this tree) into `spv-*`. The lookbehind on an
# alphanumeric character is what keeps `text-sky-500` intact while still
# catching `ky-flex`, `!ky-px-2`, `-ky-mt-2` and `group-hover:ky-flex`.
do_prefix() {
  python3 - <<'PY'
import re, subprocess, pathlib
pat = re.compile(r'(?<![A-Za-z0-9])ky-')
files = subprocess.run(['git', 'ls-files'], capture_output=True, text=True).stdout.split()
# Both of these name the old prefix on purpose; rewriting them turns
# "`pv-` instead of `ky-`" into "`pv-` instead of `pv-`".
SKIP = {
    'scripts/rename-to-pensieve.sh',
    'docs/site/reference/migrating-from-kyma.md',
}
changed = total = 0
for f in files:
    if f in SKIP:
        continue
    p = pathlib.Path(f)
    if not p.is_file():
        continue
    try:
        s = p.read_text()
    except (UnicodeDecodeError, OSError):
        continue
    if 'ky-' not in s:
        continue
    n, cnt = pat.subn('pv-', s)
    if cnt:
        p.write_text(n)
        changed += 1
        total += cnt
print(f'prefix: {changed} files, {total} substitutions')
PY
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
    # Write through the temp but copy it back over the ORIGINAL inode rather
    # than mv'ing the temp into place: mv would replace the file with the
    # temp's default 0644 and silently strip +x. That cost 49 files their
    # executable bit the first time round, including every ./scripts/*.sh
    # that CI invokes directly and the six Claude Code plugin hooks.
    xform < "$f" > "$f.pensieve-tmp" && cat "$f.pensieve-tmp" > "$f"
    rm -f "$f.pensieve-tmp"
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
  prefix)  do_prefix  ;;
  verify)  do_verify  ;;
  *) echo "usage: $0 {paths|content|prefix|verify}" >&2; exit 2 ;;
esac
