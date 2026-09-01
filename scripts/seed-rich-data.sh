#!/usr/bin/env bash
# seed-rich-data.sh — second-wave demo data that layers cross-entity
# relationships on top of the observability baseline from
# seed-demo-data.sh.
#
# What this adds to the `obs` database:
#   users          1 000 rows   (user_id, email, tier, signup_ts)
#   services         60 rows   (service_id, name, owner_team, lang, tier)
#   service_edges   230 rows   (src_service → dst_service, weight, kind)
#   api_calls    60 000 rows   (ts, user_id, service_name, endpoint,
#                                status, latency_ms, trace_id)
#   llm_memos     2 000 rows   (id, user_id, content,
#                                embedding vector(384), category)
#
# Cross-references built in on purpose so the agent's relationship tools
# (find_references_to, graph_traverse) have something meaningful to chew
# on:
#   - `user_id` appears in api_calls AND llm_memos
#   - `service_name` appears in api_calls AND service_edges
#   - `trace_id` in api_calls ties back to otel_traces (when both are seeded)
#   - `service_edges` forms a real microservice graph walkable via
#     graph-traverse
#
# Vector embeddings for llm_memos are cluster-centred (5 categories:
# billing / auth / deploy / perf / bug) with Gaussian noise — enough for
# cosine_distance searches to return semantically-grouped neighbours.
#
# Usage: ./scripts/seed-rich-data.sh [SERVER_URL]
#   SERVER_URL defaults to http://127.0.0.1:8080

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="${1:-http://127.0.0.1:8080}"
DB="obs"
CLI="${ROOT}/target/release/pensieve-cli"
[[ -x "$CLI" ]] || CLI="${ROOT}/target/debug/pensieve-cli"

if [[ -t 1 ]]; then
  GRN="\033[32m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
  GRN=""; BLU=""; DIM=""; NC=""
fi
info()    { printf "  ${DIM}%s${NC}\n" "$*"; }
success() { printf "  ${GRN}%s${NC}\n" "$*"; }
header()  { printf "\n${BLU}==> %s${NC}\n" "$*"; }

# ── bootstrap schema (idempotent: pensieve-cli create-* are safe to re-run
#    because duplicate db/table names are handled via ON CONFLICT in the
#    catalog layer — in practice they'll error once and continue) ──────
header "Bootstrap tables"

"$CLI" create-database obs 2>&1 | tail -1 || true
"$CLI" create-table --db obs --name users --schema \
  "user_id:string,email:string,tier:string,signup_ts:timestamp,country:string" \
  2>&1 | tail -1 || true
"$CLI" create-table --db obs --name services --schema \
  "service_id:string,name:string,owner_team:string,lang:string,tier:string" \
  2>&1 | tail -1 || true
"$CLI" create-table --db obs --name service_edges --schema \
  "src_service:string,dst_service:string,weight:real,kind:string" \
  2>&1 | tail -1 || true
"$CLI" create-table --db obs --name api_calls --schema \
  "ts:timestamp,user_id:string,service_name:string,endpoint:string,status:int,latency_ms:real,trace_id:string,region:string" \
  2>&1 | tail -1 || true
"$CLI" create-table --db obs --name llm_memos --schema \
  "id:string,user_id:string,content:string,category:string,embedding:vector(384),created_ts:timestamp" \
  2>&1 | tail -1 || true
success "schema ready"

# ── generate deterministic data via python3 ──────────────────────────────
header "Generate data (python3)"
TMP=$(mktemp -d /tmp/pensieve-rich.XXXXXX)
trap 'rm -rf "$TMP"' EXIT

TMP_FOR_PY="$TMP" python3 - <<'PY' > /dev/null
import json, random, hashlib, os, math, time
random.seed(20260421)

tmp = os.environ["TMP_FOR_PY"]
os.makedirs(tmp, exist_ok=True)

# ---------------- users (1000) ----------------
FIRST = ["alex","sam","jordan","riley","casey","robin","morgan","taylor","jamie","dana",
        "avery","quinn","parker","rowan","hayden","skylar","rory","drew","emerson","harper"]
LAST  = ["nguyen","kowalski","lopez","patel","singh","okafor","ivanov","takahashi","sorrento","dumas",
        "kirsch","dubois","kim","liang","mbeki","romano","sato","halsey","perez","okonkwo"]
TIERS = ["free","free","free","pro","pro","enterprise"]
COUNTRIES = ["US","US","GB","DE","FR","BR","JP","IN","CA","AU","SE","NL"]
now = int(time.time() * 1_000_000_000)

users = []
for i in range(1000):
    uid = f"u-{i:04d}"
    fn, ln = random.choice(FIRST), random.choice(LAST)
    tier = random.choice(TIERS)
    signup = now - random.randint(60*60*24*30, 60*60*24*365*3) * 1_000_000_000
    users.append({
        "user_id": uid,
        "email": f"{fn}.{ln}@{random.choice(['example.com','acme.io','demo.dev'])}",
        "tier": tier,
        "signup_ts": signup,
        "country": random.choice(COUNTRIES),
    })
with open(f"{tmp}/users.ndjson","w") as f:
    for u in users: f.write(json.dumps(u)+"\n")

# ---------------- services (60) ----------------
SERVICES = [
    # (name, team, lang, tier)
    ("auth",        "platform", "go",      "critical"),
    ("api-gateway", "platform", "rust",    "critical"),
    ("billing",     "payments", "java",    "critical"),
    ("checkout",    "commerce", "typescript", "critical"),
    ("cart",        "commerce", "typescript", "high"),
    ("catalog",     "commerce", "python",  "high"),
    ("search",      "discovery","rust",    "high"),
    ("recommend",   "discovery","python",  "medium"),
    ("user-profile","identity", "go",      "high"),
    ("notifications","comms",   "python",  "medium"),
    ("email",       "comms",    "python",  "medium"),
    ("sms",         "comms",    "python",  "low"),
    ("payments-worker","payments","java",  "critical"),
    ("fraud",       "trust",    "python",  "high"),
    ("ledger",      "payments", "java",    "critical"),
    ("inventory",   "commerce", "go",      "high"),
    ("shipping",    "commerce", "go",      "high"),
    ("returns",     "commerce", "go",      "medium"),
    ("support",     "success",  "typescript", "low"),
    ("analytics",   "data",     "python",  "medium"),
    ("etl-pipeline","data",     "python",  "low"),
    ("ml-trainer",  "ml",       "python",  "medium"),
    ("ml-serving",  "ml",       "python",  "high"),
    ("config",      "platform", "go",      "critical"),
    ("feature-flags","platform","go",      "high"),
    ("audit",       "security", "go",      "critical"),
    ("secrets",     "security", "go",      "critical"),
    ("metrics-collector","observability","rust","high"),
    ("log-router",  "observability","rust","high"),
    ("trace-router","observability","rust","medium"),
    ("web-frontend","web",      "typescript","critical"),
    ("admin-ui",    "web",      "typescript","medium"),
    ("mobile-api",  "mobile",   "go",      "high"),
    ("push-svc",    "mobile",   "go",      "medium"),
    ("rate-limiter","platform", "rust",    "high"),
    ("throttle",    "platform", "rust",    "medium"),
    ("scheduler",   "platform", "go",      "medium"),
    ("queue",       "platform", "go",      "high"),
    ("cache",       "platform", "rust",    "high"),
    ("session",     "identity", "go",      "critical"),
    ("kyc",         "identity", "python",  "high"),
    ("sso",         "identity", "go",      "critical"),
    ("webhook",     "platform", "go",      "medium"),
    ("chat",        "comms",    "typescript","medium"),
    ("voice",       "comms",    "python",  "low"),
    ("docs-api",    "success",  "typescript","low"),
    ("billing-exporter","data", "python",  "low"),
    ("finops",      "data",     "python",  "low"),
    ("reports",     "data",     "python",  "low"),
    ("tax",         "payments", "java",    "high"),
    ("currency",    "payments", "java",    "high"),
    ("reconcile",   "payments", "java",    "high"),
    ("dispute",     "payments", "java",    "medium"),
    ("chargeback",  "payments", "java",    "medium"),
    ("wallet",      "payments", "java",    "high"),
    ("payout",      "payments", "java",    "high"),
    ("subscription","payments", "java",    "high"),
    ("coupon",      "commerce", "typescript","medium"),
    ("loyalty",     "commerce", "typescript","low"),
    ("marketplace", "commerce", "python",  "medium"),
]
services = [
    {"service_id": f"svc-{i:03d}", "name": s[0], "owner_team": s[1],
     "lang": s[2], "tier": s[3]}
    for i, s in enumerate(SERVICES)
]
with open(f"{tmp}/services.ndjson","w") as f:
    for s in services: f.write(json.dumps(s)+"\n")

# ---------------- service_edges (~230) ----------------
# Realistic microservice dependency graph: web → gateway → auth/billing/search/…
# Build a layered DAG: edge (src, dst, kind, weight).
KINDS = ["http","grpc","kafka","queue"]
svc_names = [s["name"] for s in services]
edges = []
# Hand-authored critical-path edges first
critical = [
    ("web-frontend","api-gateway","http",1.0),
    ("mobile-api","api-gateway","http",1.0),
    ("api-gateway","auth","grpc",0.95),
    ("api-gateway","session","grpc",0.9),
    ("api-gateway","rate-limiter","grpc",0.7),
    ("api-gateway","catalog","http",0.6),
    ("api-gateway","search","http",0.6),
    ("api-gateway","cart","http",0.5),
    ("cart","checkout","http",0.9),
    ("checkout","billing","http",1.0),
    ("checkout","inventory","http",0.8),
    ("checkout","fraud","grpc",0.7),
    ("billing","payments-worker","queue",0.95),
    ("billing","ledger","grpc",0.9),
    ("payments-worker","payout","queue",0.8),
    ("payments-worker","tax","grpc",0.7),
    ("fraud","user-profile","grpc",0.6),
    ("fraud","audit","kafka",0.5),
    ("search","catalog","http",0.8),
    ("recommend","catalog","http",0.6),
    ("recommend","ml-serving","grpc",0.9),
    ("ml-serving","config","grpc",0.3),
    ("notifications","email","queue",0.5),
    ("notifications","sms","queue",0.3),
    ("notifications","push-svc","queue",0.5),
    ("user-profile","kyc","grpc",0.7),
    ("user-profile","sso","grpc",0.8),
    ("sso","auth","grpc",0.9),
    ("auth","secrets","grpc",0.8),
    ("auth","audit","kafka",0.4),
    ("support","user-profile","grpc",0.5),
    ("analytics","etl-pipeline","queue",0.8),
    ("etl-pipeline","ml-trainer","queue",0.7),
    ("ml-trainer","ml-serving","queue",0.5),
    ("metrics-collector","log-router","kafka",0.9),
    ("log-router","trace-router","kafka",0.8),
    ("audit","log-router","kafka",0.6),
    ("config","feature-flags","grpc",0.7),
    ("queue","scheduler","grpc",0.4),
    ("cache","config","grpc",0.3),
    ("webhook","queue","queue",0.7),
    ("chat","notifications","queue",0.5),
    ("reports","analytics","grpc",0.6),
    ("billing-exporter","ledger","grpc",0.6),
    ("billing-exporter","finops","queue",0.7),
    ("subscription","billing","grpc",0.8),
    ("wallet","ledger","grpc",0.8),
    ("dispute","chargeback","grpc",0.9),
    ("chargeback","ledger","grpc",0.7),
    ("reconcile","ledger","grpc",0.9),
    ("currency","tax","grpc",0.4),
    ("coupon","catalog","grpc",0.3),
    ("loyalty","user-profile","grpc",0.4),
    ("marketplace","catalog","http",0.6),
    ("marketplace","inventory","http",0.5),
    ("shipping","inventory","http",0.7),
    ("returns","shipping","http",0.8),
    ("returns","inventory","http",0.5),
    ("admin-ui","api-gateway","http",0.7),
    ("docs-api","api-gateway","http",0.3),
    ("throttle","rate-limiter","grpc",0.5),
    ("chat","user-profile","grpc",0.3),
]
for (s,d,k,w) in critical:
    edges.append({"src_service": s, "dst_service": d, "weight": w, "kind": k})
# Random fill-ins to reach ~230 edges
while len(edges) < 230:
    s = random.choice(svc_names); d = random.choice(svc_names)
    if s == d: continue
    if any(e for e in edges if e["src_service"]==s and e["dst_service"]==d): continue
    edges.append({"src_service": s, "dst_service": d,
                  "weight": round(random.uniform(0.1, 0.9), 2),
                  "kind": random.choice(KINDS)})
with open(f"{tmp}/service_edges.ndjson","w") as f:
    for e in edges: f.write(json.dumps(e)+"\n")

# ---------------- api_calls (60 000) ----------------
ENDPOINTS = ["/api/v1/login","/api/v1/checkout","/api/v1/cart","/api/v1/search",
             "/api/v1/catalog","/api/v1/user","/api/v1/payments","/api/v1/orders",
             "/api/v1/metrics","/api/v1/feature-flags","/api/v2/recommend",
             "/api/v1/notifications","/api/v1/deploy","/api/v1/health",
             "/api/v1/ledger","/api/v1/fraud-check","/api/v1/kyc"]
REGIONS = ["us-east-1","us-west-2","eu-west-1","eu-central-1","ap-south-1","ap-northeast-1"]
user_ids = [u["user_id"] for u in users]

with open(f"{tmp}/api_calls.ndjson","w") as f:
    for i in range(60000):
        ts = now - random.randint(0, 60*60*72) * 1_000_000_000
        svc = random.choice(svc_names)
        uid = random.choice(user_ids)
        ep  = random.choice(ENDPOINTS)
        # Skew status distribution: mostly 200s, 5% errors
        r = random.random()
        status = 200 if r < 0.90 else (500 if r < 0.95 else (404 if r < 0.98 else 429))
        # Log-normal latency, shifted by status class
        lat = max(1.0, min(30000.0, random.lognormvariate(4.5, 0.9)))
        if status == 500: lat *= 2.0
        trace_id = f"trc-{random.randint(0,999999):06d}"
        region = random.choice(REGIONS)
        f.write(json.dumps({
            "ts": ts, "user_id": uid, "service_name": svc, "endpoint": ep,
            "status": status, "latency_ms": round(lat, 1),
            "trace_id": trace_id, "region": region,
        })+"\n")

# ---------------- llm_memos (2000, with 384-dim clustered vectors) ------
CATEGORIES = ["billing","auth","deploy","perf","bug"]
# Each category has a 384-dim "centre" vector (random-but-deterministic),
# individual memos are centre + small Gaussian noise, L2-normalised.
def centre_for(cat):
    rnd = random.Random(hashlib.sha1(cat.encode()).hexdigest())
    v = [rnd.gauss(0.0, 1.0) for _ in range(384)]
    n = math.sqrt(sum(x*x for x in v))
    return [x/n for x in v]

centres = {c: centre_for(c) for c in CATEGORIES}

SNIPPETS = {
    "billing":  ["Invoice failed for user {u}", "Refund processed: ${amt}",
                 "Subscription upgrade for {u}", "Payment declined: card_expired",
                 "Dispute opened: chargeback_pending"],
    "auth":     ["Login failure rate spike for {u}", "MFA challenge required",
                 "Password reset email sent to {u}", "OAuth token refresh",
                 "Session expired for {u}"],
    "deploy":   ["Deploy {ver} failed health checks", "Rollback triggered for {svc}",
                 "Canary at 10% for {svc}", "Feature flag {svc}-beta enabled",
                 "Config drift detected in {svc}"],
    "perf":     ["p99 latency 1200ms in {svc}", "Cache hit rate dropped to 72%",
                 "Database CPU at 91% for {svc}", "Memory leak suspected in {svc}",
                 "Throttle kicking in at 250rps"],
    "bug":      ["NPE in {svc} endpoint /api/v1/orders",
                 "Stack trace attached: TypeError: Cannot read property",
                 "Regression after deploy {ver}", "Data corruption in {svc} table",
                 "Timeout after 30s calling {svc}"],
}
with open(f"{tmp}/llm_memos.ndjson","w") as f:
    for i in range(2000):
        cat = random.choice(CATEGORIES)
        uid = random.choice(user_ids)
        tmpl = random.choice(SNIPPETS[cat])
        content = tmpl.format(
            u=uid, amt=f"{random.uniform(1,999):.2f}",
            ver=f"v{random.randint(1,9)}.{random.randint(0,20)}.{random.randint(0,9)}",
            svc=random.choice(svc_names),
        )
        base = centres[cat]
        noise = [random.gauss(0, 0.15) for _ in range(384)]
        v = [b + n for b, n in zip(base, noise)]
        nrm = math.sqrt(sum(x*x for x in v))
        v = [x/nrm for x in v]
        ts = now - random.randint(0, 60*60*24*7) * 1_000_000_000
        f.write(json.dumps({
            "id": f"memo-{i:05d}",
            "user_id": uid,
            "content": content,
            "category": cat,
            "embedding": v,
            "created_ts": ts,
        })+"\n")

print("generated")
PY
success "data generated"

# ── ingest loop ───────────────────────────────────────────────────────────
ingest() {
  local table="$1"; local file="$2"; local batch=2000
  local total; total=$(wc -l < "$file" | tr -d ' ')
  local start; start=$(date +%s)
  # Split into batches to keep payloads under a few MiB (vectors bloat).
  split -l "$batch" "$file" "$file.batch."
  local sent=0
  for b in "$file".batch.*; do
    local code
    code=$(curl -s -o /tmp/pensieve-ing-err -w '%{http_code}' \
      -X POST "${SERVER}/v1/ingest" \
      -H "X-Database: ${DB}" -H "X-Table: ${table}" \
      -H 'Content-Type: application/x-ndjson' \
      --data-binary "@${b}")
    if [[ "$code" != 2* ]]; then
      printf "ingest %s failed (http %s):\n%s\n" "$table" "$code" "$(cat /tmp/pensieve-ing-err)" >&2
      return 1
    fi
    sent=$((sent + $(wc -l < "$b" | tr -d ' ')))
    printf "\r  %s: %d / %d rows  " "$table" "$sent" "$total"
  done
  local elapsed=$(( $(date +%s) - start ))
  printf "\r  %s%s: %d rows in %ds%s\n" "$GRN" "$table" "$total" "$elapsed" "$NC"
}

header "Ingest"
ingest users          "$TMP/users.ndjson"
ingest services       "$TMP/services.ndjson"
ingest service_edges  "$TMP/service_edges.ndjson"
ingest api_calls      "$TMP/api_calls.ndjson"
ingest llm_memos      "$TMP/llm_memos.ndjson"

header "Done"
TOTAL=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tA \
  -c "SELECT SUM((column_stats->>'_row_count')::int) FROM extents" 2>/dev/null \
  || echo "?")
success "Rich seed complete. 5 new tables with ~63 000 rows."
cat <<MSG

Example queries to try — hit ${SERVER} or the Web UI at http://localhost:5173:

  1. Top users by api_call volume:
     api_calls | summarize n=count() by user_id | top 10 by n

  2. Error rate per service (last 3h):
     api_calls | where ts > ago(3h) | summarize err=countif(status >= 500), n=count() by service_name

  3. Users with BOTH api_calls AND llm_memos (graph-style cross-ref):
     Ask the agent: "Which users have both api_calls and llm_memos? Give me the top 5."

  4. Service dependency traversal:
     Ask the agent: "What services does 'checkout' depend on transitively, 3 hops deep?"

  5. Vector similarity (find memos like a billing question):
     Ask the agent: "Which memos are most similar to 'subscription upgrade failed'?"

MSG
