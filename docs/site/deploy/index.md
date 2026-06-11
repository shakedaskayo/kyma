---
title: Production deployment
description: Deploy the kyma engine — pluggable compute (Fargate/EKS/Helm/local), database (Supabase/RDS/your own Postgres), and storage (S3/Supabase/any S3-compatible). One command via kyma deploy, or Terraform/Pulumi/Helm.
---

# Production deployment

The engine is backend-agnostic. `kyma deploy` picks one option on each of four
independent axes; everything else follows.

| Axis | Options |
| ---- | ------- |
| **Compute** | `fargate` (ECS, default) · `eks` (Terraform-provisioned cluster) · `helm` (your existing cluster) · `local` (docker) |
| **Database** (catalog Postgres) | `supabase` (provisioned) · `rds` (provisioned) · `external` (your `postgresql://` URL) |
| **Storage** (columnar extents) | `s3` (native AWS, keyless) · `supabase` (Supabase Storage) · `external` (MinIO / R2 / any S3-compatible) |
| **Auth** | `supabase` · `token` (minted admin token) · `oidc` |

The wizard asks for each axis, applies sane defaults, and rejects combinations
that can't work (e.g. native `s3` without an AWS compute target) with a message
telling you the nearest valid choice.

## One command

```sh
curl -fsSL https://raw.githubusercontent.com/shakedaskayo/kyma/main/install.sh | bash -s -- --prod-deploy
```

or, with the CLI installed:

```sh
kyma deploy init    # wizard: compute → database → storage → auth, + credentials
kyma deploy up      # provisions, prints your engine URL
kyma deploy status  # outputs + live /health probe
kyma deploy destroy
```

Preview any combination without touching a cloud: `kyma deploy init --print-only`.

## Common stacks

```sh
# Default: AWS Fargate + Supabase (catalog + auth + storage)
kyma deploy init

# Fully AWS-native: Fargate + RDS Postgres + native S3, token auth
kyma deploy init --compute fargate --database rds --storage s3 --auth token

# Kubernetes (EKS): cluster provisioned by Terraform, engine installed via Helm
kyma deploy init --compute eks --database rds --storage s3 --auth oidc \
  --oidc-issuer https://issuer.example.com --oidc-client-id kyma --ingress-host kyma.example.com

# Your own cluster + your own Postgres + your own object store
kyma deploy init --compute helm --database external --database-url "$DB_URL" \
  --storage external --storage-endpoint https://minio.example.com:9000 \
  --auth token --ingress-host kyma.example.com

# Local test drive (docker), bring-your-own Postgres + MinIO
kyma deploy init --compute local --database external --database-url "$DB_URL" \
  --storage external --storage-endpoint http://localhost:9000 --auth token
```

## Which compute should I pick?

| Compute | When | How traffic gets in |
| ------- | ---- | ------------------- |
| `fargate` | Default AWS path; lowest moving parts. | ALB (+ ACM/Route53 for TLS) |
| `eks` | You want Kubernetes and let kyma provision the cluster. | Kubernetes ingress |
| `helm` | You already run a cluster (EKS/GKE/AKS/anything). | Kubernetes ingress |
| `local` | Evaluate on your laptop with docker. | `localhost:8080` |

Native `s3` storage and provisioned `rds` require an AWS compute target
(`fargate`/`eks`) — they live in the stack's VPC and use the task/pod IAM role.
On `helm`/`local`, use `external` storage (endpoint + keys) and `external`/
`supabase` databases.

## Choose your path

| Path | When |
| ---- | ---- |
| [`kyma deploy` CLI](./cli) | Let the wizard drive everything. |
| [Terraform](./terraform) | Fargate/EKS — you manage IaC or customize the stack. |
| [Pulumi](./pulumi) | Same stack via the terraform-module bridge. |
| [Helm](./helm) | Install the engine chart on any Kubernetes cluster. |
| [Kubernetes / EKS](./kubernetes) | Provision an EKS cluster + install the chart. |

## Required secret: `KYMA_SECRET_KEY`

The server encrypts stored data source credentials (AES-256-GCM) with
`KYMA_SECRET_KEY` and refuses to start without it. Every deploy path sets it
for you: the Terraform stack generates one and injects it via SSM (Fargate) or
a Kubernetes Secret (EKS/Helm), `kyma deploy` writes it to the workspace
`local.env`, and `kyma service install` mints one at `~/.kyma/secret.key`.

Running the binary directly? Set it yourself:

```bash
export KYMA_SECRET_KEY="$(openssl rand -base64 32)"
```

Keep the value stable for the lifetime of a deployment — rotating it makes
credentials already encrypted at rest undecryptable (re-enter them after a
rotation).

## After deploying

1. Open the engine URL and sign in via your auth backend (Supabase / OIDC, or
   the API token you minted for `token` auth).
2. Mint an API token under **Settings → API tokens** and connect the CLI:
   `kyma connect <engine-url> --token <api-token>`.
3. Wire your coding agents: `kyma setup claude-code` (and friends) — see the
   [quickstart](/quickstart/).
