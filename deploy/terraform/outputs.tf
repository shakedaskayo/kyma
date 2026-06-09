output "engine_url" {
  value       = module.kyma.engine_url
  description = "Base URL of the deployed engine (web UI + API). Use with `kyma connect`."
}

output "compute_backend" {
  value       = module.kyma.compute_backend
  description = "Compute backend in effect (fargate | eks)."
}

output "alb_dns_name" {
  value       = module.kyma.alb_dns_name
  description = "Raw ALB DNS name (CNAME target when managing DNS manually)."
}

output "acm_validation_records" {
  value       = module.kyma.acm_validation_records
  description = "ACM DNS-validation records to create manually when `domain` is set without `route53_zone_id`."
}

output "admin_user" {
  value       = module.kyma.admin_user
  description = "Fallback admin username (primary sign-in is Supabase Auth)."
}

output "admin_password" {
  value       = module.kyma.admin_password
  description = "Fallback admin password (also stored in SSM)."
  sensitive   = true
}

output "bucket_name" {
  value       = module.kyma.bucket_name
  description = "S3 bucket holding kyma's columnar extents."
}

output "supabase_project_ref" {
  value       = module.kyma.supabase_project_ref
  description = "Supabase project ref (dashboard: https://supabase.com/dashboard/project/<ref>)."
}

output "supabase_url" {
  value       = module.kyma.supabase_url
  description = "Supabase project URL (auth + APIs)."
}

output "ecs_cluster_name" {
  value       = module.kyma.ecs_cluster_name
  description = "ECS cluster name (for aws ecs CLI inspection)."
}

output "ecs_service_name" {
  value       = module.kyma.ecs_service_name
  description = "ECS service name."
}

output "log_group" {
  value       = module.kyma.log_group
  description = "CloudWatch log group with engine logs."
}

output "database_endpoint" {
  value       = module.kyma.database_endpoint
  description = "RDS catalog hostname (null unless database_backend = rds)."
}

output "eks_cluster_name" {
  value       = module.kyma.eks_cluster_name
  description = "EKS cluster name (null unless compute_backend = eks). Use with `aws eks update-kubeconfig`."
}

output "eks_irsa_role_arn" {
  value       = module.kyma.eks_irsa_role_arn
  description = "IAM role ARN to annotate the engine service account for keyless S3 on EKS (null otherwise)."
}

output "catalog_url" {
  value       = module.kyma.catalog_url
  description = "Resolved Postgres catalog URL — injected into the Helm Secret on the EKS path."
  sensitive   = true
}

output "storage_bucket_name" {
  value       = module.kyma.storage_bucket_name
  description = "Raw extents bucket name (for the EKS Helm install)."
}
