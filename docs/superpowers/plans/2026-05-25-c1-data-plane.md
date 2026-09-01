# C1 — Data Plane on Managed Infra Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a single, shared (one-tenant) Pensieve data plane entirely on managed infrastructure — the stock `pensieve-bin` container on Railway, the Postgres catalog on Supabase project B, extents on AWS S3 — and **measure the cross-cloud hot-path latency and cost envelope**. This phase exists to de-risk: it produces *numbers* that gate every downstream phase. No multi-tenancy, no gateway, no dashboard yet.

**Architecture:** Reuse the existing multi-stage `Dockerfile` (already builds `pensieve` + `pensieve-cli` + web-ui) unchanged where possible — the engine is already designed to run on Railway (see its bootstrap-script comments). Point it at a real Supabase Postgres (catalog) and a real AWS S3 bucket (extents) instead of the local docker-compose `postgres` + `minio`. Switch the engine's S3 client from MinIO mode (`PENSIEVE_S3_ENDPOINT` set, `PATH_STYLE=true`, `ALLOW_HTTP=true`) to AWS mode (endpoint unset, `PATH_STYLE=false`, `ALLOW_HTTP=false`, creds via static keys for C1, STS in C2). Run the repo's existing `scripts/test-*.sh` and `scripts/perf-baseline.sh` against the deployed endpoint, capturing wall-clock latency and S3 request/byte counts. Everything is provisioned by the `infra/envs/dev` Terraform from C0 plus a Railway service definition.

**Tech Stack:** Existing `Dockerfile`. Railway (engine host). Supabase project B (catalog Postgres). AWS S3 (extents). Bash + curl + the existing test scripts for verification. CloudWatch / S3 server access logs (or S3 request metrics) for cost measurement. No application code changes to the engine; if a change is unavoidable it is a config/env change only.

**Prerequisite:** C0 complete — `infra/envs/dev` validates, bootstrap state backend live, CI can apply via OIDC, `s3-extent-bucket` + `supabase-project` modules exist.

**File Structure (created/modified across this plan):**

```
infra/envs/dev/
  main.tf                  + railway engine service, wired to catalog + bucket
docker/
  engine.railway.env.example   the AWS-mode env var set for the engine on Railway
scripts/cloud/
  c1-deploy-check.sh       health + smoke against the deployed endpoint
  c1-measure-latency.sh    p50/p99 of ingest + query against managed infra
  c1-measure-cost.sh       pull S3 request/byte counts for a fixed workload
docs/cloud/
  c1-envelope.md           THE DELIVERABLE: measured latency + cost, go/no-go
```

---

## Task 1: Pin the engine image for Railway deploy

**Objective:** Get a tagged, pullable engine image Railway can deploy, without changing engine code.

**Files:**
- Create: `docker/engine.railway.env.example`

- [ ] **Step 1: Confirm the image builds against the documented toolchain**

Run:
```bash
docker build -t pensieve-engine:c1 .
```
Expected: image builds (multi-stage: web → server → runtime). Note: `Dockerfile` pins `rust:1.84-bookworm` while `README.md` says rust 1.95+. If the build fails on toolchain, bump the `FROM rust:` line to match `rust-toolchain.toml` — that is the *only* permitted Dockerfile edit in C1, and it is a build fix, not a behavior change.

- [ ] **Step 2: Decide image distribution** — Railway can build from the repo (Nixpacks/Dockerfile) or pull from a registry. Choose **build-from-Dockerfile** so Railway tracks the repo. Record the choice in `docs/cloud/decisions.md` as a new dated row.

- [ ] **Step 3: Write the AWS-mode env template `docker/engine.railway.env.example`**

```bash
# Engine env for Railway against Supabase (catalog) + AWS S3 (extents).
# Catalog — Supabase project B pooled connection string (port 6543, pgbouncer).
PENSIEVE_CATALOG_URL=postgres://postgres.<ref>:***@<region>.pooler.supabase.com:6543/postgres

# Object store — REAL AWS S3 (not MinIO):
#   - endpoint UNSET  → AWS S3
#   - PATH_STYLE false → virtual-hosted addressing (AWS default)
#   - ALLOW_HTTP false → require TLS
PENSIEVE_S3_BUCKET=pensieve-dev-extents
PENSIEVE_S3_REGION=us-east-1
PENSIEVE_S3_PATH_STYLE=false
PENSIEVE_S3_ALLOW_HTTP=false
PENSIEVE_PATH_PREFIX=pensieve
# C1 uses STATIC keys for a dedicated IAM user scoped to the bucket.
# C2 replaces these with per-request STS session creds via the gateway.
PENSIEVE_S3_ACCESS_KEY_ID=${AWS_S3_ACCESS_KEY_ID}
    PENSIEVE_S3_SECRET_ACCESS_KEY = aws_iam_access_key.c1_engine.secret

# Listeners
PENSIEVE_HTTP_ADDR=0.0.0.0:8080
PENSIEVE_GRPC_ADDR=0.0.0.0:9090
PENSIEVE_OTLP_ADDR=0.0.0.0:4317
PENSIEVE_OTLP_DATABASE=default

# Single shared auth token for C1 (real auth arrives with the gateway in C2).
PENSIEVE_AUTH_TOKENS=${C1_ENGINE_TOKEN}:admin
RUST_LOG=info,sqlx=warn
```

- [ ] **Step 4: Commit**

```bash
git add docker/engine.railway.env.example
git commit -m "infra(c1): engine env template for railway against supabase+s3"
```

---

## Task 2: Provision the Supabase catalog (project B) and apply S3 bucket

**Objective:** Get a real catalog DB and a real extent bucket from the C0 modules.

**Files:**
- Modify: `infra/envs/dev/main.tf`

- [ ] **Step 1: Ensure `envs/dev` instantiates the catalog + bucket** (from C0 Task 10 these exist; confirm region + names)

```hcl
module "extents" {
  source      = "../../modules/s3-extent-bucket"
  bucket_name = "pensieve-dev-extents"
  env         = "dev"
}

module "catalog" {           # Supabase project B — engine catalog
  source  = "../../modules/supabase-project"
  name    = "pensieve-dev-catalog"
  org_id  = var.supabase_org_id
  region  = "us-east-1"       # confirm Supabase region id maps to us-east-1
  db_pass = var.supabase_db_pass
}
```

- [ ] **Step 2: Create a dedicated S3 IAM user for C1 (static keys, scoped to the bucket)**

Add to `main.tf` (C1-only; deleted in C2 when STS replaces it):
```hcl
resource "aws_iam_user" "c1_engine" { name = "pensieve-dev-c1-engine" }

data "aws_iam_policy_document" "c1_bucket" {
  statement {
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${module.extents.bucket_arn}/*"]
  }
  statement {
    actions   = ["s3:ListBucket"]
    resources = [module.extents.bucket_arn]
  }
}
resource "aws_iam_user_policy" "c1_bucket" {
  user   = aws_iam_user.c1_engine.name
  policy = data.aws_iam_policy_document.c1_bucket.json
}
resource "aws_iam_access_key" "c1_engine" { user = aws_iam_user.c1_engine.name }

output "c1_engine_access_key_id" { value = aws_iam_access_key.c1_engine.id }
output "c1_engine_secret"        { value = aws_iam_access_key.c1_engine.secret, sensitive = true }
```

- [ ] **Step 2 fmt note:** the `output` with two attributes must be split into block form:
```hcl
output "c1_engine_secret" {
  value     = aws_iam_access_key.c1_engine.secret
  sensitive = true
}
```

- [ ] **Step 3: Plan + apply via CI** (open PR touching `infra/envs/dev`, merge to trigger `infra-apply`)

Run (verify after merge):
```bash
cd infra/envs/dev && terraform output -raw c1_engine_access_key_id
```
Expected: an access key id prints. Bucket + Supabase catalog now exist.

- [ ] **Step 4: Bootstrap the catalog schema** (one-shot, using the image's `pensieve-bootstrap.sh` OR engine auto-migrate on first boot)

Run:
```bash
docker run --rm \
  -e PENSIEVE_CATALOG_URL="$SUPABASE_CATALOG_URL" \
  --entrypoint /usr/local/bin/pensieve-bootstrap.sh pensieve-engine:c1
```
Expected: "Pensieve database ready" + table list. Confirms the engine can reach Supabase and create its catalog.

- [ ] **Step 5: Commit**

```bash
git add infra/envs/dev/main.tf
git commit -m "infra(c1): catalog (supabase B) + extent bucket + scoped IAM user"
```

---

## Task 3: Deploy the engine to Railway

**Objective:** Engine running on Railway, talking to Supabase catalog + AWS S3, reachable over HTTPS.

**Files:**
- Modify: `infra/envs/dev/main.tf` (uncomment the `railway-service` engine block from C0)

- [ ] **Step 1: Wire the engine Railway service**

```hcl
module "engine" {
  source     = "../../modules/railway-service"
  project_id = var.railway_project_id
  name       = "pensieve-engine-dev"
  repo       = var.repo_url          # build from Dockerfile
  region     = "us-east"             # Railway region nearest us-east-1 (confirm)
  replicas   = 1
  env_vars = {
    PENSIEVE_CATALOG_URL          = module.catalog.database_url
    PENSIEVE_S3_BUCKET            = module.extents.bucket_name
    PENSIEVE_S3_REGION            = "us-east-1"
    PENSIEVE_S3_PATH_STYLE        = "false"
    PENSIEVE_S3_ALLOW_HTTP        = "false"
    PENSIEVE_PATH_PREFIX          = "pensieve"
    PENSIEVE_S3_ACCESS_KEY_ID     = aws_iam_access_key.c1_engine.id
    PENSIEVE_S3_SECRET_ACCESS_KEY = aws_iam_access_key.c1_engine.secret
    PENSIEVE_HTTP_ADDR            = "0.0.0.0:8080"
    PENSIEVE_GRPC_ADDR            = "0.0.0.0:9090"
    PENSIEVE_OTLP_ADDR            = "0.0.0.0:4317"
    PENSIEVE_AUTH_TOKENS          = "${var.c1_engine_token}:admin"
    RUST_LOG                  = "info,sqlx=warn"
  }
}

output "engine_url" { value = module.engine.public_domain }
```

- [ ] **Step 2: Apply + wait for healthy**

Run:
```bash
ENGINE_URL=$(terraform -chdir=infra/envs/dev output -raw engine_url)
curl -sS "https://$ENGINE_URL/health"
```
Expected: health endpoint returns OK. (Railway terminates TLS; engine speaks plain HTTP behind it. Flight gRPC on 9090 needs a TCP/HTTP2 proxy — see Step 3.)

- [ ] **Step 3: Confirm Flight gRPC exposure**

Railway's HTTP proxy may not pass HTTP/2 gRPC cleanly. Check whether the engine's Flight port is reachable. If Railway can't proxy gRPC for the public domain, note it in `c1-envelope.md` as a finding (it directly affects C3's Flight endpoint design — the gateway may need to terminate Flight). Do **not** work around it here; C1 measures and reports, C2/C3 design around it.

Run:
```bash
# from a host with the engine's grpc reachable (or via railway tcp proxy)
grpcurl -plaintext "$ENGINE_GRPC_HOST:443" list 2>&1 | head
```
Expected: either a Flight service listing (works) or a documented failure (recorded as a finding).

- [ ] **Step 4: Commit**

```bash
git add infra/envs/dev/main.tf
git commit -m "infra(c1): deploy pensieve engine to railway"
```

---

## Task 4: Smoke test against the deployed endpoint

**Objective:** Prove ingest → query works end-to-end on managed infra using the repo's own tests.

**Files:**
- Create: `scripts/cloud/c1-deploy-check.sh`

- [ ] **Step 1: Write `scripts/cloud/c1-deploy-check.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
: "${ENGINE_URL:?set ENGINE_URL (https://...)}"
: "${C1_ENGINE_TOKEN:?set C1_ENGINE_TOKEN}"
AUTH=(-H "Authorization: Bearer $C1_ENGINE_TOKEN")

echo "== health =="
curl -fsS "${AUTH[@]}" "$ENGINE_URL/health"

echo "== ingest 2 rows =="
curl -fsS -X POST "$ENGINE_URL/v1/ingest" "${AUTH[@]}" \
  -H "X-Database: obs" -H "X-Table: otel_logs" \
  -H "Content-Type: application/x-ndjson" --data-binary @- <<'EOF'
{"timestamp":"2026-04-20T14:23:01Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
{"timestamp":"2026-04-20T14:23:04Z","service.name":"payments-svc","severity_text":"ERROR","message":"card declined","attributes":{"error.code":"CARD_DECLINED"}}
EOF

echo "== query (KQL) =="
curl -fsS -X POST "$ENGINE_URL/v1/query" "${AUTH[@]}" \
  -H "X-Database: obs" -H "Content-Type: application/x-kql" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | summarize n = count() by error_code = tostring(attributes["error.code"])'
echo
echo "C1 deploy-check OK"
```

- [ ] **Step 2: Run it**

Run:
```bash
chmod +x scripts/cloud/c1-deploy-check.sh
ENGINE_URL="https://$(terraform -chdir=infra/envs/dev output -raw engine_url)" \
  C1_ENGINE_TOKEN="$C1_ENGINE_TOKEN" scripts/cloud/c1-deploy-check.sh
```
Expected: health OK, ingest accepted, query returns `n=2` for `CARD_DECLINED`. This proves the full Railway→Supabase→S3 round trip.

- [ ] **Step 3: Run the existing repo test suite against the deployed endpoint**

Most `scripts/test-*.sh` target `localhost:8080`. Run the ones that accept a base-URL override (or set the env they read) against `$ENGINE_URL`:
```bash
PENSIEVE_BASE_URL="$ENGINE_URL" bash scripts/test-kql.sh
PENSIEVE_BASE_URL="$ENGINE_URL" bash scripts/test-otlp.sh   # if OTLP enabled
```
Expected: pass. If a script hardcodes localhost, note it; do not refactor the suite in C1 (out of scope) — just record which tests are deploy-portable.

- [ ] **Step 4: Commit**

```bash
git add scripts/cloud/c1-deploy-check.sh
git commit -m "test(c1): deploy smoke check against managed endpoint"
```

---

## Task 5: Measure latency (the hot path)

**Objective:** Quantify ingest + query latency on managed infra and isolate the cross-cloud catalog/S3 contribution.

**Files:**
- Create: `scripts/cloud/c1-measure-latency.sh`

- [ ] **Step 1: Write `scripts/cloud/c1-measure-latency.sh`** — fire N ingests then N queries, report p50/p99 via curl `-w`

```bash
#!/usr/bin/env bash
set -euo pipefail
: "${ENGINE_URL:?}"; : "${C1_ENGINE_TOKEN:?}"
AUTH=(-H "Authorization: Bearer $C1_ENGINE_TOKEN")
N="${N:-200}"
Q='otel_logs | where severity_text == "ERROR" | summarize n=count() by error_code = tostring(attributes["error.code"])'

probe() { # $1=label  reads stdin body args
  local label="$1"; shift
  local f; f=$(mktemp)
  for i in $(seq 1 "$N"); do
    curl -o /dev/null -s -w "%{time_total}\n" "$@" >> "$f"
  done
  echo "== $label (n=$N) =="
  sort -n "$f" | awk '{a[NR]=$1} END{print "p50="a[int(NR*0.5)]"s p90="a[int(NR*0.9)]"s p99="a[int(NR*0.99)]"s"}'
  rm -f "$f"
}

probe "query-warm" -X POST "$ENGINE_URL/v1/query" "${AUTH[@]}" \
  -H "X-Database: obs" -H "Content-Type: application/x-kql" --data-binary "$Q"
```

- [ ] **Step 2: Run warm (cache populated) and cold (after a fresh deploy / cache flush)**

Run:
```bash
chmod +x scripts/cloud/c1-measure-latency.sh
N=200 ENGINE_URL="https://$(terraform -chdir=infra/envs/dev output -raw engine_url)" \
  C1_ENGINE_TOKEN="$C1_ENGINE_TOKEN" scripts/cloud/c1-measure-latency.sh | tee /tmp/c1-latency-warm.txt
```
Expected: numbers captured. Repeat after redeploying the engine (cold block cache) to get the cold-query number that hits S3 footers.

- [ ] **Step 3: Run the repo's own perf baseline against managed infra for an apples-to-apples vs. local**

Run:
```bash
PENSIEVE_BASE_URL="$ENGINE_URL" bash scripts/perf-baseline.sh | tee /tmp/c1-perf-baseline.txt || true
```
Expected: a baseline captured (or documented why the script needs local). Compare to the committed local baseline under `scripts/fixtures/perf-baseline/`.

- [ ] **Step 4: Commit the script**

```bash
git add scripts/cloud/c1-measure-latency.sh
git commit -m "test(c1): latency probe for managed hot path"
```

---

## Task 6: Measure cost (S3 requests + bytes for a fixed workload)

**Objective:** Establish cost-per-GB-ingested and cost-per-query in S3 request/byte terms — the dominant variable cost at "many many volumes."

**Files:**
- Create: `scripts/cloud/c1-measure-cost.sh`

- [ ] **Step 1: Enable S3 request metrics on the bucket** (Terraform, temporary for C1)

Add to `infra/envs/dev/main.tf`:
```hcl
resource "aws_s3_bucket_metric" "c1" {
  bucket = module.extents.bucket_name
  name   = "c1-request-metrics"
}
```
Apply via CI.

- [ ] **Step 2: Write `scripts/cloud/c1-measure-cost.sh`** — runs a fixed workload, then pulls S3 request counts from CloudWatch for the window

```bash
#!/usr/bin/env bash
set -euo pipefail
: "${ENGINE_URL:?}"; : "${C1_ENGINE_TOKEN:?}"; : "${BUCKET:?}"
AUTH=(-H "Authorization: Bearer $C1_ENGINE_TOKEN")
START=$(date -u +%s)

# Fixed workload: ingest a known volume, then run a fixed query set K times.
echo "ingesting fixed seed..."
curl -fsS -X POST "$ENGINE_URL/v1/ingest" "${AUTH[@]}" \
  -H "X-Database: obs" -H "X-Table: otel_logs" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @scripts/fixtures/perf-baseline/seed.ndjson
echo "running query set..."
for i in $(seq 1 50); do
  while read -r q; do
    curl -o /dev/null -s -X POST "$ENGINE_URL/v1/query" "${AUTH[@]}" \
      -H "X-Database: obs" -H "Content-Type: application/x-kql" --data-binary "$q"
  done < scripts/fixtures/perf-baseline/queries.txt
done

sleep 120  # let CloudWatch S3 request metrics populate
END=$(date -u +%s)
for m in GetRequests PutRequests BytesDownloaded BytesUploaded; do
  echo -n "$m="
  aws cloudwatch get-metric-statistics \
    --namespace AWS/S3 --metric-name "$m" \
    --dimensions Name=BucketName,Value="$BUCKET" Name=FilterId,Value=c1-request-metrics \
    --start-time "$(date -u -r $START +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d @$START +%Y-%m-%dT%H:%M:%SZ)" \
    --end-time   "$(date -u -r $END   +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d @$END   +%Y-%m-%dT%H:%M:%SZ)" \
    --period 300 --statistics Sum --query 'Datapoints[].Sum' --output text
done
```

- [ ] **Step 3: Run it and capture raw counts**

Run:
```bash
chmod +x scripts/cloud/c1-measure-cost.sh
ENGINE_URL="https://$(terraform -chdir=infra/envs/dev output -raw engine_url)" \
  C1_ENGINE_TOKEN="$C1_ENGINE_TOKEN" BUCKET="pensieve-dev-extents" \
  scripts/cloud/c1-measure-cost.sh | tee /tmp/c1-cost.txt
```
Expected: GET/PUT request counts and bytes for the fixed workload. Convert to dollars using current S3 pricing ($/1k GET, $/1k PUT, $/GB egress) in the envelope doc.

- [ ] **Step 4: Commit**

```bash
git add scripts/cloud/c1-measure-cost.sh infra/envs/dev/main.tf
git commit -m "test(c1): s3 request/byte cost measurement for fixed workload"
```

---

## Task 7: Write the envelope deliverable + go/no-go

**Objective:** The whole point of C1 — a decision document with real numbers that gates downstream phases.

**Files:**
- Create: `docs/cloud/c1-envelope.md`

- [ ] **Step 1: Write `docs/cloud/c1-envelope.md`**

````markdown
# C1 — Measured Cross-Cloud Envelope

**Date:** <fill>  **Region:** AWS us-east-1 / Supabase <region> / Railway <region>
**Engine image:** <git sha>  **Topology:** Railway engine → Supabase B catalog → AWS S3 extents

## Latency (measured)

| Path | p50 | p90 | p99 | Notes |
|---|---|---|---|---|
| Query, warm cache | | | | block cache hot |
| Query, cold cache | | | | hits S3 footers |
| Ingest (single request) | | | | group-commit window applies |
| Local baseline (for ref) | | | | from scripts/fixtures/perf-baseline |

**Cross-cloud overhead:** managed p99 minus local p99 = <X> ms. Attribution:
catalog RTT (Railway↔Supabase) ≈ <ms>; S3 footer GET ≈ <ms>.

## Cost (measured, fixed workload)

| Metric | Count | $ |
|---|---|---|
| S3 GET (query set ×50) | | |
| S3 PUT (ingest) | | |
| Bytes downloaded | | |
| Bytes uploaded (egress) | | |

Derived: **cost per 1M ingested rows ≈ $<>**, **cost per 1k queries ≈ $<>**.

## Findings
- Flight gRPC over Railway public proxy: <works / does not — implication for C3>.
- Supabase pooled vs direct connection on catalog latency: <finding>.
- Pruning effectiveness on managed infra: <% extents skipped> (from engine metrics).

## Go / No-Go for downstream phases

| Gate | Threshold | Measured | Verdict |
|---|---|---|---|
| Query p99 warm | < 250 ms | | |
| Query p99 cold | < 1 s | | |
| Cross-cloud overhead | < 100 ms p99 | | |
| Cost per 1k queries | < $<budget> | | |

**Decision:** <PROCEED to C2 as designed / PROCEED but accelerate data-plane→AWS move (C5) / REDESIGN co-location>.
````

- [ ] **Step 2: Fill it with the captured numbers** from `/tmp/c1-*.txt` and engine `/metrics` (pruning ratios).

- [ ] **Step 3: Make the explicit go/no-go call** and, if "accelerate AWS move," open a note in the master design's C5 stub.

- [ ] **Step 4: Commit**

```bash
git add docs/cloud/c1-envelope.md
git commit -m "docs(c1): measured cross-cloud latency+cost envelope and go/no-go"
```

---

## Task 8: Phase exit checklist

- [ ] Engine deployed on Railway, healthy, reachable over HTTPS.
- [ ] Catalog on Supabase B; extents in AWS S3 `pensieve-dev-extents`; full ingest->query round trip green (`c1-deploy-check.sh`).
- [ ] Repo test suite portability noted (which `test-*.sh` run against a remote base URL).
- [ ] Latency measured (warm + cold) and compared to local baseline.
- [ ] Cost measured (S3 requests/bytes -> $/1M rows, $/1k queries).
- [ ] Flight-over-Railway finding recorded (feeds C3 endpoint design).
- [ ] `docs/cloud/c1-envelope.md` has a written PROCEED / REDESIGN decision.

- [ ] **Mark C1 complete in the master design**

Add under the C1 appendix stub in `docs/superpowers/specs/2026-05-25-pensieve-cloud-platform-design.md`:
`**Status:** C1 complete — envelope in docs/cloud/c1-envelope.md; decision: <verdict>`

```bash
git add docs/superpowers/specs/2026-05-25-pensieve-cloud-platform-design.md
git commit -m "docs(cloud): mark C1 complete with envelope verdict"
```

---

## Notes for the implementer

- **C1 measures; it does not optimize.** If a number is bad, record it and surface the decision (accelerate the AWS move, change region pinning). Do not start tuning the engine — that is not this phase.
- **The S3 mode flip is the most error-prone step.** For real AWS S3: `PENSIEVE_S3_ENDPOINT` UNSET, `PENSIEVE_S3_PATH_STYLE=false`, `PENSIEVE_S3_ALLOW_HTTP=false`. Local docker-compose uses the opposite of all three. Getting one wrong fails silently as connection or signing errors.
- **Static IAM keys are a C1-only crutch.** They exist so we measure without building the gateway first. C2 deletes the `aws_iam_user.c1_engine` resources and replaces them with per-request STS. Leave a `# TODO(C2): delete, replaced by STS` on that block.
- **Supabase connection string:** prefer the **pooled** (pgbouncer, port 6543) string for the engine's many short catalog queries; record whether direct (5432) is meaningfully faster in the envelope.
- **Region pinning is the headline mitigation** (master design section 3). If Railway's nearest region isn't co-located with `us-east-1`, that shows up directly in the cross-cloud overhead number — call it out.
- **Don't add the gateway, tenancy, dashboard, or billing here.** One shared tenant, one token, one bucket prefix. The point is a clean measurement, not a product.
