output "engine_url" {
  value       = local.engine_url
  description = "Base URL of the deployed engine (web UI + API). Use with `kyma connect`."
}

output "alb_dns_name" {
  value       = module.network.alb_dns_name
  description = "Raw ALB DNS name (CNAME target when managing DNS manually)."
}

output "acm_validation_records" {
  value       = module.network.acm_validation_records
  description = "ACM DNS-validation records to create manually when `domain` is set without `route53_zone_id`."
}

output "admin_user" {
  value       = "admin"
  description = "Fallback admin username (primary sign-in is Supabase Auth)."
}

output "admin_password" {
  value       = random_password.admin_password.result
  description = "Fallback admin password (also stored in SSM)."
  sensitive   = true
}

output "bucket_name" {
  value       = module.storage.bucket_name
  description = "S3 bucket holding kyma's columnar extents."
}

output "supabase_project_ref" {
  value       = module.supabase.project_ref
  description = "Supabase project ref (dashboard: https://supabase.com/dashboard/project/<ref>)."
}

output "supabase_url" {
  value       = module.supabase.project_url
  description = "Supabase project URL (auth + APIs)."
}

output "ecs_cluster_name" {
  value       = module.engine.cluster_name
  description = "ECS cluster name (for aws ecs CLI inspection)."
}

output "ecs_service_name" {
  value       = module.engine.service_name
  description = "ECS service name."
}

output "log_group" {
  value       = module.engine.log_group
  description = "CloudWatch log group with engine logs."
}
