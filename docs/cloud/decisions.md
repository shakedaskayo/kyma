# Kyma Cloud — Decisions Record

Locks §7 of `2026-05-25-kyma-cloud-platform-design.md`. One row per decision;
change requires a new dated entry, not an edit.

## 2026-05-25 — initial locks

| # | Decision | Choice | Rationale |
|---|---|---|---|
| 1 | IaC tool | Terraform 1.9+ / OpenTofu 1.7+ | Broadest provider coverage; mature OIDC + remote state |
| 2 | Tenancy at launch | Pooled, with silo path | Density now, isolation later, cheap migration |
| 3 | Pooled isolation | Scoped STS session creds + `X-Database` + gateway quotas | Strongest practical isolation without per-tenant processes |
| 4 | Supabase split | Two projects: A control-plane, B catalog | Independent blast radius / backups / roles |
| 5 | Control-plane lang | TypeScript (Node) | Native to Supabase/Next.js/Stripe; CRUD not hot-path |
| 6 | Gateway lang | Rust, reuse `kyma-server` crates | Hot path; keep OSS engine unmodified |
| 7 | Region | `us-east-1` for AWS + Supabase + Railway | Kill cross-cloud hot-path latency |
| 8 | Provisioning split | Pooled = app logic; silo + envs = Terraform | Never terraform-apply per signup |
| 9 | Billing meters | Ingest bytes/rows · storage GB-mo · query bytes-scanned/Flight-time | Maps to real S3 + compute cost; engine already emits these |
| 10 | Secrets | GH OIDC→AWS; tokens in Secrets Manager/Doppler; none in state | Supply-chain hygiene |

## Open items to confirm during C0 (close before C0 done)
- [x] Railway Terraform provider maturity (Task 4) — provider `terraform-community-providers/railway` is pre-1.0 (0.6.2); chose the `null_resource` + Railway CLI fallback for now (revisit at C1). See `infra/modules/_providers.md`.
- [x] Supabase region parity with `us-east-1` — confirmed; Supabase is AWS-backed and offers `us-east-1` (East US, N. Virginia), co-locating with the S3 bucket + Railway (decision #7).
- [x] Secrets manager choice — **AWS Secrets Manager**. Platform is already AWS-heavy (S3/STS/IAM/OIDC); native IAM integration + rotation, no extra vendor. CI provider tokens stay in GH Actions secrets; runtime app secrets live in Secrets Manager. Doppler rejected only to avoid a second vendor.
