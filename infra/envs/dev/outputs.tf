output "extent_bucket" {
  value       = module.extents.bucket_name
  description = "Dev extent bucket name."
}

output "extent_bucket_arn" {
  value = module.extents.bucket_arn
}

output "control_plane_ref" {
  value       = module.control_plane_db.project_ref
  description = "Supabase A (control-plane) project ref."
}

output "catalog_ref" {
  value       = module.catalog.project_ref
  description = "Supabase B (engine catalog) project ref."
}

output "tenant_role_arn" {
  value       = module.tenant_role.role_arn
  description = "Role the gateway assumes (with a per-tenant session policy)."
}

output "billing_product_ids" {
  value = {
    free       = module.billing_free.product_id
    pro        = module.billing_pro.product_id
    enterprise = module.billing_enterprise.product_id
  }
}
