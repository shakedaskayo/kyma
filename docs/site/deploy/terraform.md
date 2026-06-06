---
title: Deploy with Terraform
description: The kyma self-host Terraform stack — modules, variables, domain/TLS, sign-in policy, state, costs.
---

# Deploy with Terraform

The stack lives at
[`deploy/terraform`](https://github.com/shakedaskayo/kyma/tree/main/deploy/terraform)
— a thin root (provider config) instantiating the provider-free
`stack/` module, which Pulumi users [wrap directly](./pulumi).

```sh
export SUPABASE_ACCESS_TOKEN=sbp_…   # https://supabase.com/dashboard/account/tokens
cd deploy/terraform
cp terraform.tfvars.example terraform.tfvars   # then edit
export TF_VAR_supabase_db_password="$(openssl rand -base64 24)"
terraform init
terraform apply
terraform output engine_url
```

## What it creates

| Module | Resources |
| ------ | --------- |
| `network` | 2-AZ VPC (public subnets only — **no NAT gateway**), ALB, target group with `/health` checks, optional ACM cert + Route53 records. |
| `ecs-service` | ECS cluster, ARM64 Fargate task + service, task role (scoped S3), execution role (SSM secret injection), CloudWatch logs (30d). |
| `storage` | Versioned, encrypted, private S3 extent bucket with lifecycle expiry of noncurrent versions. |
| `supabase` | Supabase project (catalog Postgres + Auth) + anon key wired into the engine env. |
| `secrets` | SSM SecureString parameters: catalog URL, `KYMA_SECRET_KEY`, fallback admin credentials. |

The engine starts with `KYMA_AUTH_BACKEND=supabase` and **no S3 keys** —
credentials come from the Fargate task role.

## Key variables

| Variable | Notes |
| -------- | ----- |
| `supabase_org_id` | Required. Your org id from the Supabase dashboard. |
| `supabase_db_password` | Required, sensitive — pass via `TF_VAR_…`. |
| `admin_emails` | Sign-ins with these emails get the kyma admin role. |
| `allowed_email_domains` | **Set it.** Without it, anyone who can register in your Supabase project gets read access. |
| `oauth_providers` | Login-page buttons (e.g. `["google","github"]`) — enable the same providers in Supabase → Authentication. |
| `domain`, `route53_zone_id` | Custom domain + automated TLS. Without a zone id, `apply` prints the ACM validation CNAME and waits for you to create it. Without a domain, the stack is plain HTTP on the ALB DNS name — demo-grade. |
| `image_tag` | Pin a release (`vX.Y.Z`). `kyma deploy` injects the tag matching the CLI. |
| `task_cpu`, `task_memory`, `desired_count` | Sizing. Keep `desired_count = 1` — the engine is single-writer per catalog. |

## State

Local state by default (simplest for one operator). For teams, uncomment
the S3 backend block in `backend.tf`, create the bucket + DynamoDB lock
table once, and `terraform init -migrate-state`.

## Costs

ALB ~$16/mo + one 0.5 vCPU / 1 GB ARM64 Fargate task ~$15/mo + S3 storage +
Supabase tier. The public-subnet design avoids the ~$32/mo NAT gateway;
the task gets a public IP for egress (ingress is locked to the ALB security
group).

## Limitations

- gRPC (Arrow Flight) and OTLP-gRPC are disabled (`KYMA_GRPC_ADDR=off`,
  `KYMA_OTLP_ADDR=off`) — the ALB is HTTP-only. OTLP/HTTP and REST ingest
  work through it. An NLB variant is future work.
- The engine self-migrates the catalog on first connect; no bootstrap job.
