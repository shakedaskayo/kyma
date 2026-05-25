# Kyma Cloud — Managed Multi-Tenant SaaS — Master Design

**Status:** draft 2026-05-25. Decomposes into 7 sub-specs (C0–C6). Each sub-spec gets its own implementation plan under `docs/superpowers/plans/`.

**Relationship to other specs:** This builds *on top of* the engine. It depends on `2026-05-19-kyma-v1-production-readiness-design.md`, especially **A4 (multi-tenancy + security)** and **A2 (MCP surface)**. Where the engine's A4 isolation is not yet landed, the Cloud **gateway** compensates (defense in depth); see §6 dependency note.

---

## 1. Goal & non-goals

### Goal

Kyma Cloud is a **managed, multi-tenant SaaS** where a developer can: sign up, create a project, point their stack's OTLP/REST telemetry at a Kyma Cloud endpoint, and **wire their telemetry into Claude Code in under a minute** via a copy-paste `claude mcp add` command — then have their agent ask production questions in KQL/SQL over Arrow Flight. No infrastructure for the customer to run.

The platform is **end-to-end**: customer-facing dashboard + auth, signup/onboarding, API-key issuance, ingest + query endpoints, the engine fleet behind them, usage metering, and usage-based billing — all reproducible from **Infrastructure-as-Code**.

Concretely, v1 delivers the **"data in → answers in Claude Code"** loop:

1. **Data in:** dashboard hands the customer an OTLP/REST endpoint + token; they point their stack at it and rows land.
2. **Answers out:** dashboard hands the customer a one-line `claude mcp add kyma --transport http https://<project>.mcp.getkyma.cloud --header "Authorization: Bearer kyma_…"`; their agent queries telemetry the same afternoon.

### Tenancy decision (the hinge)

**Launch pooled, keep a one-click silo path.** (AWS SaaS pool/bridge/silo model.)

- **Free / Pro tiers → Pooled.** One engine fleet; tenants separated by Kyma-native `X-Database` + per-tenant S3 key prefix. Cross-tenant isolation enforced by **scoped STS session credentials** (a request scoped to tenant A *cannot* GET tenant B's prefix) + **hard query budgets / rate limits** for noisy-neighbor control. Best density and cost.
- **Enterprise tier → Silo.** Dedicated engine service(s) + dedicated bucket (and optionally dedicated AWS account / Supabase project / region for data residency). Best blast-radius isolation + compliance.

The migration pool→silo is a **deployment + data-repoint change, not a rewrite** — this is Kyma's own architectural promise applied to tenancy. The design's job is to keep that promise cheap (§6, C6).

### Non-goals for Cloud v1

- On-prem / BYO-cloud / self-hosted control plane (Cloud is hosted-only at v1).
- Per-signup Terraform applies (pooled tenants are provisioned by runtime app logic; only **silo** tenants touch Terraform — §5).
- Multi-region active-active control plane (single primary region + DR at v1).
- A query GUI beyond a minimal console (the product surface is the **MCP/Flight/REST APIs** and Claude Code, not dashboards-to-click).
- SOC 2 certification at v1 (we build **SOC-2-ready** controls; certification is a fast-follow).
- Replacing the OSS engine's own tenancy/security work — Cloud consumes A4, it does not fork it.

---

## 2. System architecture

Two planes, mirroring the engine's own ingest/query split.

```
                         ┌─────────────────────────────────────────┐
   Customer browser ───► │  kyma-cloud-dashboard (Next.js, Railway) │
                         │  Supabase Auth · usage · billing portal  │
                         └───────────────┬─────────────────────────┘
                                         │  control-plane API
                         ┌───────────────▼─────────────────────────┐
                         │  kyma-cloud-api (control plane, Railway) │
                         │  orgs · projects · api_keys · usage ·    │
                         │  Stripe webhooks · silo provisioning     │
                         └──────┬───────────────────────┬──────────┘
                                │                        │
              Supabase project A│ (control-plane DB)     │ Stripe (billing/metering)
                                │                        │
   ── DATA PLANE ──────────────┼────────────────────────┼─────────────────────────
                                │                        │
   Customer stack / agent       ▼                        ▼
   (OTLP, REST, Flight, MCP) ► ┌──────────────────────────────────┐
                               │  kyma-gateway (auth proxy, Railway)│
                               │  authN api_key → tenant → X-Database│
                               │  mints scoped STS + engine token   │
                               │  quotas · rate limit · usage meter  │
                               └──────────────┬─────────────────────┘
                                              │ (X-Database, scoped creds)
                               ┌──────────────▼─────────────────────┐
                               │  kyma-bin engine fleet (Railway)    │   ← portable to AWS at scale (§6)
                               │  HTTP 8080 · Flight 9090 · MCP      │
                               └───────┬──────────────────┬─────────┘
                                       │                  │
                    Supabase project B │ (engine catalog) │ AWS S3 (extents, source of truth)
                                       ▼                  ▼
```

### Component inventory

| Component | Tech | Host | Role |
|---|---|---|---|
| `kyma-cloud-dashboard` | Next.js + Supabase Auth | Railway | Signup, onboarding, API keys, MCP snippet, usage, billing portal, minimal query console |
| `kyma-cloud-api` | TypeScript (Node) — see §7 decision | Railway | Control-plane CRUD, key issuance, Stripe webhooks, silo provisioning trigger |
| `kyma-gateway` | Rust (reuse engine crates) — see §7 | Railway | Per-request authN, tenant resolution, scoped-credential minting, quotas, usage metering. Keeps the OSS engine clean. |
| `kyma-bin` engine fleet | Existing Rust binary, containerized | Railway (→ AWS at scale) | Unmodified OSS engine; ingest + query + MCP |
| Control-plane DB | Supabase project **A** | Supabase (on AWS) | orgs, projects, api_keys, usage_events, subscriptions, audit_log |
| Engine catalog | Supabase project **B** | Supabase (on AWS) | Kyma's Postgres catalog (manifests, stats, CAS) |
| Object storage | AWS S3 | AWS | Extents — the source of truth |
| Billing | Stripe | SaaS | Products, metered prices, invoices |
| DNS / edge | Cloudflare | SaaS | `getkyma.cloud`, per-project subdomains, TLS |

**Why two Supabase projects (A and B), not one:** the control-plane DB and the engine catalog have different blast radii, backup cadences, access roles, and failure semantics. A runaway catalog migration or a catalog restore must not touch billing/identity data, and vice-versa. Separate projects (or, at minimum, separate databases + roles) keep the failure domains independent. (§7 decision.)

**Why a gateway in front of the engine:** all multi-tenant *policy* (key auth, tenant→database mapping, scoped-credential minting, quota/rate enforcement, usage capture) lives in `kyma-gateway`. The engine stays the stock OSS `kyma-bin`. This means (a) the OSS engine never grows SaaS-specific code, (b) tenancy bugs are fixed in one place, (c) defense-in-depth: gateway enforces isolation *and* the engine enforces its own A4 isolation.

---

## 3. Cross-cloud topology — the critical risk

The hot query path touches **three clouds**: Railway (compute) → Supabase/AWS (catalog, hit on *every* query for pruning) → AWS S3 (extents). Latency and egress on this path is the single biggest engineering risk.

**Mitigations, locked at design time:**

1. **Region co-location.** Supabase runs on AWS. Pin Supabase region, Railway region, and the S3 bucket to the **same AWS region/metro** (e.g. all `us-east-1`). Railway↔AWS stays cross-cloud but same-metro → single-digit-ms RTT. C1 measures and confirms the actual number before we build on it.
2. **Catalog on the hot path → cache + co-locate.** Pruning queries the catalog constantly; co-location plus the engine's block cache and the gateway's tenant-metadata cache keep it cheap.
3. **S3 request/egress cost dominates at "many many volumes."** Lean hard on what the engine already does — fat extents (group-commit) + 3-level pruning (99%+ extents skipped) + on-node block cache — so we issue few, large GETs. C1 establishes the cost-per-ingested-GB and cost-per-query envelope.
4. **Data-plane portability (the scale inflection).** Railway is the right home for the control plane and the *v1* data plane. But the data plane has gravitational pull toward **AWS, next to S3** (kill cross-cloud egress, co-locate compute with storage). Therefore: containerize the engine as a strict 12-factor, config-only image with **zero Railway-specific assumptions**, so moving engine pods from Railway → AWS ECS/Fargate/EKS is a deployment change, not a rewrite. C5 defines the trigger metric (egress $ / month, or p99 regression) that flips the data plane to AWS.

---

## 4. Tenant lifecycle & isolation

### Pooled tenant (Free/Pro) — runtime provisioning, no Terraform

On project creation, `kyma-cloud-api` performs an **idempotent, transactional** sequence:

1. Insert `org`/`project` rows (control-plane DB, project A).
2. Allocate a tenant identity: `tenant_id`, Kyma `X-Database` name, S3 key prefix `s3://kyma-prod-extents/<tenant_id>/`.
3. Register the database namespace in the engine catalog (project B).
4. Mint an **ingest token** and a **query/MCP token** (`kyma_…`), store only hashes.
5. Surface endpoints in the dashboard: OTLP/REST ingest URL, Flight URL, MCP URL, all carrying `<project>` routing.

No bucket, no IAM role, no Terraform apply per signup — those are pre-provisioned shared resources. **Isolation** is enforced at request time:

- Gateway authenticates the API key → resolves `tenant_id` → injects `X-Database` and an **STS `AssumeRole` session scoped by a session policy** to that tenant's prefix only. Even a compromised/buggy engine path cannot read another tenant's extents with that credential.
- Gateway applies the tenant's plan **quota + rate limit** (noisy-neighbor).
- Engine additionally enforces A4 tenant-segmented paths (defense in depth).

### Silo tenant (Enterprise) — Terraform-provisioned

`kyma-cloud-api` triggers a CI **Terraform apply** of the `tenant-silo` module (§5): dedicated engine service(s), dedicated bucket (optionally dedicated region/account), optional dedicated Supabase. Control-plane DB records the tenant as `isolation=silo` and routes its keys to the dedicated stack. Same key/endpoint UX as pooled — only the backing infra differs.

### Pool → silo migration (C6)

Quiesce ingest → snapshot catalog namespace + copy S3 prefix to the silo bucket → repoint the tenant's routing in the control-plane DB → resume. Tested as a gauntlet-style scenario (no data loss, identical query results before/after).

---

## 5. Infrastructure as Code

**Tooling:** **Terraform (OpenTofu-compatible).** Both Railway and Supabase and Stripe have providers; AWS is first-class. (C0 task: confirm provider maturity for Railway + Supabase + Stripe and pin versions; fall back to provider-specific config-as-code where a TF provider is thin.)

**State & auth:** remote state in an S3 backend + DynamoDB lock in a dedicated **ops/bootstrap AWS account**. CI authenticates to AWS via **GitHub OIDC** (no long-lived keys). Railway/Supabase/Stripe tokens live in GH Actions secrets + a secrets manager (AWS Secrets Manager or Doppler). **No secrets in state.**

**Layout:**

```
infra/
  bootstrap/                 TF state backend, GitHub OIDC, base accounts
  modules/
    s3-extent-bucket/        bucket + lifecycle + versioning + replication
    iam-scoped-role/         STS role + per-tenant session-policy template
    railway-service/         one engine/gateway/api/dashboard service
    supabase-project/        a Supabase project (A control-plane or B catalog)
    stripe-product/          product + metered prices for a plan tier
    tenant-silo/             composed dedicated stack for one enterprise tenant
  envs/
    dev/  staging/  prod/    compose modules into a full environment
  tenants/                   instantiations of tenant-silo (one dir/workspace per silo tenant)
```

**Rule:** pooled tenants are **app logic** (runtime, transactional). Only **silo** tenants and **environments** are Terraform. Never `terraform apply` on a signup.

**Environments:** `dev`, `staging`, `prod` — each a full stack (Railway project/env + Supabase A & B + S3 bucket + Stripe test/live). Promotion via CI.

---

## 6. Dependencies & sequencing

```
                C0 Foundations (IaC, accounts, regions, CI, decisions)
                          │
                C1 Data plane on managed infra (single shared tenant, prove topology+cost)
                          │
                C2 Control plane + gateway (pooled tenancy, scoped STS, isolation tests)
                          │
          ┌───────────────┼───────────────┐
   C3 Onboarding +   C4 Metering &    (C5 hardening is
   dashboard + MCP   billing           cross-cutting,
   (the headline)    (Stripe)          starts after C3)
          └───────────────┼───────────────┘
                          │
                C5 Hardening & scale (DR, autoscale, dogfood obs, security, SLOs)
                          │
                C6 Silo & migration (Terraform tenant-silo, pool→silo tooling, residency)
```

**Engine dependency:** C2's pooled isolation leans on engine **A4** (tenant-segmented extent paths) and C3 on engine **A2** (MCP surface). Where A4/A2 are not yet landed when Cloud reaches that phase, `kyma-gateway` compensates (gateway-level prefix scoping + an MCP-over-HTTP shim). Track A2/A4 status at the start of C2/C3 and rebase the gap there — do not assume engine state from today.

| Phase | Rough size | Outputs |
|---|---|---|
| **C0 Foundations** | 2–3 wk | `infra/bootstrap` + `infra/modules` skeletons, OIDC, region/naming decisions, CI apply pipeline, §7 decisions resolved |
| **C1 Data plane** | 2–3 wk | Containerized `kyma-bin`, engine + catalog (Supabase B) + S3 deployed on Railway; e2e ingest+query against managed infra; measured latency + cost envelope |
| **C2 Control plane** | 4–6 wk | Supabase Auth, control-plane schema (A), `kyma-cloud-api`, `kyma-gateway`, pooled tenancy, scoped STS, cross-tenant-denied tests |
| **C3 Onboarding + MCP** | 4–6 wk | Next.js dashboard, signup, project create, key issuance, OTLP/REST + Flight + **MCP endpoints**, one-line `claude mcp add` snippet, minimal query console — the 60-second loop |
| **C4 Metering + billing** | 3–4 wk | Per-tenant usage capture (ingest/storage/query), Stripe products + metered prices, plan quotas enforced at gateway, billing portal |
| **C5 Hardening + scale** | 4–6 wk | Engine autoscaling, noisy-neighbor controls, catalog PITR + S3 versioning/replication DR, **dogfood Kyma-on-Kyma** observability + Grafana, secret rotation, pen-test, SOC-2-ready controls, SLOs + runbooks |
| **C6 Silo + migration** | 3–4 wk | `tenant-silo` Terraform module, control-plane-triggered silo provisioning, pool→silo migration tooling + test, data-residency tenants |

---

## 7. Decisions to lock at C0

| Decision | Recommendation | Rationale |
|---|---|---|
| IaC tool | **Terraform / OpenTofu** | Broadest provider coverage (AWS+Railway+Supabase+Stripe); OIDC + remote state are mature |
| Tenancy at launch | **Pooled, with silo path** | Density/cost now; isolation/compliance when needed; cheap migration |
| Pooled isolation mechanism | **Scoped STS session creds + `X-Database` + gateway quotas** | Strongest practical isolation without per-tenant processes |
| Supabase project split | **Two projects: A control-plane, B catalog** | Independent blast radius, backup cadence, access roles |
| Control-plane language | **TypeScript (Node)** | Native to Supabase/Next.js/Stripe ecosystem; control plane is CRUD/glue, not hot-path |
| Gateway language | **Rust (reuse engine crates)** | Hot path; reuse auth/Flight/token code from `kyma-server`; keep OSS engine unmodified |
| Region | **Single AWS region/metro for Railway + Supabase + S3** | Kill cross-cloud hot-path latency (§3) |
| Provisioning split | **Pooled = app logic; silo + envs = Terraform** | Never terraform-apply per signup |
| Billing meters | **Ingest bytes/rows · storage GB-month · query bytes-scanned/Flight-time** | Maps to real S3 + compute cost; engine already emits these metrics |
| Secrets | **GH OIDC → AWS; tokens in Secrets Manager/Doppler; none in TF state** | Standard supply-chain hygiene |

---

## 8. Risks to monitor during execution

- **Cross-cloud hot-path latency/egress (§3) is the long pole.** If C1's measured envelope is bad, accelerate the data-plane→AWS move (C5) or co-locate sooner. Everything downstream assumes C1's numbers are acceptable.
- **Pooled-process isolation is weaker than customers may assume.** A shared engine process still holds multiple tenants' data in memory; scoped STS protects *storage*, not in-process memory. Be honest in security docs; offer silo for anyone who needs hard isolation. A4 maturity directly bounds how strong pooled isolation can be.
- **Engine A2/A4 timing.** Cloud C2/C3 depend on engine multi-tenancy + MCP. If they lag, the gateway shim grows; track at phase start.
- **Per-signup cost of pooled tenants must stay ~zero.** If anything in tenant creation reaches for Terraform or a new AWS resource, density and signup latency die. Guard the "pooled = app logic only" rule.
- **Metering accuracy = revenue integrity.** Under-count = lost revenue; over-count = angry customers + chargebacks. C4 reconciles gateway-captured usage against engine metrics and S3 inventory; discrepancies alert.
- **Selling observability means holding customers' sensitive telemetry.** Prompt bodies, tool args, and audit trails may contain secrets. TLS everywhere, scoped creds, encryption at rest (S3 SSE), audit logging (dogfooded into Kyma), and a clear data-handling policy are table stakes, not C5 nice-to-haves.
- **DR for the catalog.** The Supabase catalog is metadata-critical: lose it and the S3 extents are an unindexed blob. Catalog PITR + tested restore is a C5 must, validated against the engine's GC reconciliation procedure.

---

## Appendix — Sub-spec stubs

Each stub seeds a per-spec brainstorming/writing-plans cycle. Not implementation plans.

### C0 — Foundations
- Stand up bootstrap AWS account, TF remote state, GitHub OIDC.
- Author `infra/modules` skeletons + `envs/dev`.
- Confirm + pin Railway, Supabase, Stripe TF providers; document fallbacks.
- Resolve every §7 decision; record region + naming conventions.
- CI pipeline: plan on PR, apply on merge to env branches.

### C1 — Data plane on managed infra
- Containerize `kyma-bin` as strict 12-factor (config/env only, no Railway lock-in).
- Deploy engine + Supabase-B catalog + S3 bucket via Terraform to `dev`.
- Run the existing `scripts/e2e-test.sh` / `test-kql.sh` / `test-flight.sh` against managed infra.
- **Measure and record** hot-path latency (engine↔catalog↔S3) and cost-per-GB-ingested / cost-per-query. Gate downstream phases on acceptable numbers.

### C2 — Control plane + gateway
- Supabase Auth + control-plane schema (orgs, projects, api_keys, usage_events, subscriptions, audit_log) in project A.
- `kyma-cloud-api`: project CRUD, transactional pooled-tenant provisioning, key issuance (hash-at-rest).
- `kyma-gateway`: api-key authN, tenant resolution, scoped-STS minting, `X-Database` injection, quota/rate limit, usage capture.
- Isolation tests: tenant A key cannot read tenant B data (storage + catalog + query). Rebase on A4 status.

### C3 — Onboarding + dashboard + Claude Code (headline)
- Next.js dashboard + Supabase Auth signup/login.
- Project-create flow → surfaces OTLP/REST ingest endpoint+token, Flight endpoint, **MCP endpoint**.
- One-line `claude mcp add kyma --transport http …` snippet generator (per project, copy button).
- Minimal in-dashboard KQL/SQL query console.
- The 60-second "data in → answers in Claude Code" path, demoed end-to-end. Rebase on engine A2 (MCP) status.

### C4 — Metering + billing
- Per-tenant usage capture at gateway (ingest bytes/rows) + storage GB-month from S3 inventory + query bytes-scanned/Flight-time from engine metrics.
- Stripe products + metered prices per tier (Free/Pro/Enterprise); webhook → subscription state in DB A.
- Gateway enforces plan quotas + rate limits; billing portal + usage view in dashboard.
- Usage reconciliation job + discrepancy alerts.

### C5 — Hardening + scale
- Engine fleet autoscaling + noisy-neighbor controls; load test to target volume.
- DR: catalog PITR + tested restore; S3 versioning + cross-region replication.
- Observability: **dogfood — Kyma Cloud ingests its own telemetry into a Kyma instance**; ship Grafana dashboards + alerts.
- Security: secret rotation, encryption-at-rest audit, pen-test, SOC-2-ready control set, data-handling policy.
- Define + monitor SLOs (ingest availability, query p99, onboarding success rate); runbooks.
- Evaluate + (if triggered) execute data-plane move Railway → AWS next to S3.

### C6 — Silo + migration
- `tenant-silo` Terraform module (dedicated engine + bucket, optional region/account/Supabase).
- Control-plane-triggered silo provisioning via CI apply; route keys to dedicated stack.
- Pool→silo migration tooling (quiesce, snapshot catalog ns, copy S3 prefix, repoint, resume) + no-data-loss test.
- Data-residency tenants (region-pinned silo).
