---
title: Deploy with Terraform
description: The pensieve self-host Terraform stack — pluggable compute/database/storage/auth, modules, variables, domain/TLS, state, costs.
---

# Deploy with Terraform

The stack at
[`deploy/terraform`](https://github.com/shakedaskayo/pensieve/tree/main/deploy/terraform)
is a thin root (provider config) instantiating the provider-free `stack/`
module, which Pulumi users [wrap directly](./pulumi). It serves the `fargate`
and `eks` compute targets.

```sh
cd deploy/terraform
cp terraform.tfvars.example terraform.tfvars   # then edit (pick your backends)
export TF_VAR_supabase_db_password="$(openssl rand -base64 24)"   # supabase db
export SUPABASE_ACCESS_TOKEN=sbp_…              # only when a supabase backend is used
terraform init
terraform apply
terraform output engine_url
```

## Backend selectors

| Variable | Options | Notes |
| -------- | ------- | ----- |
| `compute_backend` | `fargate` · `eks` | EKS provisions the cluster; install the engine chart afterward (`pensieve deploy up` automates it). |
| `database_backend` | `supabase` · `rds` · `external` | `external` reads `database_url` (sensitive). |
| `storage_backend` | `supabase` · `s3` · `external` | `s3` is keyless (task/pod role); `external` reads `storage_endpoint` + keys. |
| `auth_backend` | `supabase` · `token` · `oidc` | `token` reads `admin_token`; `oidc` reads `oidc_issuer`/`oidc_client_id`. |

Modules are gated by these selectors (`count`), so only the resources your
combination needs are created.

## What it creates

| Module | When | Resources |
| ------ | ---- | --------- |
| `network` | always | 2-AZ VPC (public + private subnets, **no NAT**), and — for `fargate` — an ALB + target group with `/health` checks + optional ACM/Route53. EKS sets `create_alb = false` (ingress instead). |
| `ecs-service` | `compute=fargate` | ECS cluster, ARM64 Fargate task + service, task role (scoped S3), execution role (SSM injection), CloudWatch logs. |
| `eks` | `compute=eks` | EKS cluster + managed node group + OIDC provider + an IRSA role for keyless S3. |
| `supabase` | `database` or `storage` = `supabase` | Supabase project (catalog Postgres + Auth) + anon key. |
| `rds` | `database=rds` | Private, encrypted Postgres in the VPC, backups on, reachable only inside the VPC. |
| `storage` | `storage=s3` | Versioned, encrypted, private S3 extent bucket with lifecycle expiry. |
| `secrets` | `compute=fargate` | SSM SecureString params: catalog URL, `PENSIEVE_SECRET_KEY`, auth/storage secrets. (EKS/Helm inject secrets via a Kubernetes Secret instead.) |

## Key variables

| Variable | Notes |
| -------- | ----- |
| `supabase_org_id`, `supabase_db_password` | Required **only** when a Supabase backend is used (conditionally validated). Pass the password via `TF_VAR_…`. |
| `database_url` | Required for `database_backend = external`. Sensitive — `TF_VAR_database_url`. |
| `rds_instance_class` | RDS sizing (default `db.t4g.micro`). |
| `storage_endpoint`, `storage_bucket`, `storage_region`, `storage_access_key`, `storage_secret`, `storage_path_style` | External S3-compatible store. |
| `admin_token` | Required for `auth_backend = token` (`PENSIEVE_AUTH_TOKENS=<token>:admin`). Sensitive. |
| `oidc_issuer`, `oidc_client_id` | For `auth_backend = oidc`. |
| `admin_emails` | Sign-ins with these emails get the pensieve admin role (supabase/oidc). |
| `allowed_email_domains` | **Set it** for open Supabase signup — otherwise anyone who can register gets read access. |
| `oauth_providers` | Login-page buttons (e.g. `["google","github"]`); enable the same in Supabase. |
| `domain`, `route53_zone_id` | Custom domain + automated TLS (fargate). Without a zone id, `apply` prints the ACM validation CNAME and waits. |
| `eks_kubernetes_version`, `eks_instance_types`, `eks_desired_size` | EKS sizing. |
| `image_tag` | Pin a release (`vX.Y.Z`). |
| `task_cpu`, `task_memory`, `desired_count` | Fargate sizing. Keep `desired_count = 1` — the engine is single-writer per catalog. |

## Validate every combination

`terraform validate` checks the configuration for all backend values at once:

```sh
terraform -chdir=deploy/terraform/stack init -backend=false
terraform -chdir=deploy/terraform/stack validate
terraform fmt -check -recursive deploy/terraform
```

## State

Local state by default. For teams, uncomment the S3 backend block in
`backend.tf`, create the bucket + lock table once, and
`terraform init -migrate-state`.

## Costs

Fargate: ALB ~$16/mo + one 0.5 vCPU/1 GB ARM64 task ~$15/mo + storage +
DB tier. No NAT gateway. EKS adds the control-plane (~$73/mo) + node group —
pick it when you want Kubernetes, not for the cost floor.

## Limitations

- gRPC (Arrow Flight) and OTLP-gRPC are disabled (HTTP-only ALB/ingress);
  OTLP/HTTP and REST ingest work. An NLB variant is future work.
- The engine self-migrates the catalog on first connect; no bootstrap job.
- EKS provisions the cluster only — the engine is installed via the
  [Helm chart](./helm) (the CLI does this in `pensieve deploy up`).
