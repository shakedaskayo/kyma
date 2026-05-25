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
