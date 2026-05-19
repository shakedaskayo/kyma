#!/usr/bin/env bash
# Test: backcompat-replay.sh runs the fixed query set against a running
# engine and a fixture, and exits 0 on match / non-zero on mismatch.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE_URL="${ENGINE_URL:-http://localhost:8080}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Build a fixture against current engine
./backcompat-snapshot.sh "$ENGINE_URL" "$TMPDIR/fixture"

# Generate expected hashes from current engine (so the test self-matches)
./backcompat-replay.sh --record "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt

# Replay should now pass against the same engine
./backcompat-replay.sh "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt
echo PASS-MATCH

# Mutate one expected hash; replay should fail
sed -i.bak 's/^q-kql-where-count.*/q-kql-where-count\tDEADBEEF/' "$TMPDIR/fixture/expected-hashes.txt"
if ./backcompat-replay.sh "$ENGINE_URL" "$TMPDIR/fixture" backcompat-queries.txt 2>/dev/null; then
  echo "FAIL: replay should have failed on hash mismatch"
  exit 1
fi

echo PASS
