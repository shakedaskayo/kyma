# Deploying pensieve to production

Self-host the pensieve engine with pluggable backends. Pick one option on each axis
— the wizard wires the rest:

| Axis | Options |
| ---- | ------- |
| **Compute** | `fargate` (ECS, default) · `eks` (Terraform-provisioned) · `helm` (your cluster) · `local` (docker) |
| **Database** | `supabase` · `rds` · `external` (your `postgresql://` URL) |
| **Storage** | `s3` (native, keyless) · `supabase` · `external` (MinIO/R2/any S3-compatible) |
| **Auth** | `supabase` · `token` · `oidc` |

The engine is backend-agnostic; this directory holds the IaC + Helm chart that
expose those choices. Native `s3`/`rds` need an AWS compute target (`fargate`/`eks`).

## The easy way: `pensieve deploy`

```sh
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/pensieve/main/install.sh | bash -s -- --prod-deploy
# or, with the CLI installed:
pensieve deploy init     # wizard: compute → database → storage → auth + credentials
pensieve deploy up       # provision + connect
pensieve deploy status   # outputs + live /health probe
pensieve deploy destroy
```

Preview any combination: `pensieve deploy init --print-only`. Examples:

```sh
# AWS-native: Fargate + RDS + native S3 + token auth
pensieve deploy init --compute fargate --database rds --storage s3 --auth token

# Kubernetes: EKS cluster (Terraform) + engine via Helm, OIDC auth
pensieve deploy init --compute eks --database rds --storage s3 --auth oidc \
  --oidc-issuer https://issuer.example.com --ingress-host pensieve.example.com

# Your cluster + your Postgres + your object store
pensieve deploy init --compute helm --database external --database-url "$DB_URL" \
  --storage external --storage-endpoint https://minio:9000 --auth token \
  --ingress-host pensieve.example.com
```

## Layout

```
terraform/                 # thin root (provider config) → stack/ module (fargate/eks)
terraform/stack/modules/   # network, ecs-service, eks, supabase, rds, storage, secrets
helm/pensieve-engine/          # the engine Helm chart (helm target + EKS install)
pulumi/                    # consumes stack/ via the terraform-module bridge
```

## The manual ways

- **Terraform** (`fargate`/`eks`): `cd terraform && cp terraform.tfvars.example
  terraform.tfvars` (set your backends) → `terraform init && apply`.
- **Pulumi**: same `stack/` via the terraform-module bridge — see `pulumi/`.
- **Helm** (any cluster): `helm upgrade --install pensieve helm/pensieve-engine -n pensieve
  --create-namespace -f your-values.yaml`.

Full docs: `docs/site/deploy/` → Overview · CLI · Terraform · Pulumi · Helm ·
Kubernetes (EKS).

## Notes

- gRPC (Arrow Flight) + OTLP-gRPC are off in these stacks (HTTP-only
  ALB/ingress); OTLP/HTTP + REST ingest work. NLB variant is future work.
- The engine self-migrates the catalog on first connect — no bootstrap job.
- The Terraform `stack/` is provider-free (Pulumi-bridge requirement); EKS
  provisions the cluster only and the CLI installs the chart as a second step.
- Engine image: `ghcr.io/shakedaskayo/pensieve-engine`. Pin `image_tag` to a release.
