#!/usr/bin/env bash
# fresh-install-validate-k8s.sh — Kubernetes (Helm) fresh-install acceptance gate.
#
# Validates the SHIPPED HELM CHART path on a throwaway `kind` cluster: deploys
# in-cluster Postgres + MinIO, helm-installs the engine (deploy/helm/kyma-engine)
# from a locally-built image, waits for the pod to become ready, then asserts the
# engine serves over HTTP from a clean state (health, admin login, ingest, query,
# search) — the k8s analogue of the local and docker-compose fresh-install gates.
# Tears the cluster down on exit.
#
# Requirements: kind, kubectl, helm, docker. A kyma engine image must exist
# locally; pass it via KYMA_IMAGE (default: kyma-smoke-kyma:latest, the tag the
# compose-smoke build produces — run scripts/compose-smoke.sh first, or
# `docker build -t kyma-smoke-kyma:latest .`).
#
# Fully isolated: a dedicated kind cluster (kyma-k8s-smoke) + in-cluster PG/MinIO
# (ClusterIP, never exposed to the host) + a port-forward on an isolated host
# port. Safe to run alongside a developer's live stack.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CLUSTER="kyma-k8s-smoke"
NS="kyma-smoke"
SRC_IMAGE="${KYMA_IMAGE:-kyma-smoke-kyma:latest}"
IMAGE_REPO="kyma-engine"
IMAGE_TAG="smoke"
LPORT="${KYMA_K8S_PORT:-18091}"
BASE="http://127.0.0.1:${LPORT}"

if [[ -t 1 ]]; then RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"; else RED=""; GRN=""; BLU=""; NC=""; fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()  { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
bad() { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
assert_eq() { [[ "$2" == "$3" ]] && ok "$1" || bad "$1 (expected '$2', got '$3')"; }

PF_PID=""
cleanup() {
  section "Tear down"
  [[ -n "$PF_PID" ]] && kill "$PF_PID" 2>/dev/null || true
  kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
  echo "  cluster removed"
}
trap cleanup EXIT

for t in kind kubectl helm docker; do
  command -v "$t" >/dev/null 2>&1 || { echo "missing required tool: $t" >&2; exit 2; }
done
if ! docker image inspect "$SRC_IMAGE" >/dev/null 2>&1; then
  echo "image '$SRC_IMAGE' not found locally — build it first (scripts/compose-smoke.sh or docker build -t $SRC_IMAGE .)" >&2
  exit 2
fi

section "Create kind cluster '$CLUSTER'"
kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
kind create cluster --name "$CLUSTER" --wait 120s || { echo "kind create failed" >&2; exit 1; }
ok "cluster up"

section "Load the engine image into the cluster"
docker tag "$SRC_IMAGE" "${IMAGE_REPO}:${IMAGE_TAG}"
kind load docker-image "${IMAGE_REPO}:${IMAGE_TAG}" --name "$CLUSTER" || { echo "kind load failed" >&2; exit 1; }
kubectl create namespace "$NS" >/dev/null 2>&1 || true
ok "image ${IMAGE_REPO}:${IMAGE_TAG} loaded"

section "Deploy in-cluster Postgres + MinIO"
kubectl apply -n "$NS" -f - <<'YAML'
apiVersion: apps/v1
kind: Deployment
metadata: { name: postgres, labels: { app: postgres } }
spec:
  replicas: 1
  selector: { matchLabels: { app: postgres } }
  template:
    metadata: { labels: { app: postgres } }
    spec:
      containers:
        - name: postgres
          image: pgvector/pgvector:pg16
          env:
            - { name: POSTGRES_USER, value: kyma }
            - { name: POSTGRES_PASSWORD, value: kyma_dev }
            - { name: POSTGRES_DB, value: kyma }
          ports: [{ containerPort: 5432 }]
          readinessProbe:
            exec: { command: ["pg_isready","-U","kyma","-d","kyma"] }
            initialDelaySeconds: 3
            periodSeconds: 3
---
apiVersion: v1
kind: Service
metadata: { name: postgres }
spec:
  selector: { app: postgres }
  ports: [{ port: 5432, targetPort: 5432 }]
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: minio, labels: { app: minio } }
spec:
  replicas: 1
  selector: { matchLabels: { app: minio } }
  template:
    metadata: { labels: { app: minio } }
    spec:
      containers:
        - name: minio
          image: bitnamilegacy/minio:2025.7.23-debian-12-r5
          env:
            - { name: MINIO_ROOT_USER, value: kyma_admin }
            - { name: MINIO_ROOT_PASSWORD, value: kyma_admin_dev }
            - { name: MINIO_DEFAULT_BUCKETS, value: kyma }
          ports: [{ containerPort: 9000 }]
          readinessProbe:
            httpGet: { path: /minio/health/ready, port: 9000 }
            initialDelaySeconds: 5
            periodSeconds: 3
---
apiVersion: v1
kind: Service
metadata: { name: minio }
spec:
  selector: { app: minio }
  ports: [{ port: 9000, targetPort: 9000 }]
YAML
kubectl wait -n "$NS" --for=condition=available --timeout=180s deploy/postgres deploy/minio \
  || { echo "PG/MinIO never became available" >&2; kubectl get pods -n "$NS"; exit 1; }
ok "Postgres + MinIO ready"

section "helm install the engine (all-in-one) against in-cluster PG + MinIO"
helm install kyma deploy/helm/kyma-engine -n "$NS" \
  --set fullnameOverride=kyma \
  --set image.repository="$IMAGE_REPO" \
  --set image.tag="$IMAGE_TAG" \
  --set image.pullPolicy=Never \
  --set serviceAccount.create=true \
  --set-string env.KYMA_HTTP_ADDR="0.0.0.0:8080" \
  --set-string env.KYMA_GRPC_ADDR="off" \
  --set-string env.KYMA_OTLP_ADDR="off" \
  --set-string env.KYMA_S3_ENDPOINT="http://minio:9000" \
  --set-string env.KYMA_S3_BUCKET="kyma" \
  --set-string env.KYMA_S3_REGION="us-east-1" \
  --set-string env.KYMA_S3_PATH_STYLE="true" \
  --set-string env.KYMA_S3_ALLOW_HTTP="true" \
  --set-string env.KYMA_AUTH_BACKEND="session" \
  --set-string env.KYMA_ADMIN_USER="admin" \
  --set-string env.KYMA_ADMIN_PASSWORD="kyma_dev" \
  --set-string secretEnv.KYMA_CATALOG_URL="postgres://kyma:kyma_dev@postgres:5432/kyma" \
  --set-string secretEnv.KYMA_S3_ACCESS_KEY_ID="kyma_admin" \
  --set-string secretEnv.KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev" \
  --set-string secretEnv.KYMA_SECRET_KEY="dev_secret_key_dev_secret_key_32" \
  || { echo "helm install failed" >&2; exit 1; }

section "Wait for the engine pod to roll out"
if ! kubectl rollout status -n "$NS" deploy/kyma -w --timeout=180s; then
  bad "engine deployment never became ready"
  kubectl get pods -n "$NS"
  kubectl logs -n "$NS" -l app.kubernetes.io/name=kyma-engine --tail=80 2>/dev/null || true
  exit 1
fi
ok "engine pod ready"

section "Port-forward the Service to $BASE"
kubectl port-forward -n "$NS" svc/kyma "${LPORT}:8080" --address 127.0.0.1 >/tmp/kyma-k8s-pf.log 2>&1 &
PF_PID=$!
disown "$PF_PID" 2>/dev/null || true
healthy=0
for i in $(seq 1 40); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then healthy=1; break; fi
  sleep 2
done
[[ "$healthy" == "1" ]] && ok "/health reachable via port-forward" || { bad "/health never reachable"; cat /tmp/kyma-k8s-pf.log; exit 1; }

section "Admin login → access token"
TOKEN="$(curl -fsS -X POST "$BASE/v1/auth/login" -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"kyma_dev"}' 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin).get("access_token",""))' 2>/dev/null)"
[[ -n "$TOKEN" ]] && ok "logged in as admin" || bad "login returned no token"
AUTH=(-H "Authorization: Bearer $TOKEN")

section "Ingest + query + search"
ING="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/ingest" \
  -H 'X-Database: default' -H 'X-Table: smoke' -H 'Content-Type: application/x-ndjson' \
  --data-binary $'{"ts":"2026-04-19T10:00:00Z","service":"alpha","status":"200"}\n{"ts":"2026-04-19T10:01:00Z","service":"beta","status":"500"}\n{"ts":"2026-04-19T10:02:00Z","service":"beta","status":"504"}\n' 2>/dev/null)"
echo "$ING" | grep -q '"rows_ingested":3' && ok "ingest accepted (3 rows)" || bad "ingest response: $ING"

CNT="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/query" -H 'X-Database: default' \
  -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM smoke' 2>/dev/null \
  | python3 -c 'import sys,json
rows=[json.loads(l) for l in sys.stdin if l.strip()]
print(rows[0].get("n","?") if rows else "?")' 2>/dev/null)"
assert_eq "row count" "3" "$CNT"

HITS="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/search" -H 'Content-Type: application/json' \
  -d '{"query":"alpha","database":"default","limit":10}' 2>/dev/null \
  | python3 -c 'import sys,json
try:
    d=json.load(sys.stdin); r=d.get("results") or d.get("hits") or []
    print(len(r))
except Exception:
    print("0")' 2>/dev/null)"
[[ "${HITS:-0}" -ge 1 ]] && ok "/v1/search returned $HITS hit(s)" || bad "/v1/search returned no hits"

section "Summary"
printf "PASS: %d  FAIL: %d\n" "$pass" "$fail"
if [[ "$fail" -eq 0 ]]; then printf "${GRN}k8s fresh-install: GREEN${NC}\n"; exit 0
else printf "${RED}k8s fresh-install: FAILED${NC}\n"; exit 1; fi
