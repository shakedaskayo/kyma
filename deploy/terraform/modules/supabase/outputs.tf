output "project_ref" {
  value       = supabase_project.this.id
  description = "Supabase project ref (used in API/DB hostnames)."
}

output "project_url" {
  value       = "https://${supabase_project.this.id}.supabase.co"
  description = "Project base URL — auth (JWKS) + REST live under it."
}

# The provider exposes only the ref; the connection string is derived.
output "database_url" {
  value       = "postgresql://postgres:${var.db_pass}@db.${supabase_project.this.id}.supabase.co:5432/postgres"
  description = "Postgres connection string (kyma catalog)."
  sensitive   = true
}

output "anon_key" {
  value       = data.supabase_apikeys.this.anon_key
  description = "Publishable anon key for the login page (public by design)."
  sensitive   = true
}

output "service_role_key" {
  value       = data.supabase_apikeys.this.service_role_key
  description = "Service-role key (admin). Not wired into the engine; surfaced for operators."
  sensitive   = true
}
