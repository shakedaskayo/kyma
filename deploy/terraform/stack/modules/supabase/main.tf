# Supabase project: catalog Postgres + Auth (JWKS consumed by the engine).
# Adapted from infra/modules/supabase-project, plus the apikeys data source
# so the anon key can be wired into the engine env automatically.

resource "supabase_project" "this" {
  name              = var.name
  organization_id   = var.org_id
  region            = var.region
  database_password = var.db_pass
  instance_size     = var.instance_size

  lifecycle {
    # The provider cannot read the password back; ignore drift on it.
    ignore_changes = [database_password]
  }
}

data "supabase_apikeys" "this" {
  project_ref = supabase_project.this.id
}

# Pooler connection strings (mode → URL). The direct db.<ref> host is
# IPv6-only on current Supabase projects; engines on IPv4-only networks
# (Fargate public subnets, most laptops) must connect via the pooler.
data "supabase_pooler" "this" {
  project_ref = supabase_project.this.id
}

locals {
  pooler_session = try(data.supabase_pooler.this.url["session"], null)
  database_url = local.pooler_session != null ? replace(
    local.pooler_session, "[YOUR-PASSWORD]", var.db_pass
    ) : (
    "postgresql://postgres:${var.db_pass}@db.${supabase_project.this.id}.supabase.co:5432/postgres"
  )
}
