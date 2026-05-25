# module: s3-extent-bucket

The S3 bucket that holds Kyma extents (the source of truth). Versioned,
KMS-encrypted at rest, public access fully blocked, with a lifecycle rule that
expires noncurrent object versions after `noncurrent_expiration_days`.

**Pooled tenancy:** there is **one** extent bucket per environment. Pooled
tenants are *prefixes* inside it (`s3://<bucket>/<tenant_id>/`), never separate
buckets. The `iam-scoped-role` module + a per-request STS session policy confine
each tenant's credentials to its own prefix.

| Input | Type | Default | Purpose |
|---|---|---|---|
| `bucket_name` | string | — | Globally-unique name, e.g. `kyma-dev-extents` |
| `env` | string | — | Tag value (`dev`/`staging`/`prod`) |
| `noncurrent_expiration_days` | number | `30` | Expiry for noncurrent versions |

| Output | Purpose |
|---|---|
| `bucket_name` | Bucket id |
| `bucket_arn` | Bucket ARN (feeds `iam-scoped-role`) |
