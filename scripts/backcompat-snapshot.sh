#!/usr/bin/env bash
# backcompat-snapshot.sh — capture a back-compat fixture of a running engine.
# Usage: backcompat-snapshot.sh <engine-url> <out-dir>
# Requires KYMA_CATALOG_URL (or KYMA_DATABASE_URL as a fallback) to be set to the catalog Postgres URL.
set -euo pipefail

ENGINE_URL="${1:?engine URL required}"
OUT_DIR="${2:?output directory required}"

mkdir -p "$OUT_DIR/sample-extents"

# 1. git sha + build version
git rev-parse HEAD > "$OUT_DIR/git-sha.txt"
cargo metadata --format-version 1 --no-deps \
  | python3 -c "import json,sys; m=json.load(sys.stdin); print(next(p['version'] for p in m['packages'] if p['name']=='kyma-bin'))" \
  > "$OUT_DIR/build-version.txt"

# 2. catalog schema dump (DDL only, no data)
PG_URL="${KYMA_CATALOG_URL:-${KYMA_DATABASE_URL:?KYMA_CATALOG_URL must be set (KYMA_DATABASE_URL also accepted)}}"
pg_dump --schema-only --no-owner --no-privileges "$PG_URL" > "$OUT_DIR/catalog-schema.sql"

# 3. Catalog schema version — query the highest applied sqlx migration version
CATALOG_SCHEMA_VERSION="$(
  psql "$PG_URL" -t -c "SELECT MAX(version) FROM _sqlx_migrations WHERE success = true;" 2>/dev/null \
    | tr -d ' \n' \
  || echo "unknown"
)"

# 4. sample extents — try to fetch a small subset from a catalog endpoint;
#    if no such endpoint exists yet, write an empty marker so the fixture
#    contract is satisfied (sample-extents/ exists). A1 may add the real
#    endpoint later.
EXTENTS_LIST="$(curl -sS "$ENGINE_URL/v1/catalog/extents?limit=3" 2>/dev/null || echo '[]')"
echo "$EXTENTS_LIST" > "$OUT_DIR/sample-extents/extents.json"

# 5. manifest
cat > "$OUT_DIR/manifest.json" <<EOF
{
  "version": "$(cat "$OUT_DIR/build-version.txt")",
  "git_sha": "$(cat "$OUT_DIR/git-sha.txt")",
  "catalog_schema_version": "$CATALOG_SCHEMA_VERSION",
  "created_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

echo "snapshot written to $OUT_DIR"
