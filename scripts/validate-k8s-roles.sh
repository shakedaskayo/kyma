#!/usr/bin/env bash
# validate-k8s-roles.sh — Helm ROLE-SPLIT topology validation on a multi-node
# kind cluster (S2.4/S2.6 production deployment shape).
#
# Where fresh-install-validate-k8s.sh installs the single all-in-one Deployment,
# this exercises `roles.enabled=true`: separate edge / committer / worker
# Deployments on a 2-worker kind cluster, with staged ingest
# (PENSIEVE_INGEST_MODE=staged) so the committer actually has work to drain. It
# proves:
#   1. all three role deployments roll out and the Service serves;
#   2. the staged path is end-to-end — an edge node STAGES an ingest and the
#      lease-elected committer COMMITS it (the row becomes visible after the
#      commit window, not immediately — the test polls for it);
#   3. committer FAILOVER — delete the committer pod and a fresh one takes the
#      Postgres advisory-lock lease and keeps committing.
#
# Fully isolated (dedicated kind cluster + in-cluster ClusterIP PG/MinIO + an
# isolated host port); deletes the cluster on exit.
#
# Requirements: kind, kubectl, helm, docker, and a local engine image
# (PENSIEVE_IMAGE, default pensieve-smoke-pensieve:latest).

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"; cd "$ROOT"

CLUSTER="pensieve-roles-smoke"
NS="pensieve-roles"
SRC_IMAGE="${PENSIEVE_IMAGE:-pensieve-smoke-pensieve:latest}"
IMAGE_REPO="pensieve-engine"; IMAGE_TAG="roles"
LPORT="${PENSIEVE_ROLES_PORT:-18092}"
BASE="http://127.0.0.1:${LPORT}"

if [[ -t 1 ]]; then RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"; else RED=""; GRN=""; BLU=""; NC=""; fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()  { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
bad() { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }

PF_PID=""
cleanup() {
  section "Tear down"
  [[ -n "$PF_PID" ]] && kill "$PF_PID" 2>/dev/null || true
  if [[ "${PENSIEVE_KEEP_CLUSTER:-0}" == "1" ]]; then
    echo "  PENSIEVE_KEEP_CLUSTER=1 — leaving cluster '$CLUSTER' up for inspection"
    echo "  (delete later: kind delete cluster --name $CLUSTER)"
    return
  fi
  kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
  echo "  cluster removed"
}
trap cleanup EXIT

for t in kind kubectl helm docker; do command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t" >&2; exit 2; }; done
docker image inspect "$SRC_IMAGE" >/dev/null 2>&1 || { echo "image '$SRC_IMAGE' not found (build it first)" >&2; exit 2; }

# Authenticated query helper: prints COUNT(*) of the smoke table, or "?".
count_rows() {
  curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/query" -H 'X-Database: default' \
    -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM smoke' 2>/dev/null \
  | python3 -c 'import sys,json
rows=[json.loads(l) for l in sys.stdin if l.strip()]
print(rows[0].get("n","?") if rows else "?")' 2>/dev/null
}
# Stage one ingest request of a single row (req id $1). Prints the HTTP code so a
# staging failure is visible rather than silently swallowed.
stage_one() {
  # NDJSON auto-create infers every column as Utf8, so all values must be JSON
  # strings (a numeric rid is rejected with a 400 decode error).
  local payload
  payload="$(printf '{"ts":"2026-04-19T10:00:00Z","service":"alpha","rid":"%s"}\n' "$1")"
  curl -s -o /dev/null -w '%{http_code}' "${AUTH[@]}" -X POST "$BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: smoke' -H 'Content-Type: application/x-ndjson' \
    --data-binary "$payload"
}

section "Create 2-worker kind cluster '$CLUSTER'"
kind delete cluster --name "$CLUSTER" >/dev/null 2>&1 || true
cat <<'KIND' | kind create cluster --name "$CLUSTER" --wait 120s --config -
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
  - role: worker
  - role: worker
KIND
[[ "$?" == "0" ]] && ok "cluster up (1 control-plane + 2 workers)" || { echo "kind create failed" >&2; exit 1; }

section "Load engine image + namespace"
docker tag "$SRC_IMAGE" "${IMAGE_REPO}:${IMAGE_TAG}"
kind load docker-image "${IMAGE_REPO}:${IMAGE_TAG}" --name "$CLUSTER" || { echo "kind load failed" >&2; exit 1; }
kubectl create namespace "$NS" >/dev/null 2>&1 || true
ok "image loaded"

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
            - { name: POSTGRES_USER, value: pensieve }
            - { name: POSTGRES_PASSWORD, value: pensieve_dev }
            - { name: POSTGRES_DB, value: pensieve }
          ports: [{ containerPort: 5432 }]
          readinessProbe:
            exec: { command: ["pg_isready","-U","pensieve","-d","pensieve"] }
            initialDelaySeconds: 3
            periodSeconds: 3
---
apiVersion: v1
kind: Service
metadata: { name: postgres }
spec: { selector: { app: postgres }, ports: [{ port: 5432, targetPort: 5432 }] }
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
            - { name: MINIO_ROOT_USER, value: pensieve_admin }
            - { name: MINIO_ROOT_PASSWORD, value: pensieve_admin_dev }
            - { name: MINIO_DEFAULT_BUCKETS, value: pensieve }
          ports: [{ containerPort: 9000 }]
          readinessProbe:
            httpGet: { path: /minio/health/ready, port: 9000 }
            initialDelaySeconds: 5
            periodSeconds: 3
---
apiVersion: v1
kind: Service
metadata: { name: minio }
spec: { selector: { app: minio }, ports: [{ port: 9000, targetPort: 9000 }] }
YAML
kubectl wait -n "$NS" --for=condition=available --timeout=180s deploy/postgres deploy/minio \
  || { echo "PG/MinIO not ready" >&2; kubectl get pods -n "$NS"; exit 1; }
ok "Postgres + MinIO ready"

section "helm install with roles.enabled=true + staged ingest"
helm install pensieve deploy/helm/pensieve-engine -n "$NS" \
  --set fullnameOverride=pensieve \
  --set image.repository="$IMAGE_REPO" --set image.tag="$IMAGE_TAG" --set image.pullPolicy=Never \
  --set roles.enabled=true --set roles.edge.replicas=2 --set roles.worker.replicas=1 \
  --set-string env.PENSIEVE_HTTP_ADDR="0.0.0.0:8080" \
  --set-string env.PENSIEVE_GRPC_ADDR="off" \
  --set-string env.PENSIEVE_OTLP_ADDR="off" \
  --set-string env.PENSIEVE_INGEST_MODE="staged" \
  --set-string env.PENSIEVE_COMMIT_WINDOW_MS="1000" \
  --set-string env.PENSIEVE_S3_ENDPOINT="http://minio:9000" \
  --set-string env.PENSIEVE_S3_BUCKET="pensieve" \
  --set-string env.PENSIEVE_S3_REGION="us-east-1" \
  --set-string env.PENSIEVE_S3_PATH_STYLE="true" \
  --set-string env.PENSIEVE_S3_ALLOW_HTTP="true" \
  --set-string env.PENSIEVE_AUTH_BACKEND="session" \
  --set-string env.PENSIEVE_ADMIN_USER="admin" \
  --set-string env.PENSIEVE_ADMIN_PASSWORD="pensieve_dev" \
  --set-string secretEnv.PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@postgres:5432/pensieve" \
  --set-string secretEnv.PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin" \
  --set-string secretEnv.PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev" \
  --set-string secretEnv.PENSIEVE_SECRET_KEY="dev_secret_key_dev_secret_key_32" \
  || { echo "helm install failed" >&2; exit 1; }

section "Wait for the three role deployments to roll out"
roll_ok=1
for d in pensieve-edge pensieve-committer pensieve-worker; do
  if kubectl rollout status -n "$NS" "deploy/$d" -w --timeout=180s; then
    ok "$d rolled out"
  else
    bad "$d did not roll out"; roll_ok=0
    kubectl get pods -n "$NS"; kubectl logs -n "$NS" "deploy/$d" --tail=40 2>/dev/null || true
  fi
done
[[ "$roll_ok" == "1" ]] || exit 1
# Confirm pods actually landed on more than one node (multi-node placement).
NODES_USED="$(kubectl get pods -n "$NS" -o jsonpath='{range .items[*]}{.spec.nodeName}{"\n"}{end}' | sort -u | wc -l | tr -d ' ')"
[[ "${NODES_USED:-1}" -ge 2 ]] && ok "pods spread across $NODES_USED nodes" || echo "  (note: pods on $NODES_USED node)"

section "Port-forward the Service to $BASE"
kubectl port-forward -n "$NS" svc/pensieve "${LPORT}:8080" --address 127.0.0.1 >/tmp/pensieve-roles-pf.log 2>&1 &
PF_PID=$!; disown "$PF_PID" 2>/dev/null || true
healthy=0
for i in $(seq 1 40); do curl -fsS "$BASE/health" >/dev/null 2>&1 && { healthy=1; break; }; sleep 2; done
[[ "$healthy" == "1" ]] && ok "Service serves /health" || { bad "/health unreachable"; cat /tmp/pensieve-roles-pf.log; exit 1; }

section "Admin login"
TOKEN="$(curl -fsS -X POST "$BASE/v1/auth/login" -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"pensieve_dev"}' 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin).get("access_token",""))' 2>/dev/null)"
[[ -n "$TOKEN" ]] && ok "logged in" || bad "no token"
AUTH=(-H "Authorization: Bearer $TOKEN")

section "Staged path: edge stages 3 ingests → committer commits (poll for visibility)"
codes=""; for r in 1 2 3; do codes="$codes $(stage_one "$r")"; done
echo "  ingest HTTP codes:$codes"
visible=0
for i in $(seq 1 30); do
  c="$(count_rows)"; if [[ "$c" == "3" ]]; then visible=1; break; fi; sleep 1
done
[[ "$visible" == "1" ]] && ok "3 staged rows committed + visible (staged ingest → committer)" \
  || bad "staged rows never became visible (last count=$(count_rows))"

section "Committer failover: delete the committer pod, ingest more, expect commits resume"
kubectl delete pod -n "$NS" -l pensieve.io/role=committer --grace-period=0 --force >/dev/null 2>&1 || true
codes=""; for r in 4 5; do codes="$codes $(stage_one "$r")"; done
echo "  ingest HTTP codes:$codes"
recovered=0
for i in $(seq 1 45); do
  c="$(count_rows)"; if [[ "$c" == "5" ]]; then recovered=1; break; fi; sleep 1
done
[[ "$recovered" == "1" ]] && ok "fresh committer took the lease + drained to COUNT=5 (failover)" \
  || bad "commits did not resume after committer kill (last count=$(count_rows))"

section "Summary"
printf "PASS: %d  FAIL: %d\n" "$pass" "$fail"
if [[ "$fail" -eq 0 ]]; then printf "${GRN}k8s role-split: GREEN${NC}\n"; exit 0
else printf "${RED}k8s role-split: FAILED${NC}\n"; exit 1; fi
