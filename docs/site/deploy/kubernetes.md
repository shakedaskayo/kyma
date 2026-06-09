---
title: Deploy on Kubernetes (EKS)
description: Provision an EKS cluster with Terraform and install the kyma engine via Helm, with keyless S3 through IRSA.
---

# Deploy on Kubernetes (EKS)

The `eks` compute target provisions an EKS cluster with Terraform, then installs
the [Helm chart](./helm) onto it. The Terraform stack stays provider-free (no
`kubernetes`/`helm` providers inside it), so the install is a second step the
CLI runs for you — which also keeps the [Pulumi bridge](./pulumi) working.

```sh
kyma deploy init --compute eks \
  --database rds --storage s3 \
  --auth oidc --oidc-issuer https://issuer.example.com --oidc-client-id kyma \
  --ingress-host kyma.example.com
kyma deploy up
```

`up` does:

1. `terraform apply` — VPC, EKS cluster + managed node group, OIDC provider, an
   **IRSA** role for keyless S3, and (here) RDS Postgres + the S3 extent bucket.
2. Reads the stack outputs (cluster name, IRSA role ARN, resolved catalog URL,
   bucket name).
3. `aws eks update-kubeconfig --name <cluster> --region <region>`.
4. Renders Helm values from those outputs — the IRSA role goes on the engine
   service account (`eks.amazonaws.com/role-arn`), the catalog URL + bucket into
   the engine env — and runs `helm upgrade --install kyma … -n kyma`.

## Keyless S3 (IRSA)

With `--storage s3`, the engine pod reaches S3 through its service account's
IAM role — no static keys. The EKS module creates the role, scopes it to the
extent bucket, and trusts `system:serviceaccount:kyma:kyma-engine`; the chart
annotates that service account. This mirrors the Fargate task-role model.

## Ingress + TLS

Set `--ingress-host`. The chart emits an `Ingress`; provide your controller via
Helm values (e.g. the AWS Load Balancer Controller, or cert-manager for TLS):

```sh
helm upgrade kyma ~/.kyma/deploy/prod/helm/kyma-engine -f ~/.kyma/deploy/prod/helm-values.yaml -n kyma \
  --set ingress.className=alb \
  --set ingress.annotations."alb\.ingress\.kubernetes\.io/scheme"=internet-facing
```

Without an ingress host, port-forward:
`kubectl -n kyma port-forward svc/kyma-kyma-engine 8080:8080`.

## Bring your own cluster

Already running Kubernetes? Skip EKS provisioning and use the
[`helm` target / chart](./helm) directly against your cluster.

## Teardown

`kyma deploy destroy` runs `terraform destroy`, which deletes the cluster (and
the Helm release with it). RDS and the bucket are destroyed too — back up first
if you need the data.
