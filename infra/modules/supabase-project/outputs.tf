output "project_ref" {
  value       = supabase_project.this.id
  description = "Supabase project ref (used in API/DB hostnames)."
}

# The provider exposes only the ref; the connection string is derived from it.
output "database_url" {
  value       = "postgresql://postgres:${var.db_pass}@db.${supabase_project.this.id}.supabase.co:5432/postgres"
  description = "Postgres connection string for this project."
  sensitive   = true
}
