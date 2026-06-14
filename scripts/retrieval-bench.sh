#!/usr/bin/env bash
# retrieval-bench.sh — retrieval-quality + latency benchmark (S0.3 harness).
#
# Boots the docker-compose stack if needed, starts a fresh engine, provisions
# a vector table, runs `retrieval_bench --tier=${TIER:-pr}`, then either
# captures a new baseline (CAPTURE=1 → scripts/fixtures/retrieval-baseline.json)
# or compares against it:
#   - recall_at_10 must be >= baseline recall (tolerance 0.0 — brute force is exact)
#   - latency p95 must be within 1.5x of baseline p95
#
# Tune via:
#   TIER     pr|nightly|release   (default pr)
#   QUERIES  number of bench queries (default 100)
#   CAPTURE  1 = save baseline instead of comparing

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export KYMA_CATALOG_URL="postgres://kyma:kyma_dev@localhost:5433/kyma"
export KYMA_S3_ENDPOINT="http://localhost:9000"
export KYMA_S3_BUCKET="kyma"
export KYMA_S3_ACCESS_KEY_ID="kyma_admin"
export KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev"
export KYMA_S3_PATH_STYLE="true"
export KYMA_S3_ALLOW_HTTP="true"
export KYMA_HTTP_ADDR="127.0.0.1:8080"
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
# Credential-store encryption key (required at boot; bench never stores creds).
export KYMA_SECRET_KEY="${KYMA_SECRET_KEY:-kyma-bench-secret-not-for-production}"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-retrieval-bench.log"
SERVER_PID=""
TIER="${TIER:-pr}"
QUERIES="${QUERIES:-100}"
DB="retrieval_bench"
TABLE="vectors"
DIM=384
BASELINE="$ROOT/scripts/fixtures/retrieval-baseline.json"
REPORT="$(mktemp -t retrieval-report.XXXXXX.json)"

if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; DIM_C="\033[2m"; NC="\033[0m"
else
    RED=""; GRN=""; BLU=""; DIM_C=""; NC=""
fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()      { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
f()       { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
info()    { printf "  ${DIM_C}%s${NC}\n" "$*"; }
cleanup() {
    [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null && wait "$SERVER_PID" 2>/dev/null || true
    rm -f "$REPORT"
    [[ -n "${BIN_DIR:-}" ]] && rm -rf "$BIN_DIR"
}
trap cleanup EXIT

section "Docker stack (Postgres 5433 + MinIO 9000)"
if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    info "stack not up — starting docker compose"
    docker compose up -d --quiet-pull >/dev/null 2>&1 || docker-compose up -d --quiet-pull >/dev/null 2>&1
    for _ in $(seq 1 60); do
        if docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1 &&
           docker exec kyma-minio mc ready local >/dev/null 2>&1; then
            break
        fi
        sleep 1
    done
fi
docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null || { printf "${RED}postgres not healthy${NC}\n"; exit 2; }
ok "stack healthy"

section "Build binaries"
# kyma-bin emits `kyma` (the server); kyma-cli emits `kyma-cli` (the CLI) —
# distinct names since the binary-collision fix, so no copy-aside dance.
BIN_DIR="$(mktemp -d -t retrieval-bench-bins.XXXXXX)"
cargo build -q -p kyma-bin -p kyma-cli
cp target/debug/kyma "$BIN_DIR/kyma-engine"
cp target/debug/kyma-cli "$BIN_DIR/kyma-ctl"
cargo build -q --release -p kyma-retrieval-eval --bin retrieval_bench
ok "built kyma engine, kyma CLI, retrieval_bench"

section "Reset + start kyma"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc alias set local http://localhost:9000 kyma_admin kyma_admin_dev >/dev/null 2>&1 || true
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
"$BIN_DIR/kyma-engine" >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 40); do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi
    sleep 0.5
done
curl -sf "$HTTP_BASE/health" >/dev/null || { printf "${RED}server did not become healthy; see %s${NC}\n" "$LOG_FILE"; exit 1; }
ok "server up on 8080"

section "Provision vector table ($DB.$TABLE, dim=$DIM)"
"$BIN_DIR/kyma-ctl" create-database "$DB" >/dev/null
"$BIN_DIR/kyma-ctl" create-table --db "$DB" --name "$TABLE" \
    --schema "id:string,text:string,embedding:vector($DIM)" >/dev/null
ok "table created"

section "Run retrieval_bench --tier=$TIER --queries=$QUERIES"
./target/release/retrieval_bench \
    --engine-url="$HTTP_BASE" \
    --tier="$TIER" \
    --queries="$QUERIES" \
    --dim="$DIM" \
    --db="$DB" \
    --table="$TABLE" \
    --out="$REPORT"
ok "bench completed"

if [[ "${CAPTURE:-0}" == "1" ]]; then
    section "Capture baseline"
    mkdir -p "$(dirname "$BASELINE")"
    cp "$REPORT" "$BASELINE"
    ok "baseline saved to $BASELINE"
    python3 -m json.tool "$BASELINE"
else
    section "Compare vs baseline"
    if [[ ! -f "$BASELINE" ]]; then
        info "no baseline at $BASELINE — run with CAPTURE=1 to create one (skipping comparison)"
    else
        if python3 - "$REPORT" "$BASELINE" <<'PY'
import json, sys
cur = json.load(open(sys.argv[1]))
base = json.load(open(sys.argv[2]))
failures = []

# Recall must not drop below the baseline at all (tolerance 0.0): brute-force
# SQL is exact today, so any drop means the harness or engine is broken.
if cur["recall_at_10"] < base["recall_at_10"] - 0.0:
    failures.append(
        f"recall_at_10 regressed: {cur['recall_at_10']:.4f} < baseline {base['recall_at_10']:.4f}"
    )

# Latency within 1.5x of baseline p95 (skip when baseline p95 is 0).
base_p95 = base["latency"]["p95_ms"]
if base_p95 > 0 and cur["latency"]["p95_ms"] > 1.5 * base_p95:
    failures.append(
        f"p95 latency regressed: {cur['latency']['p95_ms']:.1f}ms > 1.5x baseline {base_p95:.1f}ms"
    )

print(f"  recall_at_10: current={cur['recall_at_10']:.4f} baseline={base['recall_at_10']:.4f}")
print(f"  p95_ms:       current={cur['latency']['p95_ms']:.1f} baseline={base_p95:.1f}")
for msg in failures:
    print(f"  REGRESSION: {msg}")
sys.exit(1 if failures else 0)
PY
        then
            ok "within baseline tolerances"
        else
            f "baseline comparison failed"
        fi
    fi
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -40 "$LOG_FILE"; exit 1; fi
exit 0
