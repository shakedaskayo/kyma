output "engine_url" {
  value       = local.engine_url
  description = "Base URL of the engine (web UI + API). Empty for EKS until the ingress is up — the CLI resolves it via kubectl. Use with `pensieve connect`."
}

output "compute_backend" {
  value       = var.compute_backend
  description = "Compute backend in effect (fargate | eks)."
}

output "alb_dns_name" {
  value       = module.network.alb_dns_name
  description = "Raw ALB DNS name (Fargate; CNAME target when managing DNS manually)."
}

output "acm_validation_records" {
  value       = module.network.acm_validation_records
  description = "ACM DNS-validation records to create manually when `domain` is set without `route53_zone_id`."
}

output "admin_user" {
  value       = "admin"
  description = "Fallback admin username."
}

output "admin_password" {
  value       = random_password.admin_password.result
  description = "Fallback admin password (also stored in SSM on Fargate)."
  sensitive   = true
}

output "bucket_name" {
  value = (
    local.s3_native ? one(module.storage[*].bucket_name) :
    local.ext_storage ? "${var.storage_bucket} (external S3-compatible)" :
    "${var.storage_bucket} (Supabase Storage)"
  )
  description = "Bucket holding pensieve's columnar extents."
}

output "supabase_project_ref" {
  value       = one(module.supabase[*].project_ref)
  description = "Supabase project ref (null unless a Supabase backend is in use)."
}

output "supabase_url" {
  value       = one(module.supabase[*].project_url)
  description = "Supabase project URL (auth + APIs); null unless Supabase is in use."
}

output "database_endpoint" {
  value       = one(module.rds[*].endpoint)
  description = "RDS catalog hostname (null unless database_backend = rds)."
}

output "ecs_cluster_name" {
  value       = one(module.engine[*].cluster_name)
  description = "ECS cluster name (Fargate; null otherwise)."
}

output "ecs_service_name" {
  value       = one(module.engine[*].service_name)
  description = "ECS service name (Fargate; null otherwise)."
}

output "log_group" {
  value       = one(module.engine[*].log_group)
  description = "CloudWatch log group with engine logs (Fargate; null otherwise)."
}

output "eks_cluster_name" {
  value       = one(module.eks[*].cluster_name)
  description = "EKS cluster name (null unless compute_backend = eks). Use with `aws eks update-kubeconfig`."
}

output "eks_irsa_role_arn" {
  value       = one(module.eks[*].irsa_role_arn)
  description = "IAM role ARN to annotate the engine service account for keyless S3 on EKS (null otherwise)."
}

output "catalog_url" {
  value       = local.catalog_url
  description = "Resolved Postgres catalog URL — the CLI injects it into the Helm Secret on the EKS path."
  sensitive   = true
}

output "storage_bucket_name" {
  value       = local.s3_native ? one(module.storage[*].bucket_name) : var.storage_bucket
  description = "Raw extents bucket name (for the EKS Helm install)."
}
