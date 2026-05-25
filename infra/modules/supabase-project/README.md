# module: supabase-project

Provisions one Supabase project via the official `supabase/supabase` provider.
Kyma Cloud uses two instances of this module (decision #4): **A** for the
control-plane DB and **B** for the engine catalog — separate projects so a
catalog migration or restore can never touch billing/identity data.

Region defaults to `us-east-1` (Supabase is AWS-backed and supports it),
co-located with the S3 extent bucket and Railway compute to kill cross-cloud
hot-path latency (decision #7).

| Input | Sensitive | Purpose |
|---|---|---|
| `name` | no | project name |
| `org_id` | no | owning Supabase org |
| `region` | no | AWS-aligned region slug (default `us-east-1`) |
| `db_pass` | **yes** | DB password (from `TF_VAR_supabase_db_pass`/secret) |
| `instance_size` | no | compute size; `null` = default |

| Output | Sensitive | Purpose |
|---|---|---|
| `project_ref` | no | project ref for hostnames |
| `database_url` | **yes** | derived Postgres connection string |

The provider returns only the project `id` (ref); `database_url` is composed
from the ref + password. `database_password` drift is ignored (the API does not
read it back).
