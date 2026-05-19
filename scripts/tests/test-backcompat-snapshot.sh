#!/usr/bin/env bash
# Test: backcompat-snapshot.sh produces a fixture directory with the
# expected files against a running kyma engine.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE_URL="${ENGINE_URL:-http://localhost:8080}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

./backcompat-snapshot.sh "$ENGINE_URL" "$TMPDIR/fixture"

# Required artifacts
for f in manifest.json catalog-schema.sql sample-extents/ git-sha.txt build-version.txt; do
  if [ ! -e "$TMPDIR/fixture/$f" ]; then
    echo "FAIL: missing $f"
    exit 1
  fi
done

# manifest.json must include at least these fields
python3 -c "
import json, sys
m = json.load(open('$TMPDIR/fixture/manifest.json'))
for k in ('version', 'git_sha', 'catalog_schema_version', 'created_at'):
    assert k in m, k
"

echo PASS
