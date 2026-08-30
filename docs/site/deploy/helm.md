---
title: Deploy with Helm
description: Install the pensieve engine on any Kubernetes cluster with the pensieve-engine Helm chart — values reference, ingress/TLS, secrets, IRSA.
---

# Deploy with Helm

The `pensieve-engine` chart at
[`deploy/helm/pensieve-engine`](https://github.com/shakedaskayo/pensieve/tree/main/deploy/helm/pensieve-engine)
runs the engine on any Kubernetes cluster (EKS, GKE, AKS, k3s, …). It's the same
chart the [EKS path](./kubernetes) installs after provisioning a cluster.

By default the engine runs **all-in-one** — a single pod (`replicaCount: 1`,
`Recreate`) that serves queries, ingests, commits, and runs every background
job. It is single-writer per catalog; don't raise `replicaCount`. To scale past
one pod, use the [role split](#scaling-out-the-role-split) instead, which runs
stateless query/ingest pods behind the Service and a single committer.

## Via `pensieve deploy`

```sh
pensieve deploy init --compute helm \
  --database external --database-url "$DB_URL" \
  --storage external --storage-endpoint https://minio.example.com:9000 \
    --storage-bucket pensieve --storage-access-key "$AK" --storage-secret "$SK" \
  --auth token --ingress-host pensieve.example.com
pensieve deploy up        # helm upgrade --install pensieve … -n pensieve --create-namespace
```

`init` renders `~/.pensieve/deploy/<name>/helm-values.yaml` from your answers (and
provisions a Supabase project if `--database supabase`). `up` installs it into
your current kubectl context (override with `--kube-context`).

## Manual install

```sh
helm upgrade --install pensieve deploy/helm/pensieve-engine \
  -n pensieve --create-namespace \
  --set image.tag=v0.0.7 \
  --set ingress.enabled=true --set ingress.host=pensieve.example.com --set ingress.tls=true \
  --set env.PENSIEVE_AUTH_BACKEND=token \
  --set secretEnv.PENSIEVE_AUTH_TOKENS='mytoken:admin' \
  --set secretEnv.PENSIEVE_CATALOG_URL="$DB_URL" \
  --set env.PENSIEVE_S3_ENDPOINT=https://minio.example.com:9000 \
  --set env.PENSIEVE_S3_BUCKET=pensieve --set env.PENSIEVE_S3_REGION=us-east-1 \
  --set env.PENSIEVE_S3_PATH_STYLE=true \
  --set secretEnv.PENSIEVE_S3_ACCESS_KEY_ID="$AK" \
  --set secretEnv.PENSIEVE_S3_SECRET_ACCESS_KEY="$SK"
```

## Values

| Key | Default | Purpose |
| --- | ------- | ------- |
| `image.repository` / `image.tag` / `image.pullPolicy` | `ghcr.io/shakedaskayo/pensieve-engine` / `latest` / `IfNotPresent` | Engine image. |
| `replicaCount` | `1` | Keep it 1 (single-writer). |
| `service.type` / `service.port` | `ClusterIP` / `8080` | In-cluster service. |
| `ingress.enabled` / `.host` / `.tls` / `.className` / `.annotations` | `false` | Ingress; set `host` (+ `tls`, and your controller's `annotations`). |
| `serviceAccount.create` / `.name` / `.annotations` | `true` / `pensieve-engine` / `{}` | On EKS, set `annotations."eks.amazonaws.com/role-arn"` for keyless S3. |
| `env` | engine `PENSIEVE_*` defaults | Non-secret env (`PENSIEVE_AUTH_BACKEND`, `PENSIEVE_S3_*`, …) — container `env`. |
| `secretEnv` | `{}` | Secret env (`PENSIEVE_CATALOG_URL`, `PENSIEVE_AUTH_TOKENS`, S3 keys) — rendered into a `Secret` and injected via `envFrom`. |
| `resources` | `{}` | Pod resource requests/limits. |
| `roles.enabled` | `false` | Turn on the [role split](#scaling-out-the-role-split). When `true`, the single all-in-one Deployment is replaced by `edge` / `committer` / `worker` Deployments. |
| `roles.edge.replicas` | `2` | Stateless edge (query + ingest) pod count when autoscaling is off. |
| `roles.edge.autoscaling.enabled` / `.minReplicas` / `.maxReplicas` / `.targetCPUUtilizationPercentage` | `false` / `2` / `6` / `70` | HPA for the edge Deployment (replaces `replicas`). |
| `roles.committer.pdb` | `true` | Render a `maxUnavailable: 1` PodDisruptionBudget for the committer. |
| `roles.worker.replicas` | `1` | Background-job worker pod count. |

## Auth on Kubernetes

- **token** — `env.PENSIEVE_AUTH_BACKEND=token` + `secretEnv.PENSIEVE_AUTH_TOKENS=<token>:admin`.
- **oidc** — `env.PENSIEVE_AUTH_BACKEND=oidc` + `env.PENSIEVE_OIDC_ISSUER` + `env.PENSIEVE_OIDC_CLIENT_ID`.
- **supabase** — `env.PENSIEVE_AUTH_BACKEND=supabase` + `PENSIEVE_SUPABASE_URL`/`PENSIEVE_SUPABASE_ANON_KEY`
  (the wizard provisions these when you pick `--database supabase`).

## Scaling out: the role split

The all-in-one pod can't scale horizontally — only one pod may commit to a
catalog at a time. To run many pods, set `roles.enabled=true`. The chart then
drops the single Deployment and renders three roles, each setting the `PENSIEVE_ROLE`
env the engine reads to decide which components to run:

| Role | `PENSIEVE_ROLE` | Runs | Strategy / scaling |
| ---- | ----------- | ---- | ------------------ |
| **edge** | `edge` | HTTP only — queries + ingest staging. No committer, no background jobs. | `RollingUpdate`, behind the Service, optional HPA. Scale freely. |
| **committer** | `committer` | The commit loop only — drains staged extents into the catalog. | `replicas: 1`, `Recreate`. A Postgres advisory-lock lease elects the single active committer, so failover is fast; the optional PDB keeps a node drain from evicting more than one committer pod at a time. |
| **worker** | `worker` | Background jobs only — compaction, index/graph builds, retention, GC. Jobs are claimed with `SKIP LOCKED`. | `RollingUpdate`, scales horizontally. |

The Service selects only `edge` pods, so all client traffic lands on the
stateless tier. `PENSIEVE_ROLE` values map as: `edge`/`query`/`ingest` → stateless;
`committer` → commit only; `worker`/`compaction` → jobs only; anything else
(including unset) → all-in-one.

**The role split requires staged ingest.** Edge pods stage writes to object
storage and ack; the committer drains them. Set `env.PENSIEVE_INGEST_MODE=staged` —
without it edge pods have nothing to hand off and the committer sits idle.

```sh
helm upgrade --install pensieve deploy/helm/pensieve-engine \
  -n pensieve --create-namespace \
  --set roles.enabled=true \
  --set env.PENSIEVE_INGEST_MODE=staged \
  --set roles.edge.autoscaling.enabled=true \
  --set roles.edge.autoscaling.minReplicas=2 --set roles.edge.autoscaling.maxReplicas=8 \
  --set roles.worker.replicas=2 \
  # …plus the image / catalog / storage / auth values from "Manual install" above
```

All three roles share the same image, `env`, and `secretEnv`, so point them at
the same catalog and object store. Local mode (`pensieve serve`) is always
all-in-one and ignores `PENSIEVE_ROLE`.

## Reaching the engine

With an ingress host, the engine is at `https://<host>`. Without one:

```sh
kubectl -n pensieve port-forward svc/pensieve-pensieve-engine 8080:8080
```

then `pensieve connect http://localhost:8080 --token <api-token>`. Secrets live in
the rendered `helm-values.yaml` (0600) and the Kubernetes `Secret` — swap to
[external-secrets](https://external-secrets.io/) if your cluster requires it.
