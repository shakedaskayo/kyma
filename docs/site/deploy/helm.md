---
title: Deploy with Helm
description: Install the kyma engine on any Kubernetes cluster with the kyma-engine Helm chart — values reference, ingress/TLS, secrets, IRSA.
---

# Deploy with Helm

The `kyma-engine` chart at
[`deploy/helm/kyma-engine`](https://github.com/shakedaskayo/kyma/tree/main/deploy/helm/kyma-engine)
runs the engine on any Kubernetes cluster (EKS, GKE, AKS, k3s, …). It's the same
chart the [EKS path](./kubernetes) installs after provisioning a cluster.

The engine is **single-writer per catalog** — the chart pins `replicaCount: 1`
with a `Recreate` strategy. Don't scale it up.

## Via `kyma deploy`

```sh
kyma deploy init --compute helm \
  --database external --database-url "$DB_URL" \
  --storage external --storage-endpoint https://minio.example.com:9000 \
    --storage-bucket kyma --storage-access-key "$AK" --storage-secret "$SK" \
  --auth token --ingress-host kyma.example.com
kyma deploy up        # helm upgrade --install kyma … -n kyma --create-namespace
```

`init` renders `~/.kyma/deploy/<name>/helm-values.yaml` from your answers (and
provisions a Supabase project if `--database supabase`). `up` installs it into
your current kubectl context (override with `--kube-context`).

## Manual install

```sh
helm upgrade --install kyma deploy/helm/kyma-engine \
  -n kyma --create-namespace \
  --set image.tag=v0.0.7 \
  --set ingress.enabled=true --set ingress.host=kyma.example.com --set ingress.tls=true \
  --set env.KYMA_AUTH_BACKEND=token \
  --set secretEnv.KYMA_AUTH_TOKENS='mytoken:admin' \
  --set secretEnv.KYMA_CATALOG_URL="$DB_URL" \
  --set env.KYMA_S3_ENDPOINT=https://minio.example.com:9000 \
  --set env.KYMA_S3_BUCKET=kyma --set env.KYMA_S3_REGION=us-east-1 \
  --set env.KYMA_S3_PATH_STYLE=true \
  --set secretEnv.KYMA_S3_ACCESS_KEY_ID="$AK" \
  --set secretEnv.KYMA_S3_SECRET_ACCESS_KEY="$SK"
```

## Values

| Key | Default | Purpose |
| --- | ------- | ------- |
| `image.repository` / `image.tag` / `image.pullPolicy` | `ghcr.io/shakedaskayo/kyma-engine` / `latest` / `IfNotPresent` | Engine image. |
| `replicaCount` | `1` | Keep it 1 (single-writer). |
| `service.type` / `service.port` | `ClusterIP` / `8080` | In-cluster service. |
| `ingress.enabled` / `.host` / `.tls` / `.className` / `.annotations` | `false` | Ingress; set `host` (+ `tls`, and your controller's `annotations`). |
| `serviceAccount.create` / `.name` / `.annotations` | `true` / `kyma-engine` / `{}` | On EKS, set `annotations."eks.amazonaws.com/role-arn"` for keyless S3. |
| `env` | engine `KYMA_*` defaults | Non-secret env (`KYMA_AUTH_BACKEND`, `KYMA_S3_*`, …) — container `env`. |
| `secretEnv` | `{}` | Secret env (`KYMA_CATALOG_URL`, `KYMA_AUTH_TOKENS`, S3 keys) — rendered into a `Secret` and injected via `envFrom`. |
| `resources` | `{}` | Pod resource requests/limits. |

## Auth on Kubernetes

- **token** — `env.KYMA_AUTH_BACKEND=token` + `secretEnv.KYMA_AUTH_TOKENS=<token>:admin`.
- **oidc** — `env.KYMA_AUTH_BACKEND=oidc` + `env.KYMA_OIDC_ISSUER` + `env.KYMA_OIDC_CLIENT_ID`.
- **supabase** — `env.KYMA_AUTH_BACKEND=supabase` + `KYMA_SUPABASE_URL`/`KYMA_SUPABASE_ANON_KEY`
  (the wizard provisions these when you pick `--database supabase`).

## Reaching the engine

With an ingress host, the engine is at `https://<host>`. Without one:

```sh
kubectl -n kyma port-forward svc/kyma-kyma-engine 8080:8080
```

then `kyma connect http://localhost:8080 --token <api-token>`. Secrets live in
the rendered `helm-values.yaml` (0600) and the Kubernetes `Secret` — swap to
[external-secrets](https://external-secrets.io/) if your cluster requires it.
