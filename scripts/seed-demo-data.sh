#!/usr/bin/env bash
# seed-demo-data.sh — Populate the `obs` database with realistic demo data
# for the kyma Web UI.
#
# Usage: ./scripts/seed-demo-data.sh [SERVER_URL]
#   SERVER_URL defaults to http://127.0.0.1:7070
#
# Requires: curl, python3, kyma-cli (./target/debug/kyma-cli) for schema bootstrap.
#
# NOTE: The `dynamic` column type is stored as Arrow Binary, which the Arrow
# JSON NDJSON reader does not support. All tables therefore use flat string/int/
# real columns. The schemas are designed to be immediately queryable with KQL.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="${1:-http://127.0.0.1:7070}"
DB="obs"
BATCH_SIZE=1000

# ── colours ──────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  GRN="\033[32m"; YLW="\033[33m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
  GRN=""; YLW=""; BLU=""; DIM=""; NC=""
fi

info()    { printf "  ${DIM}%s${NC}\n" "$*"; }
success() { printf "  ${GRN}%s${NC}\n" "$*"; }
header()  { printf "\n${BLU}==> %s${NC}\n" "$*"; }

# ── ingest helper ─────────────────────────────────────────────────────────────
ingest_table() {
  local table="$1"
  local ndjson="$2"
  local http_code body tmpf

  tmpf=$(mktemp /tmp/kyma_curl_out.XXXXXX)
  http_code=$(curl -s -o "$tmpf" -w '%{http_code}' \
    -X POST "${SERVER}/v1/ingest" \
    -H "X-Database: ${DB}" \
    -H "X-Table: ${table}" \
    -H 'Content-Type: application/x-ndjson' \
    --data-binary "@${ndjson}")
  body=$(cat "$tmpf"); rm -f "$tmpf"

  if [[ "$http_code" != "2"* ]]; then
    printf "\n[FATAL] Ingest to %s failed (HTTP %s):\n%s\n" "$table" "$http_code" "$body" >&2
    exit 1
  fi
}

# seed_table TABLE NDJSON_FILE — batches file into 1000-row POSTs, prints progress
seed_table() {
  local table="$1"
  local tmpfile="$2"
  local t0 t1 elapsed

  t0=$(date +%s%3N)

  local line_count=0
  local batch_file
  batch_file=$(mktemp /tmp/kyma_seed_batch.XXXXXX)
  trap 'rm -f "$batch_file"' RETURN

  while IFS= read -r line; do
    printf '%s\n' "$line" >> "$batch_file"
    line_count=$((line_count + 1))

    if (( line_count % BATCH_SIZE == 0 )); then
      ingest_table "$table" "$batch_file"
      : > "$batch_file"
    fi
  done < "$tmpfile"

  if [[ -s "$batch_file" ]]; then
    ingest_table "$table" "$batch_file"
  fi

  t1=$(date +%s%3N)
  elapsed=$(echo "scale=1; ($t1 - $t0) / 1000" | bc)
  # Print progress to stderr so stdout (the count) stays clean for capture
  printf "  ${GRN}seeded %s: %d rows in %ss${NC}\n" "$table" "$line_count" "$elapsed" >&2
  echo "$line_count"
}

# ── ensure obs database + tables exist ───────────────────────────────────────
header "Ensuring obs database and tables exist"
CLI="${ROOT}/target/debug/kyma-cli"
if [[ ! -x "$CLI" ]]; then
  info "kyma-cli not found at $CLI — skipping schema bootstrap (tables must pre-exist)"
else
  # Tables are flat-typed (no dynamic/Binary columns) so the Arrow JSON reader
  # can parse NDJSON directly. Errors are suppressed: if the db/table already
  # exists we continue silently.
  "$CLI" create-database "$DB" 2>/dev/null || true

  # otel_logs: ~20 000 rows — log stream with error bursts
  "$CLI" create-table --db "$DB" --name otel_logs \
    --schema 'timestamp:timestamp,service_name:string,severity_text:string,message:string,region:string,http_status_code:int,user_id:string,duration_ms:real,error_code:string,retry_count:int,pod:string' \
    2>/dev/null || true

  # otel_traces: ~15 000 rows — distributed trace spans
  "$CLI" create-table --db "$DB" --name otel_traces \
    --schema 'timestamp:timestamp,service_name:string,span_name:string,span_kind:string,duration_ms:real,status_code:int,trace_id:string,http_method:string,http_url:string,peer_service:string,pod:string,db_statement:string' \
    2>/dev/null || true

  # otel_metrics: ~10 000 rows — one-minute aggregates
  "$CLI" create-table --db "$DB" --name otel_metrics \
    --schema 'timestamp:timestamp,service_name:string,metric_name:string,value:real,endpoint:string,status_bucket:string,pod:string' \
    2>/dev/null || true

  # deploys: ~30 rows — sparse deploy events
  "$CLI" create-table --db "$DB" --name deploys \
    --schema 'timestamp:timestamp,service_name:string,version:string,git_sha:string,author:string,env:string,pr_number:int' \
    2>/dev/null || true

  # llm_calls: ~5 000 rows — agent tool-call telemetry
  "$CLI" create-table --db "$DB" --name llm_calls \
    --schema 'timestamp:timestamp,model:string,tokens_in:int,tokens_out:int,latency_ms:real,tool_name:string,session_id:string,user_intent:string,cost_usd:real' \
    2>/dev/null || true

  info "schema bootstrap complete"
fi

# ── generate NDJSON files ─────────────────────────────────────────────────────
header "Generating demo data (this may take a few seconds)"

TMPDIR_SEED=$(mktemp -d /tmp/kyma_seed.XXXXXX)
trap 'rm -rf "$TMPDIR_SEED"' EXIT

GLOBAL_T0=$(date +%s%3N)

python3 - "$TMPDIR_SEED" <<'PYEOF'
import json, math, random, sys, uuid, os
from datetime import datetime, timezone, timedelta

outdir = sys.argv[1]
rng = random.Random(0xDEADBEEF)

NOW     = datetime.now(timezone.utc)
DAY_AGO = NOW - timedelta(hours=24)

SERVICES = ["api-gateway", "payments", "checkout", "users", "billing", "auth", "workers"]
SVC_WEIGHTS = [35, 20, 15, 12, 10, 5, 3]   # api-gateway highest, workers lowest
REGIONS  = ["us-east-1", "us-west-2", "eu-west-1", "ap-southeast-1"]

def rand_ts(base=DAY_AGO, window_sec=86400):
    delta = rng.uniform(0, window_sec)
    dt = base + timedelta(seconds=delta)
    ms = dt.microsecond // 1000
    return dt.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms:03d}Z"

def lognormal(mu, sigma, lo=None, hi=None):
    v = math.exp(rng.gauss(mu, sigma))
    if lo is not None: v = max(lo, v)
    if hi is not None: v = min(hi, v)
    return v

def rand_svc():
    return rng.choices(SERVICES, weights=SVC_WEIGHTS)[0]

# ── 4 error burst windows (2-minute each) ────────────────────────────────────
burst_starts = sorted(rng.sample(range(0, 86400 - 120), 4))
burst_services = [rng.choice(SERVICES) for _ in range(4)]
BURSTS = [
    (DAY_AGO + timedelta(seconds=s),
     DAY_AGO + timedelta(seconds=s + 120),
     svc)
    for s, svc in zip(burst_starts, burst_services)
]

def in_burst(ts_str):
    """Return (is_burst, burst_service) for a given ISO-8601 timestamp string."""
    try:
        dt = datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
    except Exception:
        return False, None
    for bstart, bend, bsvc in BURSTS:
        if bstart <= dt < bend:
            return True, bsvc
    return False, None

# ─────────────────────────────────────────────────────────────────────────────
# 1. otel_logs (~20 000 rows)
# ─────────────────────────────────────────────────────────────────────────────
MESSAGES = {
    "INFO": [
        "request processed successfully",
        "cache hit for key=u_{uid}",
        "payment authorised for order={oid}",
        "user session refreshed",
        "health check passed",
        "scheduled job completed in {ms}ms",
        "config reloaded",
        "connected to upstream in {ms}ms",
        "response serialised in {ms}ms",
        "rate-limit budget ok (remaining={n})",
        "token refreshed for user={uid}",
        "batch processed: {n} items",
    ],
    "WARN": [
        "retry {r}/5 connecting to upstream",
        "cache miss for key=u_{uid}",
        "slow query detected: {ms}ms > threshold",
        "circuit breaker half-open, probing",
        "disk usage above 80% on pod={pod}",
        "high memory pressure: {mb}MB used",
        "auth token nearing expiry",
        "queue depth elevated: {n} items",
        "connection pool utilisation high ({pct}%)",
        "deprecated endpoint called",
    ],
    "ERROR": [
        "card declined for order={oid}",
        "DB connection pool exhausted",
        "auth token expired for user={uid}",
        "upstream timeout after {ms}ms",
        "unhandled exception in handler",
        "failed to publish event: connection refused",
        "payment gateway returned 402",
        "disk write failed on pod={pod}",
        "RPC to billing-svc failed: deadline exceeded",
        "schema validation error on field amount",
        "circuit breaker OPEN for upstream",
        "max retries exceeded connecting to cache",
    ],
}
ERROR_CODES = [
    "UPSTREAM_TIMEOUT", "DB_POOL_EXHAUSTED", "AUTH_EXPIRED",
    "PAYMENT_DECLINED", "VALIDATION_FAILED", "CIRCUIT_OPEN",
    "CACHE_UNAVAILABLE", "RPC_DEADLINE_EXCEEDED",
]

def fmt_msg(tmpl):
    return tmpl.format(
        uid=rng.randint(1000, 9999), oid=rng.randint(100000, 999999),
        ms=rng.randint(100, 8000), r=rng.randint(1, 5),
        pod=f"pod-{rng.randint(1,8)}", mb=rng.randint(512, 4096),
        n=rng.randint(10, 5000), pct=rng.randint(70, 99),
    )

TARGET_LOGS   = 20000
BURST_LOG_N   = 500   # rows per burst window
NORMAL_LOGS   = TARGET_LOGS - 4 * BURST_LOG_N

logs_out = open(os.path.join(outdir, "otel_logs.ndjson"), "w")

def write_log(ts, svc, sev):
    msg = fmt_msg(rng.choice(MESSAGES[sev]))
    is_err = (sev == "ERROR")
    row = {
        "timestamp":       ts,
        "service_name":    svc,
        "severity_text":   sev,
        "message":         msg,
        "region":          rng.choice(REGIONS),
        "http_status_code": (200 if sev == "INFO"
                             else rng.choice([400, 500, 502, 503]) if sev == "ERROR"
                             else rng.choice([200, 429, 503])),
        "user_id":         f"u_{rng.randint(1000, 9999)}",
        "duration_ms":     round(lognormal(4.5, 1.0, lo=1, hi=30000), 2),
        "error_code":      rng.choice(ERROR_CODES) if is_err else None,
        "retry_count":     rng.randint(0, 5) if is_err else 0,
        "pod":             f"pod-{rng.randint(1, 8)}",
    }
    logs_out.write(json.dumps(row) + "\n")

for _ in range(NORMAL_LOGS):
    ts = rand_ts()
    svc = rand_svc()
    sev = rng.choices(["INFO", "WARN", "ERROR"], weights=[80, 12, 8])[0]
    write_log(ts, svc, sev)

for bstart, bend, bsvc in BURSTS:
    for _ in range(BURST_LOG_N):
        write_log(rand_ts(bstart, 120), bsvc, "ERROR")

logs_out.close()
print("otel_logs generated", flush=True)

# ─────────────────────────────────────────────────────────────────────────────
# 2. otel_traces (~15 000 rows)
# ─────────────────────────────────────────────────────────────────────────────
SPAN_NAMES = [
    "http.server.auth-svc.verify", "http.server.payments.charge",
    "http.server.checkout.submit", "http.server.users.profile",
    "http.client.auth-svc.verify", "http.client.billing.charge",
    "http.client.users.lookup",    "http.client.api-gateway.route",
    "db.query.users.lookup",       "db.query.orders.insert",
    "db.query.payments.update",    "db.query.sessions.get",
    "cache.get", "cache.set", "cache.delete",
    "rpc.billing.charge",  "rpc.auth.validate",  "rpc.users.get",
    "queue.publish.payment_event", "queue.consume.order_created",
    "grpc.checkout.submit",        "grpc.payments.authorize",
    "middleware.auth",      "middleware.rate_limit", "middleware.logging",
    "s3.put_object",        "s3.get_object",
    "redis.get",            "redis.set",            "redis.expire",
    "kafka.produce",        "kafka.consume",
]
SPAN_KINDS = ["server", "client", "internal", "producer", "consumer"]
HTTP_METHODS = ["GET", "POST", "PUT", "DELETE", "PATCH"]
HTTP_URLS = [
    "/api/v1/users/{id}", "/api/v1/orders", "/api/v1/payments/charge",
    "/api/v1/checkout", "/internal/health", "/api/v1/auth/verify",
    "/api/v1/sessions/{id}", "/api/v1/billing/invoice",
]
DB_STMTS = [
    "SELECT * FROM users WHERE id=?",
    "INSERT INTO orders VALUES (?, ?, ?)",
    "UPDATE payments SET status=? WHERE id=?",
    "SELECT * FROM sessions WHERE token=? LIMIT 1",
    "DELETE FROM tokens WHERE expires_at < NOW()",
]

TARGET_TRACES   = 15000
BURST_TRACE_N   = 300

traces_out = open(os.path.join(outdir, "otel_traces.ndjson"), "w")

def write_trace(ts, svc, is_burst=False):
    span = rng.choice(SPAN_NAMES)
    if is_burst:
        dur    = lognormal(7.5, 1.2, lo=500, hi=30000)
        status = rng.choices([200, 500, 503], weights=[30, 50, 20])[0]
    else:
        dur    = lognormal(5.5, 1.5, lo=1, hi=5000)
        if rng.random() < 0.02:
            dur = lognormal(9, 1, lo=5000, hi=60000)
        status = rng.choices([200, 201, 400, 404, 500], weights=[80, 5, 5, 3, 7])[0]

    trace_id = uuid.UUID(int=rng.getrandbits(128), version=4).hex
    url_tmpl = rng.choice(HTTP_URLS)
    url = url_tmpl.replace("{id}", str(rng.randint(100, 9999)))

    row = {
        "timestamp":    ts,
        "service_name": svc,
        "span_name":    span,
        "span_kind":    rng.choice(SPAN_KINDS),
        "duration_ms":  round(dur, 2),
        "status_code":  status,
        "trace_id":     trace_id,
        "http_method":  rng.choice(HTTP_METHODS),
        "http_url":     url,
        "peer_service": rng.choice(SERVICES),
        "pod":          f"pod-{rng.randint(1,8)}",
        "db_statement": rng.choice(DB_STMTS) if "db" in span else None,
    }
    traces_out.write(json.dumps(row) + "\n")

for _ in range(TARGET_TRACES - 4 * BURST_TRACE_N):
    write_trace(rand_ts(), rand_svc())

for bstart, bend, bsvc in BURSTS:
    for _ in range(BURST_TRACE_N):
        write_trace(rand_ts(bstart, 120), bsvc, is_burst=True)

traces_out.close()
print("otel_traces generated", flush=True)

# ─────────────────────────────────────────────────────────────────────────────
# 3. otel_metrics (~10 000 rows, one-minute aggregates)
# ─────────────────────────────────────────────────────────────────────────────
METRIC_NAMES = [
    "request_rate", "error_rate", "p99_latency_ms",
    "cpu_pct", "memory_mb", "queue_depth", "cache_hit_ratio",
]
ENDPOINTS = [
    "/api/v1/users", "/api/v1/orders", "/api/v1/payments",
    "/api/v1/checkout", "/internal/health",
]
STATUS_BUCKETS = ["2xx", "4xx", "5xx"]

def metric_value(name, burst=False):
    if name == "request_rate":    return round(lognormal(4.5, 0.8, lo=1, hi=10000), 2)
    if name == "error_rate":      return round(rng.uniform(0.001, 0.45 if burst else 0.05), 4)
    if name == "p99_latency_ms":  return round(lognormal(5.5, 1.0, lo=5, hi=10000) * (5 if burst else 1), 2)
    if name == "cpu_pct":         return round(rng.uniform(10, 99 if burst else 90), 2)
    if name == "memory_mb":       return round(rng.uniform(128, 3072), 2)
    if name == "queue_depth":     return round(lognormal(3, 1.5, lo=0, hi=50000), 0)
    if name == "cache_hit_ratio": return round(rng.uniform(0.55, 0.99), 4)
    return 0.0

metrics_out = open(os.path.join(outdir, "otel_metrics.ndjson"), "w")

for _ in range(10000):
    delta = rng.uniform(0, 86400)
    dt    = DAY_AGO + timedelta(seconds=delta)
    dt    = dt.replace(second=0, microsecond=0)
    ts    = dt.strftime("%Y-%m-%dT%H:%M:%SZ")
    svc   = rand_svc()
    name  = rng.choice(METRIC_NAMES)
    in_b  = any(bstart <= dt < bend and bsvc == svc for bstart, bend, bsvc in BURSTS)

    row = {
        "timestamp":    ts,
        "service_name": svc,
        "metric_name":  name,
        "value":        metric_value(name, burst=in_b),
        "endpoint":     rng.choice(ENDPOINTS),
        "status_bucket": rng.choice(STATUS_BUCKETS),
        "pod":          f"pod-{rng.randint(1,8)}",
    }
    metrics_out.write(json.dumps(row) + "\n")

metrics_out.close()
print("otel_metrics generated", flush=True)

# ─────────────────────────────────────────────────────────────────────────────
# 4. deploys (~30 rows)
# Placed just before error bursts for a discoverable narrative.
# ─────────────────────────────────────────────────────────────────────────────
AUTHORS = ["alice", "bob", "carlos", "diana", "evan", "fatima", "george", "hana"]

def semver():
    return f"{rng.choice([1,2])}.{rng.randint(0,12)}.{rng.randint(0,20)}"

def git_sha():
    return ''.join(rng.choices('0123456789abcdef', k=7))

deploys_out = open(os.path.join(outdir, "deploys.ndjson"), "w")
deploy_rows = []

# 1-2 deploys per burst, just before it (= narrative: deploy → spike)
for bstart, _, bsvc in BURSTS:
    for i in range(rng.randint(1, 2)):
        dt = bstart - timedelta(minutes=rng.randint(5, 60))
        deploy_rows.append((dt, bsvc, rng.choice(["prod", "staging"])))

# Fill to ~30
for _ in range(30 - len(deploy_rows)):
    dt  = DAY_AGO + timedelta(seconds=rng.uniform(0, 86400))
    deploy_rows.append((dt, rand_svc(), rng.choices(["prod","staging"], weights=[40,60])[0]))

deploy_rows.sort(key=lambda x: x[0])
for dt, svc, env in deploy_rows:
    deploys_out.write(json.dumps({
        "timestamp":    dt.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "service_name": svc,
        "version":      semver(),
        "git_sha":      git_sha(),
        "author":       rng.choice(AUTHORS),
        "env":          env,
        "pr_number":    rng.randint(1000, 9999),
    }) + "\n")

deploys_out.close()
print("deploys generated", flush=True)

# ─────────────────────────────────────────────────────────────────────────────
# 5. llm_calls (~5 000 rows)
# ─────────────────────────────────────────────────────────────────────────────
MODELS        = ["claude-opus-4-7", "claude-sonnet-4-6", "gpt-4o", "gpt-4o-mini"]
MODEL_WEIGHTS = [15, 40, 30, 15]
TOOLS         = [None, "grep", "read_file", "write_file", "shell",
                 "browser", "python_repl", "list_files", "search", "sql_query"]
TOOL_WEIGHTS  = [30, 15, 12, 10, 10, 5, 5, 5, 4, 4]
INTENTS       = [
    "debug perf regression", "summarize logs", "generate migration",
    "explain error", "write unit tests", "refactor auth module",
    "optimize SQL query", "review PR diff", "trace request flow",
    "find memory leak", "generate API docs", "parse log file",
    "create dashboard query", "investigate incident", "update schema",
]
# Input/output cost per token (USD)
COST_IN  = {"claude-opus-4-7": 15e-6, "claude-sonnet-4-6": 3e-6, "gpt-4o": 5e-6, "gpt-4o-mini": 0.15e-6}
COST_OUT = {"claude-opus-4-7": 75e-6, "claude-sonnet-4-6": 15e-6, "gpt-4o": 15e-6, "gpt-4o-mini": 0.6e-6}

llm_out = open(os.path.join(outdir, "llm_calls.ndjson"), "w")

for _ in range(5000):
    model   = rng.choices(MODELS, weights=MODEL_WEIGHTS)[0]
    tin     = int(lognormal(6.5, 1.0, lo=50, hi=4000))
    tout    = int(lognormal(5.5, 1.0, lo=10, hi=2000))
    latency = round(lognormal(7.5, 1.2, lo=200, hi=15000), 2)
    tool    = rng.choices(TOOLS, weights=TOOL_WEIGHTS)[0]
    cost    = round(tin * COST_IN[model] + tout * COST_OUT[model], 6)
    llm_out.write(json.dumps({
        "timestamp":   rand_ts(),
        "model":       model,
        "tokens_in":   tin,
        "tokens_out":  tout,
        "latency_ms":  latency,
        "tool_name":   tool,
        "session_id":  str(uuid.UUID(int=rng.getrandbits(128), version=4)),
        "user_intent": rng.choice(INTENTS),
        "cost_usd":    cost,
    }) + "\n")

llm_out.close()
print("llm_calls generated", flush=True)
print("ALL_DONE", flush=True)
PYEOF

info "Data generation complete"

# ── ingest ────────────────────────────────────────────────────────────────────
header "Ingesting data into ${SERVER}"

TOTAL_ROWS=0
COUNTS_otel_logs=0
COUNTS_otel_traces=0
COUNTS_otel_metrics=0
COUNTS_deploys=0
COUNTS_llm_calls=0

for table in otel_logs otel_traces otel_metrics deploys llm_calls; do
  f="${TMPDIR_SEED}/${table}.ndjson"
  rows=$(seed_table "$table" "$f")
  TOTAL_ROWS=$((TOTAL_ROWS + rows))
  # Use indirect variable naming (bash 3 compatible)
  eval "COUNTS_${table}=${rows}"
done

# ── validation ────────────────────────────────────────────────────────────────
header "Validation"

VALIDATE=$(curl -s -X POST "${SERVER}/v1/query" \
  -H 'Content-Type: application/x-kql' \
  -H "X-Database: ${DB}" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | take 3')

ERROR_ROWS=$(printf '%s\n' "$VALIDATE" | python3 -c "
import sys, json
lines = [l.strip() for l in sys.stdin if l.strip()]
print(len(lines))
")

if (( ERROR_ROWS < 3 )); then
  printf "\n[FATAL] Validation failed: expected >= 3 ERROR rows, got %d\n" "$ERROR_ROWS" >&2
  printf "Response:\n%s\n" "$VALIDATE" >&2
  exit 1
fi
success "Validation passed: ERROR rows returned = ${ERROR_ROWS}"

# ── summary ───────────────────────────────────────────────────────────────────
GLOBAL_T1=$(date +%s%3N)
TOTAL_ELAPSED=$(echo "scale=1; ($GLOBAL_T1 - $GLOBAL_T0) / 1000" | bc)

header "Seeding complete"
printf "\n  ${GRN}Total rows: %d  |  Wall time: %ss${NC}\n\n" "$TOTAL_ROWS" "$TOTAL_ELAPSED"
info "  otel_logs:    ${COUNTS_otel_logs} rows"
info "  otel_traces:  ${COUNTS_otel_traces} rows"
info "  otel_metrics: ${COUNTS_otel_metrics} rows"
info "  deploys:      ${COUNTS_deploys} rows"
info "  llm_calls:    ${COUNTS_llm_calls} rows"

# ── example queries ───────────────────────────────────────────────────────────
printf "\n"
cat <<'QUERIES'
══════════════════════════════════════════════════════════════════════════
  Example KQL queries to try in the Web UI   (database: obs)
══════════════════════════════════════════════════════════════════════════

  1. Error summary by service (last 1 h):
     otel_logs | where severity_text == "ERROR" | where timestamp > ago(1h) | summarize n=count() by service_name

  2. Slow traces (> 1 s) in last 1 h:
     otel_traces | where duration_ms > 1000 | where timestamp > ago(1h) | take 50

  3. p99 latency trend (last 2 h):
     otel_metrics | where metric_name == "p99_latency_ms" | where timestamp > ago(2h)

  4. LLM shell-tool cost by model:
     llm_calls | where tool_name == "shell" | summarize total=sum(cost_usd) by model

  5. Recent deployments (last 24 h):
     deploys | where timestamp > ago(24h) | project timestamp, service_name, version, author

  6. Latest 100 errors (all time):
     otel_logs | where severity_text == "ERROR" | take 100

  7. Top error messages:
     otel_logs | where severity_text == "ERROR" | summarize n=count() by message | sort by n desc | take 20

  8. LLM token + cost breakdown by model:
     llm_calls | summarize avg_in=avg(tokens_in), avg_out=avg(tokens_out), total_cost=sum(cost_usd), calls=count() by model

══════════════════════════════════════════════════════════════════════════
QUERIES

# ── Demo dashboard "Production Pulse" ────────────────────────────────────────
echo ""
echo "Seeding demo dashboard…"

DASH=$(curl -sS -X POST "$SERVER/v1/dashboards" \
  -H 'content-type: application/json' \
  -d '{"name":"Production Pulse","description":"Errors, latency, and deploys across the stack","time_range_preset":"24h","refresh_interval_seconds":60}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')

curl -sS -X PATCH "$SERVER/v1/dashboards/$DASH" \
  -H 'content-type: application/json' \
  -d @- <<JSON
{
  "panels": [
    { "title": "Errors by service", "panel_type": "chart", "query": "otel_logs | where severity_text == \"ERROR\" | where timestamp > ago(24h) | summarize n=count() by service_name | order by n desc", "database_name": "obs", "config": {}, "grid_x": 0, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 0 },
    { "title": "p99 latency (ms)", "panel_type": "chart", "query": "otel_metrics | where metric_name == \"p99_latency_ms\" | where timestamp > ago(24h) | take 500", "database_name": "obs", "config": {"chartType":"line"}, "grid_x": 6, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 1 },
    { "title": "Recent deploys", "panel_type": "table", "query": "deploys | where timestamp > ago(24h) | project timestamp, service_name, version, author, env", "database_name": "obs", "config": {"maxRows":20}, "grid_x": 0, "grid_y": 4, "grid_w": 6, "grid_h": 4, "display_order": 2 },
    { "title": "Total LLM spend (24h)", "panel_type": "stat", "query": "llm_calls | where timestamp > ago(24h) | summarize total=sum(cost_usd)", "database_name": "obs", "config": {"label":"USD, last 24h", "format":"number"}, "grid_x": 6, "grid_y": 4, "grid_w": 3, "grid_h": 2, "display_order": 3 },
    { "title": "Error count (24h)", "panel_type": "stat", "query": "otel_logs | where severity_text == \"ERROR\" | where timestamp > ago(24h) | summarize n=count()", "database_name": "obs", "config": {"label":"error events", "format":"number"}, "grid_x": 9, "grid_y": 4, "grid_w": 3, "grid_h": 2, "display_order": 4 },
    { "title": "About this dashboard", "panel_type": "markdown", "query": null, "database_name": null, "config": {"markdown":"# Production Pulse\n\nQuick-look dashboard for on-call. All panels are last-24h unless noted.\n\n- **Errors by service** — per-service error counts\n- **p99 latency** — request-latency trend\n- **Recent deploys** — what shipped lately\n- **LLM spend** — agent cost burn\n- **Error count** — overall error volume\n\nUse the time range picker above to scope all panels."}, "grid_x": 0, "grid_y": 8, "grid_w": 12, "grid_h": 3, "display_order": 5 }
  ]
}
JSON

echo "Seeded dashboard: $DASH"
