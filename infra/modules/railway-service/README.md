# module: railway-service

Deploys one Railway service (engine / gateway / api / dashboard). Because the
only Railway Terraform provider is pre-1.0 (`terraform-community-providers/
railway` 0.6.2 — see `../_providers.md`), this module uses the **CLI fallback**:
a `null_resource` whose `triggers` map forces a redeploy when any input changes,
with the Railway CLI invoked from a `local-exec` provisioner.

`env_vars` is sensitive and hashed into `triggers` (not stored plaintext). It
carries the engine's 12-factor config (`KYMA_S3_BUCKET`, `KYMA_S3_REGION`,
`KYMA_CATALOG_URL`, `KYMA_S3_PATH_STYLE`, …) so the image stays portable
(Railway → AWS later is a config change, not a rewrite).

| Input | Sensitive | Purpose |
|---|---|---|
| `project_id` | no | Railway project |
| `name` | no | service name (`kyma-<env>-<role>`) |
| `image` | no | container image ref |
| `region` | no | logical region (co-located with us-east-1) |
| `replicas` | no | replica count |
| `env_vars` | **yes** | service config/secrets |

| Output | Purpose |
|---|---|
| `service_name` | stable handle |

**C1 finishes this:** real `railway up` / `railway variables --set` and
capturing `service_id` + `public_domain` from `railway status --json`. The
`local-exec` here is a stub that echoes the intended deploy.
