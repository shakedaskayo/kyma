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
