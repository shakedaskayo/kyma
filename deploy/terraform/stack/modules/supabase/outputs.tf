output "project_ref" {
  value       = supabase_project.this.id
  description = "Supabase project ref (used in API/DB hostnames)."
}

output "project_url" {
  value       = "https://${supabase_project.this.id}.supabase.co"
  description = "Project base URL — auth (JWKS) + REST live under it."
}

# Session-mode pooler URL (IPv4-friendly); falls back to the direct host
# if the pooler data source returns nothing.
output "database_url" {
  value       = local.database_url
  description = "Postgres connection string (pensieve catalog)."
  sensitive   = true
}

output "storage_s3_endpoint" {
  value       = "https://${supabase_project.this.id}.storage.supabase.co/storage/v1/s3"
  description = "Supabase Storage S3-protocol endpoint (extents store)."
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
